// Reproducción: una mezcla de capas (sonidos en loop, cada uno con su volumen) sobre
// una sola salida de audio, con fades. No sabe nada de la interfaz; `main.rs` lo maneja
// y decide cuándo correr los timers de `tick`/`refill`.
//
// La salida de audio (el stream hacia la tarjeta de sonido) se abre solo mientras algo
// suena o hace fade. En pausa se cierra y se recuerda la posición de cada capa, para que
// la app en segundo plano no gaste nada mandando silencio; al volver a dar play se
// reabre y cada capa retoma desde donde iba.
//
// Cada capa prendida tiene su propio Player conectado al mixer de la salida, y rodio
// los mezcla. Lo que cuesta CPU es decodificar: cada capa sonando suma su archivo.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::cpal::{self, StreamError};
use rodio::stream::MixerDeviceSink;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

pub const FADE_DURATION: Duration = Duration::from_millis(400);
// Volumen propio con el que entra una capa que nunca se ajustó. Debajo del máximo para
// dejar margen: varias capas a full sumadas pueden saturar.
pub const LAYER_DEFAULT_VOLUME: f32 = 0.7;

// Dispositivos de salida conectados ahora mismo, como (id, nombre). El id es estable
// entre reinicios y reconexiones, así que es lo que se guarda para recordar la
// elección; el nombre es solo para mostrar (puede repetirse entre dos iguales).
pub struct OutputDevices {
    pub default: Option<(String, String)>,
    pub devices: Vec<(String, String)>,
}

fn device_entry(device: &cpal::Device) -> Option<(String, String)> {
    Some((device.id().ok()?.to_string(), device.description().ok()?.name().to_string()))
}

pub fn output_devices() -> OutputDevices {
    let host = cpal::default_host();
    OutputDevices {
        default: host.default_output_device().as_ref().and_then(device_entry),
        devices: host
            .output_devices()
            .map(|devices| devices.filter_map(|d| device_entry(&d)).collect())
            .unwrap_or_default(),
    }
}

// Lo que se avisa (desde el hilo de audio) si el dispositivo se desconecta.
pub type LostNotifier = Box<dyn Fn() + Send + 'static>;

// Salida abierta: el stream hacia el dispositivo. Los Players de las capas cuelgan de su
// mixer, así que siempre se sueltan antes que esto (ver `Audio::close`).
struct Output {
    sink: MixerDeviceSink,
    device_id: Option<String>,
}

// Abre la salida en el dispositivo con ese id, o en el predeterminado de Windows si es
// None o si ya no está conectado.
fn open_output(device_id: Option<&str>, on_lost: LostNotifier) -> Result<Output, String> {
    let host = cpal::default_host();
    let device = device_id
        .and_then(|id| {
            host.output_devices()
                .ok()?
                .find(|d| d.id().is_ok_and(|d_id| d_id.to_string() == id))
        })
        .or_else(|| host.default_output_device())
        .ok_or("No hay ningún dispositivo de salida de audio.")?;
    let opened_id = device.id().ok().map(|id| id.to_string());
    let mut sink = DeviceSinkBuilder::from_device(device)
        .map_err(|e| format!("No se pudo abrir la salida de audio: {e}"))?
        .with_error_callback(move |e| {
            if matches!(e, StreamError::DeviceNotAvailable | StreamError::StreamInvalidated) {
                on_lost();
            }
        })
        .open_stream()
        .map_err(|e| format!("No se pudo abrir la salida de audio: {e}"))?;
    sink.log_on_drop(false);
    Ok(Output { sink, device_id: opened_id })
}

// Abre un decoder *nuevo y liviano* (streaming, no precarga el archivo entero) y lo
// encola, empezando en `start_at`. Así evitamos dos problemas a la vez:
// - decodificar todo el archivo a un Vec en RAM de una (inviable con tracks largos,
//   ~1.8GB para un mp3 de 90min)
// - Source::repeat_infinite(), que tiene un leak de memoria conocido
//   (RustAudio/rodio#673: crece ~10MB cada 15s)
// El costo es reabrir el archivo cada vez que se vacía la cola, que es barato.
fn queue_fresh_source(player: &Player, path: &Path, start_at: Duration) -> Result<(), String> {
    let mut decoder = File::open(path)
        .map_err(|e| e.to_string())
        .and_then(|f| Decoder::try_from(f).map_err(|e| e.to_string()))
        .map_err(|e| format!("No se pudo decodificar el audio: {e}"))?;
    if !start_at.is_zero() {
        // Si no se puede saltar, simplemente arranca desde el principio.
        let _ = decoder.try_seek(start_at);
    }
    player.append(decoder);
    Ok(())
}

// Una rampa de ganancia de `from` a `to` en FADE_DURATION.
#[derive(Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    start: Instant,
}

impl Fade {
    fn new(from: f32, to: f32) -> Self {
        Self { from, to, start: Instant::now() }
    }

    // Ganancia actual y si la rampa ya terminó.
    fn value(&self) -> (f32, bool) {
        let t = (self.start.elapsed().as_secs_f32() / FADE_DURATION.as_secs_f32()).min(1.0);
        (self.from + (self.to - self.from) * t, t >= 1.0)
    }
}

pub struct Layer {
    path: PathBuf,
    volume: f32,
    on: bool,
    // Solo existe con la salida abierta y la capa sonando (o apagándose con fade).
    player: Option<Player>,
    // Ganancia propia 0..1 para que la capa entre y salga suave al prenderla/apagarla.
    gain: f32,
    fade: Option<Fade>,
    // Posición donde retomar al volver a abrir la salida.
    resume_at: Duration,
}

impl Layer {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    fn position(&self) -> Duration {
        self.player.as_ref().map_or(self.resume_at, |p| p.get_pos())
    }

    // Suelta el player recordando dónde iba.
    fn stop(&mut self) {
        self.resume_at = self.position();
        self.player = None;
        self.fade = None;
    }
}

pub struct Audio {
    output: Option<Output>,
    // Dispositivo elegido (None = predeterminado de Windows); se usa al abrir la salida.
    device_pref: Option<String>,
    // Crea el aviso de "dispositivo perdido" para cada salida que se abre.
    make_notifier: Box<dyn Fn() -> LostNotifier>,
    layers: Vec<Layer>,
    // Players de capas quitadas que todavía se están apagando con fade: ya no están en
    // `layers` (así los índices del panel no cambian de golpe) y se sueltan al terminar.
    leftovers: Vec<(Player, f32, Fade)>,
    // Volumen general (el slider de arriba); multiplica al de cada capa.
    master_volume: f32,
    // Ganancia general 0..1 para el fade de play/pausa.
    master_gain: f32,
    master_fade: Option<Fade>,
    // Intención del usuario: sonando (o por sonar) o en pausa. En pausa la salida se
    // cierra al terminar el fade-out.
    playing: bool,
}

impl Audio {
    pub fn new(
        device_pref: Option<String>,
        master_volume: f32,
        make_notifier: impl Fn() -> LostNotifier + 'static,
    ) -> Self {
        Self {
            output: None,
            device_pref,
            make_notifier: Box::new(make_notifier),
            layers: Vec::new(),
            leftovers: Vec::new(),
            master_volume,
            master_gain: 0.0,
            master_fade: None,
            playing: false,
        }
    }

    // Agrega una capa (apagada o prendida, sin empezar a sonar) y devuelve su índice.
    pub fn add_layer(&mut self, path: PathBuf, volume: f32, on: bool) -> usize {
        self.layers.push(Layer {
            path,
            volume,
            on,
            player: None,
            gain: 1.0,
            fade: None,
            resume_at: Duration::ZERO,
        });
        self.layers.len() - 1
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    // Saca una capa de la mezcla y devuelve su archivo. Si estaba sonando, su player se
    // apaga con fade aparte (ver `leftovers`) y el archivo sigue abierto hasta que
    // termine: por eso devuelve también si hay que esperar FADE_DURATION para tocarlo.
    pub fn remove_layer(&mut self, index: usize) -> Option<(PathBuf, bool)> {
        if index >= self.layers.len() {
            return None;
        }
        let layer = self.layers.remove(index);
        let still_open = layer.player.is_some();
        if let Some(player) = layer.player {
            let volume = player.volume();
            self.leftovers.push((player, volume, Fade::new(1.0, 0.0)));
        }
        if self.active_count() == 0 {
            self.pause();
        }
        Some((layer.path, still_open))
    }

    pub fn active_count(&self) -> usize {
        self.layers.iter().filter(|l| l.on).count()
    }

    // Id del dispositivo en el que está sonando ahora (None si la salida está cerrada).
    pub fn device_id(&self) -> Option<&str> {
        self.output.as_ref()?.device_id.as_deref()
    }

    pub fn is_open(&self) -> bool {
        self.output.is_some()
    }

    pub fn volume(&self) -> f32 {
        self.master_volume
    }

    // Sonando o por sonar (un fade-out hacia la pausa ya cuenta como pausado).
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_fading(&self) -> bool {
        self.master_fade.is_some() || !self.leftovers.is_empty() || self.layers.iter().any(|l| l.fade.is_some())
    }

    // La salida está abierta y el audio avanza (incluye el fade-out): mientras tanto
    // hay que ir reencolando.
    pub fn is_audible(&self) -> bool {
        self.output.is_some()
    }

    fn apply_volumes(&self) {
        for layer in &self.layers {
            if let Some(player) = &layer.player {
                player.set_volume(self.master_volume * self.master_gain * layer.volume * layer.gain);
            }
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = volume;
        self.apply_volumes();
    }

    pub fn set_layer_volume(&mut self, index: usize, volume: f32) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.volume = volume;
        }
        self.apply_volumes();
    }

    // Crea el player de una capa (si no lo tiene) y la encola desde donde iba.
    fn start_layer(&mut self, index: usize) -> Result<(), String> {
        let Some(output) = &self.output else { return Ok(()) };
        let layer = &mut self.layers[index];
        if layer.player.is_some() {
            return Ok(());
        }
        let player = Player::connect_new(output.sink.mixer());
        player.set_volume(0.0);
        if let Err(e) = queue_fresh_source(&player, &layer.path, layer.resume_at) {
            layer.on = false;
            return Err(e);
        }
        layer.player = Some(player);
        Ok(())
    }

    // Abre la salida si hace falta y hace sonar todas las capas prendidas, con fade-in
    // general. Sin capas prendidas no hace nada.
    pub fn play(&mut self) -> Result<(), String> {
        if self.active_count() == 0 {
            return Ok(());
        }
        if self.output.is_none() {
            self.output = Some(open_output(self.device_pref.as_deref(), (self.make_notifier)())?);
        }
        let mut result = Ok(());
        for i in 0..self.layers.len() {
            if self.layers[i].on
                && let Err(e) = self.start_layer(i)
            {
                result = Err(e);
            }
        }
        self.playing = true;
        self.master_fade = Some(Fade::new(self.master_gain, 1.0));
        self.apply_volumes();
        if self.active_count() == 0 {
            // Todas fallaron al abrirse: no queda nada que suene.
            self.close();
        }
        result
    }

    // Fade-out general; al terminar, `tick` cierra la salida.
    pub fn pause(&mut self) {
        if !self.playing {
            return;
        }
        self.playing = false;
        self.master_fade = Some(Fade::new(self.master_gain, 0.0));
    }

    // Play/pausa con fade. Si se toca en medio de un fade, lo invierte desde donde iba.
    pub fn toggle(&mut self) -> Result<(), String> {
        if self.playing {
            self.pause();
            Ok(())
        } else {
            self.play()
        }
    }

    // Prende o apaga una capa. Prenderla con todo en pausa hace sonar la mezcla; apagar
    // la última pausa todo.
    pub fn toggle_layer(&mut self, index: usize) -> Result<(), String> {
        let Some(layer) = self.layers.get_mut(index) else { return Ok(()) };
        layer.on = !layer.on;
        if !layer.on {
            if layer.player.is_some() {
                layer.fade = Some(Fade::new(layer.gain, 0.0));
            }
            if self.active_count() == 0 {
                self.pause();
            }
            return Ok(());
        }
        if !self.playing {
            // La capa entra junto con el fade-in general.
            layer.gain = 1.0;
            return self.play();
        }
        // Con la mezcla sonando, la capa entra sola con su propio fade-in (o, si se
        // estaba apagando, vuelve desde donde iba).
        if layer.player.is_none() {
            layer.gain = 0.0;
        }
        layer.fade = Some(Fade::new(layer.gain, 1.0));
        let result = self.start_layer(index);
        self.apply_volumes();
        result
    }

    // Avanza los fades en curso. Devuelve true mientras siga habiendo alguno. Al
    // terminar el fade-out general cierra la salida; al terminar el de una capa apagada
    // suelta su player.
    pub fn tick(&mut self) -> bool {
        if let Some(fade) = self.master_fade {
            let (gain, done) = fade.value();
            self.master_gain = gain;
            if done {
                self.master_fade = None;
            }
        }
        for layer in &mut self.layers {
            if let Some(fade) = layer.fade {
                let (gain, done) = fade.value();
                layer.gain = gain;
                if done {
                    layer.fade = None;
                    if !layer.on {
                        layer.stop();
                        layer.gain = 1.0;
                    }
                }
            }
        }
        self.leftovers.retain(|(player, volume, fade)| {
            let (gain, done) = fade.value();
            player.set_volume(volume * gain);
            !done
        });
        self.apply_volumes();
        if !self.playing && self.master_fade.is_none() && self.output.is_some() {
            self.close();
        }
        self.is_fading()
    }

    // Suelta todos los players (recordando posiciones) y cierra la salida.
    fn close(&mut self) {
        for layer in &mut self.layers {
            layer.stop();
            layer.gain = 1.0;
        }
        self.leftovers.clear();
        self.output = None;
        self.master_gain = 0.0;
        self.master_fade = None;
        self.playing = false;
    }

    // Cambia el dispositivo elegido. Con `move_now`, si está sonando, la salida se mueve
    // al nuevo y sigue desde donde iba; si no, se usa la próxima vez que se dé play.
    pub fn set_device(&mut self, device_pref: Option<String>, move_now: bool) -> Result<(), String> {
        self.device_pref = device_pref;
        if move_now { self.reopen() } else { Ok(()) }
    }

    // Vuelve a abrir la salida si estaba abierta (p. ej. porque se desconectó el
    // dispositivo o cambió el predeterminado), retomando cada capa donde iba.
    pub fn reopen(&mut self) -> Result<(), String> {
        if self.output.is_none() {
            return Ok(());
        }
        let was_playing = self.playing;
        self.close();
        if was_playing { self.play() } else { Ok(()) }
    }

    // Mantiene siempre una fuente fresca en cola detrás de la que suena en cada capa,
    // así los loops empalman sin cortes (ver `queue_fresh_source`). Si una capa falla,
    // se apaga (y si era la última, se pausa todo) para no repetir el error.
    pub fn refill(&mut self) -> Result<(), String> {
        let mut result = Ok(());
        for layer in &mut self.layers {
            let Some(player) = &layer.player else { continue };
            if player.len() >= 2 {
                continue;
            }
            if let Err(e) = queue_fresh_source(player, &layer.path, Duration::ZERO) {
                layer.on = false;
                layer.stop();
                result = Err(e);
            }
        }
        if result.is_err() && self.active_count() == 0 {
            self.pause();
        }
        result
    }
}
