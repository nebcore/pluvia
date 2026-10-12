# Changelog — Pluvia

Registro de cambios por versión. La versión vigente es la de `Cargo.toml`, que es la
única fuente de verdad: el diálogo "Acerca de" y las propiedades del `.exe` la toman
de ahí.

## Cómo se numeran las versiones

Formato `MAYOR.MENOR.PARCHE` ([SemVer](https://semver.org/lang/es/)):

| Sube… | Cuándo | Ejemplo |
|---|---|---|
| **PARCHE** | solo arreglos de bugs, sin nada nuevo visible | 0.2.0 → 0.2.1 |
| **MENOR** | algo nuevo: una opción de menú, una función, un cambio de comportamiento | 0.2.1 → 0.3.0 (el parche vuelve a 0) |
| **MAYOR** | se queda en 0 mientras sea beta. Pasa a 1.0.0 cuando se considere estable; después sube solo con cambios que rompan algo (p. ej. que se pierda la configuración guardada) | 0.9.0 → 1.0.0 |

Al publicar una versión:
1. Cambiar `version` en `Cargo.toml`.
2. Mover lo de "Sin publicar" a una sección nueva con la versión y la fecha.
3. Armar el paquete con `powershell -ExecutionPolicy Bypass -File packaging\package.ps1` (compila y deja `Pluvia app/Pluvia-vX.Y.Z.zip`).

Mientras se trabaja, los cambios se anotan en "Sin publicar".

---

## Sin publicar

_(nada todavía)_

## 1.0.0 — 2026-10-11

Primera versión estable y primera publicada en el repositorio.

### Nuevo
- **Ícono nuevo** (una flor, hecho por un amigo) en el `.exe` y en la bandeja. El `icon.ico` trae todos los tamaños (16 a 256 px) y la bandeja pide a Windows el que corresponde a la escala de pantalla, así se ve nítido al 100 %, 125 %, 150 % o 200 %.

### Arreglado
- Cambiar `icon.ico`/`icon.png` no recompilaba el ícono del `.exe` (faltaba avisarle a cargo que `build.rs` depende de ellos).
- Un `icon.ico` propio junto al `.exe` se cargaba en su tamaño más grande y Windows lo achicaba; ahora se pide el tamaño justo.

### Medición (release)
- En pausa: 0 % de CPU, ~17,9 MB (+0,7 MB por el ícono multitamaño respecto de 0.4.0).

## 0.4.0 — 2026-10-11

### Nuevo
- **Mezcla de sonidos:** cada audio de `assets/` es una capa que se prende o apaga con un interruptor y, prendida, tiene su propio volumen (el slider de arriba pasa a ser el volumen general). Play/Pausa y el clic derecho pausan o reanudan toda la mezcla con fade; cada capa entra y sale con su propio fade. Prender un sonido con todo en pausa hace sonar la mezcla; apagar el último la pausa.
- Con **más de 4 sonidos prendidos** aparece un aviso en rojo de que consume más rendimiento (no es un límite: se pueden prender más).
- Se recuerda la mezcla entre reinicios (qué sonidos están prendidos y su volumen). El formato viejo de `.pluvia_state` (una sola pista) se sigue leyendo: esa pista pasa a ser la única capa prendida, a volumen completo.
- "Agregar audio..." suma el sonido nuevo prendido a la mezcla.
- **Eliminar sonidos:** clic derecho sobre un sonido → menú contextual → "Eliminar pista". Si estaba sonando se apaga con fade; el archivo va a la **Papelera de Windows** (recuperable), no se borra.
- `--panel` (solo en builds de desarrollo) abre el panel al iniciar, para revisar el diseño sin clickear la bandeja.

### Arreglado
- Un `.pluvia_state` guardado con marca BOM (UTF-8 de algunos editores) perdía su primera línea; ahora se ignora la marca.

### Cambiado
- La sección "AUDIOS" del panel pasa a llamarse "SONIDOS".
- El paquete incluye solo los sonidos de una lista fija del script (hoy, la lluvia), no todo lo que haya en `assets/` del proyecto. El `LEEME.txt` explica la mezcla y cómo agregar/eliminar sonidos.
- Medido en release: en pausa sigue en 0 % de CPU con cualquier cantidad de sonidos prendidos; sonando, cada sonido suma ~0,4–0,5 % de un núcleo y ~0,25 MB (1 sonido ~1 %, 5 sonidos ~3 %).

## 0.3.2 — 2026-10-11

### Nuevo
- **Indicador de reproducción** en el encabezado del panel (reemplaza el círculo): tres barritas quietas, azules y desparejas mientras suena, grises y bajas en pausa. Es estático a propósito: animado costaba CPU con el panel abierto (~8 % con `animation-tick()`, ~1,5 % a 8 cuadros/s); quieto no agrega nada (~0,4–0,8 % con el panel abierto y sonando, medido en release).
- Etiqueta **"Volumen"** bajo el final del slider, sin agregar altura al panel.

### Arreglado
- El slider de volumen quedaba corrido hacia arriba respecto del botón de play y el porcentaje.

## 0.3.1 — 2026-10-11

### Arreglado
- Al arrancar con "Iniciar con Windows", el panel no marcaba la pista cargada: mostraba "Elige un audio" aunque la lluvia sonara, y clickear la pista no hacía nada.

## 0.3.0 — 2026-10-11

### Nuevo
- **Panel oscuro propio** en lugar del menú nativo de Windows: se abre con clic izquierdo en el ícono, junto a él, y se cierra al hacer clic afuera o con Esc. Hecho con [Slint](https://slint.dev).
- **Clic derecho en el ícono: play/pausa** directo, sin abrir nada.
- **Slider de volumen** continuo (también con la rueda del mouse), en vez de los niveles fijos.
- **Elección del dispositivo de salida** (o "Predeterminado de Windows"). Se recuerda entre reinicios; si el elegido se desconecta suena el predeterminado y vuelve al elegido cuando se reconecta.
- **Bienvenida en el primer inicio**: pide elegir el volumen (arranca en 20 %) antes de usar la app.
- Se recuerda el **volumen** entre reinicios, además de la pista.
- Si no hay audios, el panel se abre solo al iniciar y el primer audio que se agregue empieza a sonar.

### Cambiado
- El volumen inicial por defecto bajó de 50 % a 20 % (a pedido de un tester: arrancaba muy fuerte).
- `.pluvia_state` pasa a guardar `clave=valor` (pista, volumen, salida). El formato viejo se sigue leyendo.
- El ícono de la bandeja va incrustado en el `.exe` (un `icon.png` al lado sigue sirviendo para reemplazarlo), y el paquete pasa a ser `Pluvia/` con `Pluvia.exe`, `LEEME.txt` y `assets/`.
- **Iniciar con Windows** ya no arranca sonando: deja el ícono listo en la bandeja con la pista cargada y en pausa (se registra con `--autostart`; las entradas viejas se actualizan solas al abrir la app).
- Play/Pausa en medio de un fade lo invierte en vez de ignorar el clic.
- Si se desconecta el dispositivo de salida, el audio se pasa solo a otro.
- La app ya no se despierta en pausa: los clics del ícono llegan por evento y los timers corren solo durante un fade o mientras suena.
- **En pausa se cierra la salida de audio** (antes seguía mandando silencio a la tarjeta de sonido) y al dar play se reabre retomando donde iba. Al iniciar no se abre hasta que algo suena. Medido en release: en pausa pasó de ~0,4 % de CPU y 18,4 MB a **0 % y 16,9 MB**; sonando queda en ~1 % de un núcleo (lo que cuesta decodificar el mp3).
- Al cambiar de dispositivo de salida mientras suena, sigue desde donde iba en vez de volver al principio.

## 0.2.0 — 2026-10-10

### Nuevo
- Opción **"Iniciar con Windows"** en el menú (autoinicio por usuario, sin permisos de administrador).
- El submenú Volumen marca con un tilde el nivel actual.
- Si no hay audios, el mensaje de inicio indica que se puede usar *Audios > Agregar audio...*.

### Arreglado
- Silencio al final del loop después de varias pausas: el reencolado ahora usa el largo real de la cola en vez de estimar tiempos.
- Cambiar el volumen durante un fade-in ya no se pierde.
- Sin dispositivo de audio (o si falla el ícono de la bandeja), la app avisa con un diálogo en vez de cerrarse sin mostrar nada.
- Un audio que no se puede decodificar muestra un solo aviso y pausa, en vez de repetir el diálogo.
- Clickear la pista que ya suena ya no le saca el tilde.
- Al agregar el primer audio desaparece el ítem "(sin archivos en assets/)".
- Si se mueve el `.exe`, el autoinicio se actualiza a la ruta nueva al abrirlo.
- Al iniciar con Windows sin `assets/` junto al `.exe`, ya no intenta crear la carpeta en `System32`.

## 0.1.0 — 2026-08-22

Primera beta repartida (`Pluvia app/PLUVIA.zip`).
- Reproducción en loop de un audio de `assets/` (mp3/wav/ogg/flac) con fade-in/fade-out.
- Menú en la bandeja: Play/Pausa, Audios (cambiar pista y agregar audio), Volumen, Ajustes, Acerca de y Salir.
- Recuerda la última pista elegida (`.pluvia_state`).
- Ícono propio en la bandeja y en el `.exe`.
