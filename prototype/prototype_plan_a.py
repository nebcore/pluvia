import numpy as np          #para el ruido
from scipy import signal    #diseño y filtros
import sounddevice as sd    #audio en tiempo real

SAMPLE_RATE = 48000
BLOCK_SIZE = 1024
CHANNELS = 2          # tu headset es estéreo
LOW_CUT = 400.0        # Hz, borde inferior del pasabanda
HIGH_CUT = 4000.0      # Hz, borde superior del pasabanda
LFO_FREQ = 0.1         # Hz, un ciclo completo cada 10s
LFO_DEPTH = 0.1        # ±10% de variación de amplitud
MASTER_GAIN = 0.06      # volumen general

sos = signal.butter(4, [LOW_CUT, HIGH_CUT], btype='band', fs=SAMPLE_RATE, output='sos')
zi_prototype = signal.sosfilt_zi(sos)
zi = np.stack([zi_prototype] * CHANNELS, axis=-1)

# Modelo de gota adaptado de Raindrop-Generator (JUCE), drop_v2.hpp:
# https://github.com/747745124/Raindrop-Generator
# Solo fase de resonancia (sinusoide amortiguada) — la fase de impacto
# original (pulso arccos) sonaba a glitch digital y se descartó.
DROP_GAIN = 0.1

def synthesize_drop():
    delta_t2 = 0.002 + np.random.uniform(0.0, 0.004)  # cuándo arranca la resonancia
    delta_t3 = delta_t2 + np.random.uniform(0.002, 0.006)  # cuándo termina la resonancia
    m = np.random.uniform(3.0, 15.0)    # tasa de decaimiento de la resonancia
    f = np.random.uniform(1000.0, 2000.0)  # frecuencia de la resonancia
    A1 = 1.2   # amplitud de la resonancia

    n = max(2, int(SAMPLE_RATE * delta_t3))
    t = np.arange(n) / SAMPLE_RATE
    out = np.zeros(n)

    ring_mask = (t >= delta_t2) & (t < delta_t3)
    tr = t[ring_mask] - delta_t2
    out[ring_mask] = np.exp(-m * tr / (delta_t3 - delta_t2)) * A1 * np.sin(2 * np.pi * f * tr)

    # fade-out final: garantiza que el array termine en 0 aunque la resonancia
    # no haya decaído del todo (evita el click por corte abrupto).
    fade_len = min(n, max(1, int(SAMPLE_RATE * 0.001)))
    fade = np.ones(n)
    fade[-fade_len:] = np.linspace(1.0, 0.0, fade_len)
    out *= fade

    return (out * DROP_GAIN).astype(np.float32)

sample_index = 0

next_drop_sample = np.random.randint(0, SAMPLE_RATE)  # primer disparo dentro del primer segundo
pending_drop = None
pending_drop_pos = 0

def schedule_next_drop():
    global next_drop_sample
    interval_s = np.random.uniform(0.02, 0.1)  # 10-50 gotas/seg
    next_drop_sample += int(interval_s * SAMPLE_RATE)

def callback(outdata, frames, time_info, status):
    global zi, sample_index, next_drop_sample, pending_drop, pending_drop_pos

    if status:
        print(status)

    noise = np.random.normal(0, 1, size=(frames, CHANNELS))
    filtered, zi = signal.sosfilt(sos, noise, axis=0, zi=zi)

    t = (sample_index + np.arange(frames)) / SAMPLE_RATE
    lfo = 1.0 + LFO_DEPTH * np.sin(2 * np.pi * LFO_FREQ * t)

    outdata[:] = filtered * lfo[:, None] * MASTER_GAIN

    block_start = sample_index
    block_end = sample_index + frames

    if pending_drop is not None:
        remaining = len(pending_drop) - pending_drop_pos
        take = min(remaining, frames)
        chunk = pending_drop[pending_drop_pos:pending_drop_pos + take]
        outdata[:take, 0] += chunk
        outdata[:take, 1] += chunk
        pending_drop_pos += take
        if pending_drop_pos >= len(pending_drop):
            pending_drop = None
            pending_drop_pos = 0

    while next_drop_sample < block_end:
        drop = synthesize_drop()
        offset = max(0, next_drop_sample - block_start)
        space = frames - offset
        take = min(space, len(drop))
        outdata[offset:offset + take, 0] += drop[:take]
        outdata[offset:offset + take, 1] += drop[:take]
        if take < len(drop):
            pending_drop = drop
            pending_drop_pos = take
        schedule_next_drop()

    sample_index += frames

if __name__ == "__main__":
    stream = sd.OutputStream(
        samplerate=SAMPLE_RATE,
        blocksize=BLOCK_SIZE,
        channels=CHANNELS,
        dtype='float32',
        callback=callback,
    )
    with stream:
        print("Reproduciendo lluvia (Plan A). Presiona Enter para detener...")
        input()


