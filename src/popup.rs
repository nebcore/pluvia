// Ventana del panel: ubicarla junto al ícono de la bandeja y darle el estilo nativo de
// Windows 11.

use std::cell::Cell;

use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};

use crate::Panel;

// Esquinas redondeadas y borde sutil nativos de Windows 11 (en Windows 10 se ignora).
pub fn style_native_window(window: &winit::window::Window) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
    let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let corners = DWMWCP_ROUND;
    let border: u32 = 0x003a_3434; // COLORREF es 0x00BBGGRR
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as _, (&raw const corners).cast(), 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as _, (&raw const border).cast(), 4);
    }
}

// Abre el panel pegado al ícono: centrado sobre él, arriba si la barra de tareas está
// abajo (lo normal) o debajo si está arriba, sin salirse del monitor. Si queda arriba
// del ícono, guarda en `bottom_anchor` su borde inferior, para que al crecer (p. ej. al
// desplegar los dispositivos) crezca hacia arriba y no se meta sobre la barra de tareas.
pub fn show_near_icon(panel: &Panel, icon: tray_icon::Rect, bottom_anchor: &Cell<Option<i32>>) {
    // Se muestra primero fuera de la pantalla y recién después se ubica, para no ver un
    // parpadeo en la posición por defecto.
    panel.window().set_position(slint::PhysicalPosition::new(-10_000, -10_000));
    if panel.show().is_err() {
        return;
    }
    panel.window().with_winit_window(|window| {
        style_native_window(window);
        let size = window.outer_size();
        let (w, h) = (size.width as i32, size.height as i32);
        let icon_x = icon.position.x as i32;
        let icon_y = icon.position.y as i32;
        let (icon_w, icon_h) = (icon.size.width as i32, icon.size.height as i32);
        let gap = 8;

        let monitor = window
            .available_monitors()
            .find(|m| {
                let (p, s) = (m.position(), m.size());
                (p.x..p.x + s.width as i32).contains(&icon_x)
                    && (p.y..p.y + s.height as i32).contains(&icon_y)
            })
            .or_else(|| window.primary_monitor());
        let (mon_x, mon_y, mon_w, mon_h) = monitor
            .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.size().height as i32))
            .unwrap_or((0, 0, i32::MAX / 2, i32::MAX / 2));

        let x = (icon_x + icon_w / 2 - w / 2).clamp(mon_x + gap, mon_x + mon_w - w - gap);
        let y = if icon_y - mon_y > mon_h / 2 {
            bottom_anchor.set(Some(icon_y - gap));
            icon_y - h - gap
        } else {
            bottom_anchor.set(None);
            icon_y + icon_h + gap
        };
        window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        window.focus_window();
    });
}
