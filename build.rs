use std::path::Path;

fn main() {
    slint_build::compile("ui/panel.slint").expect("no se pudo compilar ui/panel.slint");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let ico_path = "icon.ico";
    let png_path = "icon.png";
    // slint-build declara sus propios archivos, y con eso cargo deja de volver a correr
    // este script ante otros cambios: sin estas líneas, cambiar el ícono no llegaba al .exe.
    println!("cargo:rerun-if-changed={ico_path}");
    println!("cargo:rerun-if-changed={png_path}");

    // El .exe necesita específicamente un .ico incrustado; si el usuario solo tiene
    // un icon.png (el que usa el ícono de la bandeja en tiempo de ejecución), lo
    // convertimos una vez a icon.ico para reutilizarlo acá también.
    if !Path::new(ico_path).exists() {
        let Ok(img) = image::open(png_path) else {
            return; // sin icon.ico ni icon.png: se compila con el ícono por defecto
        };
        if img.save(ico_path).is_err() {
            return;
        }
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path);
    if let Err(e) = res.compile() {
        eprintln!("cargo:warning=no se pudo incrustar el ícono en el .exe: {e}");
    }
}
