# Compilar minmark

Todos los comandos de esta guía parten de la raíz del repositorio. La aplicación
actual está dirigida a Linux y necesita una sesión gráfica para ejecutarse.

## Requisitos

- Rust estable reciente y Cargo. No se ha establecido una versión mínima de
  Rust (MSRV). En la revisión de documentación se usa `rustc 1.98.1`.
- Compilador y enlazador del sistema, más `pkg-config`.
- GTK ≥ 4.10 y WebKitGTK 6.0 con cabeceras de desarrollo.

En Ubuntu 24.04:

```sh
sudo apt update
sudo apt install build-essential pkg-config libgtk-4-dev libwebkitgtk-6.0-dev
```

En otras distribuciones instala los paquetes equivalentes y comprueba sus
versiones. WebKitGTK 4.0/4.1 no sustituye a la API 6.0 utilizada aquí.

```sh
rustc --version
cargo --version
pkg-config --modversion gtk4 webkitgtk-6.0
```

El core no necesita GTK ni WebKit. Typst y sus fuentes se incorporan como
dependencias Rust; no hace falta instalar un ejecutable `typst`.

## Compilación

```sh
# Aplicación de escritorio optimizada; incluye el core
cargo build --locked --manifest-path ui/Cargo.toml --release

# Desarrollo: compilar y abrir un archivo
cargo run --locked --manifest-path ui/Cargo.toml -- notas.md

# Solo el core
cargo build --locked --manifest-path core/Cargo.toml --workspace
```

`--locked` conserva las versiones de los `Cargo.lock` versionados. La primera
compilación requiere acceso a los registros de dependencias; una vez descargadas,
puedes añadir `--offline`. La compilación, especialmente Typst, puede consumir
bastante memoria y disco. Si el compilador termina por falta de memoria, prueba
con `-j 1`.

Sin `CARGO_TARGET_DIR` personalizado, los binarios quedan en
`target/release/minmark` o `target/debug/minmark`. La configuración
`.cargo/config.toml` hace que core y UI compartan `target/`, sin duplicar sus
dependencias comunes cuando los perfiles y features coinciden.

El perfil de desarrollo omite información de depuración y desactiva la caché
incremental para reducir disco. Esto limita el detalle del depurador y puede
ralentizar recompilaciones. Release elimina los símbolos del ejecutable.

## Ejecución e instalación local opcional

```sh
./target/release/minmark
./target/release/minmark notas.md otra-nota.md
./target/release/minmark mi-proyecto/
```

Para copiar el ejecutable al directorio de binarios del usuario:

```sh
install -Dm755 target/release/minmark "$HOME/.local/bin/minmark"
```

Añade `~/.local/bin` al `PATH` si tu shell no lo incluye. Para desinstalar esa
copia, elimina `~/.local/bin/minmark`. Este procedimiento no instala un paquete,
un lanzador de escritorio ni las bibliotecas compartidas: GTK y WebKitGTK deben
seguir instalados. Tampoco elimina tus documentos ni los datos locales descritos
en el [README](README.md#archivos-y-datos-locales).

## Liberar espacio después de compilar

Cargo conserva los archivos de compilación para reutilizarlos. Después de
instalar o copiar el ejecutable fuera de `target/`, puedes eliminar la carpeta
compartida:

```sh
cargo clean --manifest-path ui/Cargo.toml
```

Esto también elimina los binarios de `target/`, pero conserva el ejecutable
instalado. La siguiente compilación será completa.

## Tests y comprobaciones

La matriz automatizada y la generación de paquetes se explican en
[CI y releases](docs/CI.md). Windows y macOS prueban los seis crates portables;
la aplicación de escritorio se compila y empaqueta para Linux.

```sh
cargo test --locked --manifest-path core/Cargo.toml --workspace
cargo test --locked --manifest-path ui/Cargo.toml
```

Los tests de lógica de la UI se pueden ejecutar sin abrir ventanas, pero el
binario de tests necesita las bibliotecas GTK/WebKit para compilar y enlazar.
No sustituyen a una prueba manual de diálogos, edición y previsualización.

Para revisar formato y diagnósticos, si tienes los componentes `rustfmt` y
`clippy` instalados:

```sh
cargo fmt --manifest-path core/Cargo.toml --all -- --check
cargo fmt --manifest-path ui/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path core/Cargo.toml --workspace --all-targets
cargo clippy --locked --manifest-path ui/Cargo.toml --all-targets
```

Antes de publicar un binario, comprueba manualmente apertura y guardado,
alternancia Markdown/Visual, deshacer/rehacer, Preview, búsqueda y proyecto.
Verifica la exportación PDF cuando se complete su integración en la UI.

## Problemas frecuentes

### No encuentra `Cargo.toml`

No hay workspace en la raíz. Utiliza `--manifest-path core/Cargo.toml` o
`--manifest-path ui/Cargo.toml`, como en los ejemplos anteriores.

### No encuentra GTK o WebKit, o la versión de GTK es insuficiente

Comprueba `pkg-config --modversion gtk4 webkitgtk-6.0`. Necesitas los paquetes de
**desarrollo**, y GTK ≥ 4.10. Si usas un prefijo de instalación personalizado,
configura `PKG_CONFIG_PATH` para ese prefijo.

### No se puede abrir el display

Ejecuta la aplicación dentro de una sesión gráfica Wayland o X11. Compilar y
pasar los tests de lógica no requiere abrir la aplicación.

### Fallo de `dbus-proxy` o `bwrap` al iniciar Preview

El arranque actual prueba si `bwrap` funciona y desactiva el sandbox de WebKit
si la prueba falla, por ejemplo en sistemas que restringen los user namespaces.
Esto reduce el aislamiento del proceso de previsualización. Al informar del
fallo, incluye la salida de terminal, la distribución y la versión de WebKitGTK.

### Error al usar `--locked`

Un cambio en las dependencias puede requerir actualizar el lockfile del
manifiesto afectado. En desarrollo, ejecuta Cargo sin `--locked` para resolverlo
y revisa el diff del `Cargo.lock`. Para reproducir una versión del proyecto,
utiliza el manifiesto y lockfile del mismo commit.
