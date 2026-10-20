# Compilar minmark

Todos los comandos de esta guía parten de la raíz del repositorio. La
aplicación necesita una sesión gráfica para ejecutarse; Linux es la
plataforma principal y Windows y macOS se empaquetan desde CI con las mismas
fuentes (ADR-005).

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

### En Windows

- Rust estable con toolchain MSVC (`rustup default stable-msvc`) y Visual
  Studio 2022 con la carga de trabajo de C++ (en los runners de GitHub ya
  está instalado).
- MSYS2, que gvsbuild usa para herramientas auxiliares
  (https://www.msys2.org).
- GTK 4 compilado con [gvsbuild](https://github.com/wingtk/gvsbuild):

  ```sh
  pip install gvsbuild
  gvsbuild build gtk4 pkgconf
  ```

  La primera compilación tarda bastante. Después añade
  `C:\gtk-build\gtk\x64\release\bin` al `PATH` y define
  `PKG_CONFIG_PATH=C:\gtk-build\gtk\x64\release\lib\pkgconfig` y
  `PKG_CONFIG_ALLOW_SYSTEM_LIBS=1` para que Cargo encuentre GTK mediante
  `pkg-config`. La segunda variable es imprescindible: `pkgconf` recorta el
  `-L` de sus propios directorios de sistema — y el prefijo de gvsbuild lo
  es — y sin ella los crates `-sys` no dan la ruta a `link.exe`
  (`LNK1181: cannot open input file 'gtk-4.lib'`).
- Para generar el instalador: .NET SDK (instala WiX como herramienta `dotnet`)
  y el instalador offline del WebView2 Evergreen; ver «Empaquetado».

### En macOS (Apple Silicon)

```sh
brew install gtk4 pkg-config
```

GTK 4 procede de Homebrew (`/opt/homebrew`); la empaquetación copia sus
dylibs dentro del `.app` para que no dependan de Homebrew al ejecutarse.

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

## Empaquetado por plataforma

[CI y releases](docs/CI.md) explica la matriz completa. Los mismos scripts se
pueden ejecutar en local; cada uno produce los archivos de su plataforma en
`dist/` junto con `SHA256SUMS` (una línea por archivo). Todos necesitan
Python ≥ 3.11 y respetan `CARGO_TARGET_DIR`.

### Linux

```sh
cargo build --locked --release --manifest-path ui/Cargo.toml
python3 scripts/package-linux.py
```

`ldd` no debe reportar ninguna biblioteca «not found».

### Windows

```powershell
cargo build --locked --release --manifest-path ui/Cargo.toml
dotnet tool install --global wix --version 6.0.2
$env:WEBVIEW2_INSTALLER = "$env:TEMP\MicrosoftEdgeWebView2RuntimeInstallerX64.exe"
curl.exe -L -o $env:WEBVIEW2_INSTALLER "https://go.microsoft.com/fwlink/?linkid=2124701"
python scripts\package-windows.py
```

- WiX v6 es la toolchain del MSI (ADR-005: WiX v5 dejó de tener soporte en
  2026-02-05). El script añade las extensiones que necesita a la caché de WiX.
- `WEBVIEW2_INSTALLER` debe apuntar al instalador **offline** (Evergreen
  Standalone x64) descargado de Microsoft; el bundle Burn lo incrusta y solo
  instala el runtime de WebView2 si la máquina no lo tiene.
- El script recorre las tablas de importación PE de `minmark.exe`, copia las
  DLLs no-sistema (GTK 4 y el cargador de WebView2 si existe) junto al
  ejecutable, lo lanza brevemente (fallaría con `0xC0000135` si falta una
  DLL) y genera el `.zip` portátil, el `.msi` y el `.exe` del bundle.

### macOS

```sh
cargo build --locked --release --manifest-path ui/Cargo.toml
python3 scripts/package-macos.py
```

- Solo en Apple Silicon; usa `otool`, `install_name_tool`, `codesign` e
  `hdiutil` (Xcode Command Line Tools).
- El script copia las dylibs no-sistema de Homebrew a
  `minmark.app/Contents/Frameworks`, reescribe sus rutas a
  `@executable_path`/`@loader_path`, firma ad-hoc (Developer ID y
  notarización están pendientes: Gatekeeper avisa en el primer arranque),
  comprueba que no queda ninguna ruta de Homebrew, lanza el binario
  brevemente y crea el `.dmg` y el `.tar.gz`.

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
[CI y releases](docs/CI.md). Los siete crates del core se prueban en los tres
sistemas operativos; la aplicación de escritorio se compila y empaqueta para
cada plataforma.

```sh
# Pruebas del core en Linux, Windows (PowerShell/cmd) y macOS: mismo comando
cargo test --locked --manifest-path core/Cargo.toml --workspace

# En Windows y macOS, la matriz de CI ejecuta exactamente esto en modo release
cargo test --locked --release --manifest-path core/Cargo.toml --workspace

# Solo Linux (necesita GTK/WebKitGTK para compilar): lógica de la UI
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
