# Prototipos de la Fase 0

Experimentos históricos para comparar UI y parser antes de construir minmark.
No forman parte del binario actual. Para compilar la aplicación, sigue
[BUILDING.md](../BUILDING.md).

| Directorio | Propósito |
| --- | --- |
| `ui-webview/` | UI con GTK4 y WebKitGTK 6.0 |
| `ui-rust-native/` | UI nativa con iced 0.12 |
| `parser/` | Benchmark de pulldown-cmark sobre un archivo Markdown |

Cada prototipo tiene su propio manifiesto y lockfile. Requieren Rust y Cargo;
los prototipos gráficos necesitan una sesión gráfica y las bibliotecas de su
backend. El de WebView necesita los paquetes GTK/WebKit descritos en la guía de
compilación.

## Compilar y ejecutar

Desde la raíz del repositorio, ejecuta el prototipo que quieras medir:

```sh
cargo run --locked --release --manifest-path prototypes/ui-webview/Cargo.toml
cargo run --locked --release --manifest-path prototypes/ui-rust-native/Cargo.toml
cargo run --locked --release --manifest-path prototypes/parser/Cargo.toml -- ruta/al/corpus.md
```

El benchmark del parser acepta un archivo UTF-8 y muestra su tamaño, número de
eventos y tiempo de parseo. Los corpus históricos de `docs/benchmarks/` ocupan
más de lo que indican sus nombres; no son necesarios para compilar o probar la
aplicación. Consulta la [nota de publicación](../docs/PUBLISHING.md).

## Resultados

La Fase 0 está cerrada. Los resultados registrados se encuentran en
[`docs/benchmarks/`](../docs/benchmarks/), con el
[protocolo de medición](../docs/benchmarks/PROTOCOLO.md). La selección de GTK4 y
WebKitGTK está documentada en [ADR-001](../docs/adr/001-tecnologia-ui.md).

Para nuevas mediciones, registra commit, toolchain, tamaño real del corpus,
sistema y sesión gráfica. Los resultados históricos no sustituyen una medición
del editor actual.
