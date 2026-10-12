# 🌧️ Pluvia

Sonidos ambientales de fondo (empezando por la lluvia) desde la bandeja de Windows. Liviano: en pausa no consume CPU.

## Descargar

En **Releases**, bajar `Pluvia-vX.Y.Z.zip`, descomprimir y abrir `Pluvia.exe`. Solo Windows.

## Uso

- **Clic izquierdo** en el ícono de la bandeja: abre el panel.
- **Clic derecho**: play / pausa.
- En el panel se pueden mezclar varios sonidos, cada uno con su volumen.
- **Agregar audio...** suma tus propios sonidos (mp3, wav, ogg, flac). Clic derecho sobre uno para eliminarlo.

## Compilar

Requiere Rust y Windows. Los sonidos no están en el repositorio: copiar alguno a `assets/` (la lluvia viene en el zip de cada release).

```bash
cargo run --release
```

Cambios por versión: [CHANGELOG.md](CHANGELOG.md). Interfaz hecha con [Slint](https://slint.dev).
