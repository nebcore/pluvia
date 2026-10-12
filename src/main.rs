// Sin esto, Windows abre una consola además del ícono de bandeja. Solo se aplica en
// release: en debug queda la consola para poder ver panics/errores mientras se prueba.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod popup;
mod state;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use audio::{Audio, LAYER_DEFAULT_VOLUME};
use state::{SavedLayer, SavedState};

slint::include_modules!();

const SUPPORTED_EXTENSIONS: [&str; 4] = ["mp3", "wav", "ogg", "flac"];
const FADE_TICK: Duration = Duration::from_millis(10);
// Cada cuánto se revisa que haya una fuente encolada detrás de la que suena. Con pistas
// de minutos alcanza de sobra, y despierta a la app una vez por segundo solo mientras
// suena (en pausa el timer se apaga).
const REFILL_INTERVAL: Duration = Duration::from_secs(1);
// Volumen con el que arranca la bienvenida del primer inicio: bajo a propósito, para que
// la primera vez nunca suene fuerte.
const WELCOME_VOLUME: f32 = 0.2;

// Carpeta base para assets/, icon.* y el archivo de estado. Si hay una carpeta
// assets/ junto al .exe, se usa esa (permite mover/copiar el .exe a cualquier lado
// y que siga encontrando sus archivos). Si no, y la carpeta actual tiene assets/, se
// usa la actual — es el caso de `cargo run`, donde el .exe vive en target/debug/ pero
// el proyecto (con su assets/) está en la raíz. Si ninguna tiene, se queda con la del
// .exe: al iniciar con Windows la carpeta actual suele ser System32, y ahí no hay que
// crear nada.
fn app_dir() -> PathBuf {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    match exe_dir {
        Some(dir) if dir.join("assets").is_dir() => dir,
        _ if Path::new("assets").is_dir() => PathBuf::from("."),
        Some(dir) => dir,
        None => PathBuf::from("."),
    }
}

fn list_audio_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files
}

// Autoinicio vía la clave Run del usuario actual — no requiere permisos de
// administrador (a diferencia de la clave equivalente en HKEY_LOCAL_MACHINE).
const AUTOSTART_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_NAME: &str = "Pluvia";
// Argumento con el que se registra el autoinicio: así la app sabe que la abrió Windows
// y arranca en pausa, solo con el ícono listo en la bandeja.
const AUTOSTART_ARG: &str = "--autostart";

fn is_autostart_enabled() -> bool {
    let Ok(key) = winreg::HKCU.open_subkey(AUTOSTART_KEY) else { return false };
    key.get_value::<String, _>(AUTOSTART_NAME).is_ok()
}

fn set_autostart(enabled: bool) {
    let Ok((key, _)) = winreg::HKCU.create_subkey(AUTOSTART_KEY) else { return };
    if enabled {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value(AUTOSTART_NAME, &format!("\"{}\" {AUTOSTART_ARG}", exe.display()));
        }
    } else {
        let _ = key.delete_value(AUTOSTART_NAME);
    }
}

fn icon_from_image(img: image::DynamicImage) -> Option<Icon> {
    let rgba = img.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Icon::from_rgba(rgba.into_raw(), width, height).ok()
}

// Tamaño de los íconos chicos de Windows (el de la bandeja) según la escala de pantalla:
// 16 px al 100 %, 24 px al 150 %, 32 px al 200 %... Se consulta después de crear el
// panel, cuando winit ya marcó la app como consciente del DPI.
fn small_icon_size() -> (u32, u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
    if w > 0 && h > 0 { (w as u32, h as u32) } else { (16, 16) }
}

// Ícono de la bandeja, siempre en el tamaño justo para que se vea nítido:
// 1. Uno propio junto al .exe, si existe (icon.ico o icon.png): opcional, para cambiarlo
//    sin recompilar. Va aparte de assets/, que es la carpeta de sonidos.
// 2. El icon.ico multitamaño que build.rs incrusta en el .exe (recurso 1): Windows elige
//    la entrada que corresponde a la escala de pantalla.
// 3. Último recurso: el icon.png del proyecto, incrustado al compilar.
fn tray_icon() -> Icon {
    let size = small_icon_size();
    let dir = app_dir();
    if let Ok(icon) = Icon::from_path(dir.join("icon.ico"), Some(size)) {
        return icon;
    }
    if let Some(icon) = image::open(dir.join("icon.png")).ok().and_then(icon_from_image) {
        return icon;
    }
    if let Ok(icon) = Icon::from_resource(1, Some(size)) {
        return icon;
    }
    image::load_from_memory(include_bytes!("../icon.png"))
        .ok()
        .and_then(icon_from_image)
        .expect("icon.png incrustado inválido")
}

// Ojo: es modal y deja correr el event loop (timers incluidos) mientras está abierto,
// así que nunca se llama con un RefCell de `Ctx` prestado.
fn show_message(title: &str, description: &str) {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .show();
}

// Manda un archivo a la Papelera de Windows (recuperable), en vez de borrarlo.
fn move_to_recycle_bin(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    // La API pide ruta absoluta y terminada en doble nulo.
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut from: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code == 0 && op.fAnyOperationsAborted == 0 {
        Ok(())
    } else {
        Err(format!("No se pudo mandar \"{}\" a la Papelera (código {code}).", absolute.display()))
    }
}

fn track_label(path: &Path) -> &str {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("(sin nombre)")
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().and_then(|n| n.to_str()).map(String::from)
}

// Crea, para cada salida de audio que se abre, lo que el hilo de audio llama si se
// desconecta el dispositivo: pasa el aviso al event loop a través del callback
// `output-lost` del panel. Cada salida recibe un número nuevo en `generation`, para
// ignorar avisos tardíos de una que ya se reemplazó o se cerró.
fn lost_notifier_factory(
    panel: slint::Weak<Panel>,
    generation: Rc<Cell<u32>>,
) -> impl Fn() -> audio::LostNotifier + 'static {
    move || {
        let current = generation.get().wrapping_add(1);
        generation.set(current);
        let panel = panel.clone();
        Box::new(move || {
            let _ = panel.upgrade_in_event_loop(move |p| p.invoke_output_lost(current as i32));
        })
    }
}

// Todo el estado de la app, compartido por los callbacks del panel. Cada parte va en su
// propio RefCell y los préstamos duran una sola instrucción, para que un diálogo modal
// (que deja correr timers) nunca encuentre algo ya prestado.
struct Ctx {
    panel: slint::Weak<Panel>,
    audio: RefCell<Audio>,
    // Lo que muestra el panel de cada capa de `audio`, en el mismo orden.
    layers_model: Rc<VecModel<LayerItem>>,
    saved: RefCell<SavedState>,
    assets_dir: PathBuf,
    // Dispositivos que muestra el panel, en el mismo orden (en el panel el índice 0 es el
    // predeterminado, así que el dispositivo `i` del panel es `device_ids[i - 1]`).
    device_ids: RefCell<Vec<String>>,
    default_device_id: RefCell<Option<String>>,
    // Número de la última salida de audio abierta (ver `lost_notifier_factory`).
    output_generation: Rc<Cell<u32>>,
    fade_timer: Timer,
    refill_timer: Timer,
    // Momento en que se ocultó el panel: si el clic en el ícono llega justo después,
    // es el mismo clic que lo cerró al quitarle el foco, y no hay que reabrirlo.
    last_hidden: Cell<Instant>,
    // Borde inferior del panel cuando se abre arriba del ícono (ver `popup`).
    bottom_anchor: Cell<Option<i32>>,
}

impl Ctx {
    fn panel(&self) -> Panel {
        self.panel.unwrap()
    }

    // Refleja el estado del audio en el panel y prende/apaga los timers según haga
    // falta: el de fade corre solo durante un fade y el de reencolado solo mientras
    // suena. En pausa la app no se despierta para nada.
    fn sync(self: &Rc<Self>) {
        let (playing, fading, audible) = {
            let audio = self.audio.borrow();
            (audio.is_playing(), audio.is_fading(), audio.is_audible())
        };
        self.panel().set_playing(playing);

        if fading && !self.fade_timer.running() {
            let ctx = Rc::downgrade(self);
            self.fade_timer.start(TimerMode::Repeated, FADE_TICK, move || {
                let Some(ctx) = ctx.upgrade() else { return };
                let still_fading = ctx.audio.borrow_mut().tick();
                if !still_fading {
                    ctx.fade_timer.stop();
                    ctx.sync();
                }
            });
        }

        if !audible {
            self.refill_timer.stop();
        } else if !self.refill_timer.running() {
            let ctx = Rc::downgrade(self);
            self.refill_timer.start(TimerMode::Repeated, REFILL_INTERVAL, move || {
                let Some(ctx) = ctx.upgrade() else { return };
                let result = ctx.audio.borrow_mut().refill();
                if let Err(e) = result {
                    ctx.sync();
                    show_message("Pluvia", &e);
                }
            });
        }
    }

    fn after_audio(self: &Rc<Self>, result: Result<(), String>) {
        self.sync();
        if let Err(e) = result {
            show_message("Pluvia", &e);
        }
    }

    // Vuelca el estado de las capas al panel: la lista de sonidos, el texto de arriba y
    // cuántos hay prendidos (para el aviso de rendimiento). No se llama mientras se
    // arrastra el slider de una capa, para no rearmar la lista bajo el mouse.
    fn update_layers_model(&self) {
        let audio = self.audio.borrow();
        let items: Vec<LayerItem> = audio
            .layers()
            .iter()
            .map(|l| LayerItem { name: track_label(l.path()).into(), on: l.is_on(), volume: l.volume() })
            .collect();
        let active: Vec<&str> = audio.layers().iter().filter(|l| l.is_on()).map(|l| track_label(l.path())).collect();
        let mix_label = match active.as_slice() {
            [] => String::new(),
            [only] => only.to_string(),
            many => format!("{} sonidos", many.len()),
        };
        self.layers_model.set_vec(items);
        let panel = self.panel();
        panel.set_mix_label(mix_label.into());
        panel.set_active_count(active.len() as i32);
    }

    fn toggle_layer(self: &Rc<Self>, index: usize) {
        let result = self.audio.borrow_mut().toggle_layer(index);
        self.update_layers_model();
        self.after_audio(result);
    }

    // Vuelve a leer los dispositivos (solo al abrir el panel, no en segundo plano) y, si
    // está sonando en otro lugar que el que corresponde, mueve la salida: p. ej. si se
    // reconectó el elegido o si cambió el predeterminado de Windows. Si el elegido está
    // desconectado suena el predeterminado, pero la elección se conserva por si vuelve.
    // Con la salida cerrada (en pausa) no hace falta: se elige al dar play.
    fn refresh_devices(self: &Rc<Self>) {
        let list = audio::output_devices();
        let selected = self.saved.borrow().device.clone();
        let selected_index =
            selected.and_then(|id| list.devices.iter().position(|(device_id, _)| *device_id == id));
        let default_id = list.default.as_ref().map(|(id, _)| id.clone());
        let wanted = selected_index.map(|i| list.devices[i].0.clone()).or(default_id.clone());

        let names: Vec<SharedString> = list.devices.iter().map(|(_, name)| name.as_str().into()).collect();
        let panel = self.panel();
        panel.set_default_device_name(list.default.map(|(_, name)| name).unwrap_or_default().into());
        panel.set_devices(ModelRc::new(VecModel::from(names)));
        panel.set_current_device(selected_index.map_or(0, |i| i as i32 + 1));
        panel.set_devices_open(false);
        *self.device_ids.borrow_mut() = list.devices.into_iter().map(|(id, _)| id).collect();
        *self.default_device_id.borrow_mut() = default_id;

        let misplaced = {
            let audio = self.audio.borrow();
            audio.is_open() && wanted.is_some() && wanted.as_deref() != audio.device_id()
        };
        if misplaced {
            let result = self.audio.borrow_mut().reopen();
            self.after_audio(result);
        }
    }

    fn persist(&self) {
        let mut saved = self.saved.borrow_mut();
        let audio = self.audio.borrow();
        // Mientras la bienvenida está abierta el volumen todavía no está elegido.
        if !self.panel().get_welcome() {
            saved.volume = Some(audio.volume());
        }
        saved.layers = audio
            .layers()
            .iter()
            .filter_map(|l| Some(SavedLayer { file: file_name(l.path())?, volume: l.volume(), on: l.is_on() }))
            .collect();
        saved.save();
    }

    fn show_panel(self: &Rc<Self>, icon: Option<tray_icon::Rect>) {
        self.refresh_devices();
        self.update_layers_model();
        let panel = self.panel();
        // Un menú contextual que quedó abierto no sobrevive a cerrar el panel.
        panel.set_menu_layer(-1);
        match icon {
            Some(rect) => popup::show_near_icon(&panel, rect, &self.bottom_anchor),
            None => {
                let _ = panel.show();
                panel.window().with_winit_window(popup::style_native_window);
            }
        }
    }

    fn hide_panel(&self) {
        let panel = self.panel();
        // La bienvenida solo se cierra con "Listo".
        if panel.get_welcome() || !panel.window().is_visible() {
            return;
        }
        self.last_hidden.set(Instant::now());
        let _ = panel.hide();
        // Se guarda al cerrar el panel y no en cada movimiento del slider, para no
        // escribir el archivo decenas de veces por segundo.
        self.persist();
    }
}

fn main() {
    if let Err(e) = run() {
        show_message("Pluvia", &e);
    }
}

fn run() -> Result<(), String> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_skip_taskbar(true).with_resizable(false)
        })
        .select()
        .map_err(|e| e.to_string())?;

    let assets_dir = app_dir().join("assets");
    let _ = std::fs::create_dir_all(&assets_dir);
    let files = list_audio_files(&assets_dir);
    let saved = SavedState::load();
    let first_run = saved.volume.is_none();
    let volume = saved.volume.unwrap_or(WELCOME_VOLUME);

    let panel = Panel::new().map_err(|e| e.to_string())?;
    let output_generation = Rc::new(Cell::new(0));
    let mut audio = Audio::new(
        saved.device.clone(),
        volume,
        lost_notifier_factory(panel.as_weak(), output_generation.clone()),
    );
    // Cada archivo de assets/ es una capa, con el volumen y estado que tenía guardados.
    // Sin nada guardado (primer inicio) se prende el primero a volumen completo, así el
    // volumen de la bienvenida es el que realmente se escucha.
    for (i, path) in files.into_iter().enumerate() {
        let saved_layer = file_name(&path).and_then(|name| saved.layer(&name));
        let (layer_volume, on) = match saved_layer {
            Some(layer) => (layer.volume, layer.on),
            None if saved.layers.is_empty() && i == 0 => (1.0, true),
            None => (LAYER_DEFAULT_VOLUME, false),
        };
        audio.add_layer(path, layer_volume, on);
    }
    let layers_model = Rc::new(VecModel::default());
    panel.set_layers(ModelRc::from(layers_model.clone()));

    panel.set_version(env!("CARGO_PKG_VERSION").into());
    panel.set_volume(volume);
    panel.set_welcome(first_run);
    let autostart = is_autostart_enabled();
    // Reescribe la entrada con la ruta actual del .exe, por si se movió de carpeta
    // desde que se activó el autoinicio.
    if autostart {
        set_autostart(true);
    }
    panel.set_autostart(autostart);

    let ctx = Rc::new(Ctx {
        panel: panel.as_weak(),
        audio: RefCell::new(audio),
        layers_model,
        saved: RefCell::new(saved),
        assets_dir,
        device_ids: RefCell::new(Vec::new()),
        default_device_id: RefCell::new(None),
        output_generation,
        fade_timer: Timer::default(),
        refill_timer: Timer::default(),
        last_hidden: Cell::new(Instant::now() - Duration::from_secs(10)),
        bottom_anchor: Cell::new(None),
    });

    // Abierta por el autoinicio de Windows: la mezcla queda lista pero en pausa, y el
    // panel no se abre solo (salvo la bienvenida, que hace falta para elegir volumen).
    let from_autostart = std::env::args().any(|a| a == AUTOSTART_ARG);
    ctx.update_layers_model();
    if !from_autostart {
        let result = ctx.audio.borrow_mut().play();
        ctx.after_audio(result);
    }

    let c = ctx.clone();
    panel.on_toggle_play(move || {
        if c.audio.borrow().active_count() == 0 {
            return;
        }
        let result = c.audio.borrow_mut().toggle();
        c.after_audio(result);
    });

    let c = ctx.clone();
    panel.on_volume_changed(move |v| c.audio.borrow_mut().set_volume(v));

    let c = ctx.clone();
    panel.on_toggle_layer(move |i| c.toggle_layer(i as usize));

    let c = ctx.clone();
    panel.on_layer_volume_changed(move |i, v| c.audio.borrow_mut().set_layer_volume(i as usize, v));

    let c = ctx.clone();
    panel.on_remove_layer(move |i| {
        let Some((path, still_open)) = c.audio.borrow_mut().remove_layer(i as usize) else { return };
        c.update_layers_model();
        c.sync();
        c.persist();
        // Si estaba sonando, su archivo sigue abierto mientras se apaga con fade: se manda
        // a la Papelera recién cuando termina (Windows no deja mover un archivo abierto).
        let recycle = move || {
            if let Err(e) = move_to_recycle_bin(&path) {
                show_message("Pluvia", &e);
            }
        };
        if still_open {
            Timer::single_shot(audio::FADE_DURATION + Duration::from_millis(300), recycle);
        } else {
            recycle();
        }
    });

    let c = ctx.clone();
    panel.on_select_device(move |i| {
        let id = usize::try_from(i - 1).ok().and_then(|i| c.device_ids.borrow().get(i).cloned());
        c.saved.borrow_mut().device = id.clone();
        let panel = c.panel();
        panel.set_current_device(i);
        panel.set_devices_open(false);
        let wanted = id.clone().or_else(|| c.default_device_id.borrow().clone());
        let move_now = wanted.as_deref() != c.audio.borrow().device_id();
        let result = c.audio.borrow_mut().set_device(id, move_now);
        c.after_audio(result);
    });

    let c = ctx.clone();
    panel.on_add_audio(move || {
        let Some(picked) = rfd::FileDialog::new()
            .add_filter("Audio", &SUPPORTED_EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        let Some(name) = picked.file_name() else {
            show_message("Pluvia", "No se pudo leer el nombre del archivo elegido.");
            return;
        };
        let dest = c.assets_dir.join(name);
        if dest.exists() {
            show_message(
                "Pluvia",
                "Ya hay un archivo con ese nombre en 'assets'. Renombrarlo y probar de nuevo.",
            );
            return;
        }
        if let Err(e) = std::fs::copy(&picked, &dest) {
            show_message("Pluvia", &format!("No se pudo copiar el archivo: {e}"));
            return;
        }
        // El sonido nuevo entra prendido y suena (sumándose a la mezcla, si ya sonaba algo).
        let index = c.audio.borrow_mut().add_layer(dest, LAYER_DEFAULT_VOLUME, false);
        c.toggle_layer(index);
    });

    let c = ctx.clone();
    panel.on_toggle_autostart(move || {
        let panel = c.panel();
        let enabled = !panel.get_autostart();
        set_autostart(enabled);
        panel.set_autostart(enabled);
    });

    let c = ctx.clone();
    panel.on_open_folder(move || {
        let _ = std::process::Command::new("explorer").arg(&c.assets_dir).spawn();
    });

    panel.on_about(|| {
        // La versión sale de Cargo.toml, que es la única fuente (ver CHANGELOG.md).
        let about = format!(
            "Pluvia v{}\nSonidos ambientales de fondo, empezando por la lluvia.\n\nInterfaz hecha con Slint (slint.dev).",
            env!("CARGO_PKG_VERSION")
        );
        show_message("Acerca de Pluvia", &about);
    });

    let c = ctx.clone();
    panel.on_quit(move || {
        c.persist();
        let _ = slint::quit_event_loop();
    });

    let c = ctx.clone();
    panel.on_dismiss(move || c.hide_panel());

    let c = ctx.clone();
    panel.on_welcome_done(move || {
        c.panel().set_welcome(false);
        c.persist();
    });

    let c = ctx.clone();
    panel.on_output_lost(move |generation| {
        if generation as u32 != c.output_generation.get() {
            return;
        }
        // Se reabre donde corresponda (el elegido o, si no está, el predeterminado). Si
        // no hay ningún dispositivo queda en pausa hasta el próximo play.
        let result = c.audio.borrow_mut().reopen();
        c.after_audio(result);
    });

    let c = ctx.clone();
    panel.on_tray_clicked(move |x, y, width, height| {
        if c.panel().window().is_visible() {
            c.hide_panel();
        } else if c.last_hidden.get().elapsed() > Duration::from_millis(300) {
            let rect = tray_icon::Rect {
                position: tray_icon::dpi::PhysicalPosition::new(x as f64, y as f64),
                size: tray_icon::dpi::PhysicalSize::new(width as u32, height as u32),
            };
            c.show_panel(Some(rect));
        }
    });

    let c = ctx.clone();
    panel.window().on_winit_window_event(move |_, event| {
        match event {
            winit::event::WindowEvent::Focused(false) => {
                // Se difiere para no ocultar la ventana mientras winit procesa su evento.
                let c = c.clone();
                Timer::single_shot(Duration::ZERO, move || c.hide_panel());
            }
            // Si el panel está arriba del ícono y crece (p. ej. al desplegar los
            // dispositivos o al pasar de la bienvenida al panel), crece hacia arriba
            // para no meterse sobre la barra de tareas.
            winit::event::WindowEvent::Resized(size) => {
                if let Some(bottom) = c.bottom_anchor.get() {
                    c.panel().window().with_winit_window(|w| {
                        if let Ok(pos) = w.outer_position() {
                            let y = bottom - size.height as i32;
                            if pos.y != y {
                                w.set_outer_position(winit::dpi::PhysicalPosition::new(pos.x, y));
                            }
                        }
                    });
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });

    let tray = Rc::new(
        TrayIconBuilder::new()
            .with_tooltip("Pluvia")
            .with_icon(tray_icon())
            .build()
            .map_err(|e| format!("No se pudo crear el ícono de la bandeja: {e}"))?,
    );

    // Los clics en el ícono llegan por evento (sin revisar cada tanto) y se pasan al
    // event loop: el izquierdo abre/cierra el panel (callback `tray-clicked`) y el
    // derecho hace play/pausa sin abrir nada.
    let weak = panel.as_weak();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, rect, .. } = event else {
            return;
        };
        let _ = match button {
            MouseButton::Left => {
                let (x, y) = (rect.position.x as i32, rect.position.y as i32);
                let (w, h) = (rect.size.width as i32, rect.size.height as i32);
                weak.upgrade_in_event_loop(move |p| p.invoke_tray_clicked(x, y, w, h))
            }
            MouseButton::Right => weak.upgrade_in_event_loop(|p| p.invoke_toggle_play()),
            MouseButton::Middle => Ok(()),
        };
    }));

    // Primer inicio (bienvenida) o sin audios: el panel se abre solo junto al ícono.
    // Se difiere un poco para que el ícono ya esté ubicado en la bandeja.
    // `--panel` (solo en debug) lo abre siempre: sirve para revisar el diseño sin tener
    // que clickear la bandeja.
    let force_panel = cfg!(debug_assertions) && std::env::args().any(|a| a == "--panel");
    if first_run || force_panel || (ctx.audio.borrow().layers().is_empty() && !from_autostart) {
        let c = ctx.clone();
        let tray = tray.clone();
        Timer::single_shot(Duration::from_millis(300), move || c.show_panel(tray.rect()));
    }

    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    drop(tray);
    Ok(())
}
