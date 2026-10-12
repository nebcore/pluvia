// Lo que se recuerda entre reinicios, en `.pluvia_state` junto al resto de los archivos
// de la app (ver `app_dir()`): una clave por línea (`clave=valor`). Se siguen leyendo
// los formatos viejos: 0.1–0.2 guardaban solo el nombre de la pista, sin clave, y 0.3
// guardaba `track=<archivo>` (una sola pista).

use std::path::PathBuf;

// Una capa de la mezcla: archivo (nombre, no ruta, dentro de assets/), su volumen
// propio y si está prendida.
pub struct SavedLayer {
    pub file: String,
    pub volume: f32,
    pub on: bool,
}

#[derive(Default)]
pub struct SavedState {
    pub layers: Vec<SavedLayer>,
    // None = nunca se eligió volumen, o sea que es el primer inicio.
    pub volume: Option<f32>,
    // Id estable del dispositivo de salida; None = predeterminado de Windows.
    pub device: Option<String>,
}

impl SavedState {
    fn path() -> PathBuf {
        crate::app_dir().join(".pluvia_state")
    }

    pub fn load() -> Self {
        let mut state = Self::default();
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            return state;
        };
        // Sin la marca BOM que agregan algunos editores al guardar en UTF-8: si no, la
        // primera línea no se reconoce.
        let text = text.trim_start_matches('\u{feff}');
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            match line.split_once('=') {
                // layer=<archivo>|<volumen>|<1 prendida, 0 apagada>
                Some(("layer", value)) => {
                    let mut parts = value.rsplitn(3, '|');
                    let (Some(on), Some(volume), Some(file)) = (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let Some(volume) = volume.parse().ok().filter(|v| (0.0..=1.0).contains(v)) else {
                        continue;
                    };
                    state.layers.push(SavedLayer { file: file.to_string(), volume, on: on == "1" });
                }
                Some(("track", value)) => state.add_legacy_track(value),
                Some(("volume", value)) => {
                    state.volume = value.parse().ok().filter(|v| (0.0..=1.0).contains(v));
                }
                Some(("device", value)) => state.device = Some(value.to_string()),
                Some(_) => {}
                None => state.add_legacy_track(line),
            }
        }
        state
    }

    // La pista única de las versiones viejas pasa a ser la única capa prendida, a
    // volumen completo para que suene igual que antes de actualizar.
    fn add_legacy_track(&mut self, file: &str) {
        self.layers.push(SavedLayer { file: file.to_string(), volume: 1.0, on: true });
    }

    pub fn layer(&self, file: &str) -> Option<&SavedLayer> {
        self.layers.iter().find(|l| l.file == file)
    }

    pub fn save(&self) {
        let mut text = String::new();
        for layer in &self.layers {
            text += &format!("layer={}|{:.2}|{}\n", layer.file, layer.volume, u8::from(layer.on));
        }
        if let Some(volume) = self.volume {
            text += &format!("volume={volume:.2}\n");
        }
        if let Some(device) = &self.device {
            text += &format!("device={device}\n");
        }
        let _ = std::fs::write(Self::path(), text);
    }
}
