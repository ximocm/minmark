# CI y releases

Los workflows se ejecutan en [GitHub Actions](https://github.com/ximocm/minmark/actions).
Las contribuciones se revisan mediante pull request; `CODEOWNERS` asigna todos
los archivos a `@ximocm`.

## Cobertura por plataforma

| Plataforma | Pruebas | Aplicación y paquete |
| --- | --- | --- |
| Ubuntu 24.04 x86_64 | Los siete crates del core y lógica de la UI | Binario release y `.tar.gz` |
| Windows Server 2022 | Los siete crates del core | `minmark.exe` portátil (ZIP), MSI y bundle con WebView2 |
| macOS 15 (Apple Silicon) | Los siete crates del core | `minmark.app` en `.dmg` y `.tar.gz` |

Los siete crates del core — `minmark-document`, `minmark-config`,
`minmark-math`, `minmark-parser`, `minmark-renderer`, `minmark-pdf` y
`minmark-fs` — se prueban en los tres sistemas operativos. El filesystem es
portable por diseño: `minmark-fs` vigila con `inotify` en Linux (API del
sistema, sin dependencias), con `ReadDirectoryChangesW` vía `notify` en
Windows y con FSEvents vía `notify` en macOS (§46: notificaciones nativas en
los tres, nunca rescaneo continuo); la papelera del sistema es XDG en Linux y
la Papelera de reciclaje/`~/.Trash` vía `trash` en Windows/macOS. Las rutas de
configuración, datos y estado se resuelven en un único punto
(`minmark_config::dirs`), con las ubicaciones XDG de Linux intactas.

La UI se compila en los tres sistemas operativos tras el motor de Preview
nativo (ADR-005): WebKitGTK en Linux, WebView2 en Windows y WKWebView en
macOS. Los paquetes de Windows y macOS los genera la misma matriz de CI; un
build verde comprueba que compilan, se empaquetan y arrancan sin DLLs/dylibs
ausentes, pero no sustituye la QA gráfica manual. En Windows y macOS esa
verificación manual sigue pendiente (ADR-005: «build verde» no equivale a
«confirmación manual»).

## Integración continua

[ci.yml](../.github/workflows/ci.yml) se ejecuta en cada PR y push a `main`.
Usa Rust estable, dependencias bloqueadas por los `Cargo.lock` y dos procesos
de compilación para limitar el uso de memoria de Typst. Las acciones están
fijadas a commits y el token del workflow solo tiene permiso de lectura.

El job **CI success** solo pasa cuando toda la matriz termina correctamente.
El job Linux prueba y compila en modo release, comprueba las bibliotecas
compartidas y empaqueta el mismo binario. El artefacto `linux-x86_64` se conserva
14 días y permite revisar el paquete desde la PR sin publicar una release.

En Windows, la matriz instala GTK 4 con `gvsbuild` (los proyectos de
`GVSBUILD_PROJECTS` se construyen una vez y su árbol `C:\gtk-build\gtk` se
cachea con una clave que combina la versión de gvsbuild y la lista de
proyectos), instala WiX v6 como herramienta `dotnet` (v5 dejó de tener
soporte, ADR-005) y descarga de Microsoft, durante la ejecución, el
instalador **offline** del WebView2 Evergreen. `scripts/package-windows.py`
recorre las tablas de importación PE de `minmark.exe` para copiar las DLLs
no-sistema (GTK 4 y el cargador de WebView2 si está presente), arranca el
ejecutable empaquetado bajo un tiempo de espera para descartar el error
`0xC0000135` (DLL ausente) y produce el ZIP portátil (SRS §50.2), el MSI y
el bundle Burn `minmark-<versión>-windows-x86_64.exe`, que instala el
runtime de WebView2 solo si falta. El artefacto `windows-x86_64` se conserva
14 días.

En macOS, la matriz instala GTK 4 con Homebrew y
`scripts/package-macos.py` construye `minmark.app`: copia las dylibs
no-sistema a `Contents/Frameworks` recorriendo `otool -L` y reescribe sus
rutas con `install_name_tool` a `@executable_path`/`@loader_path` (sin
referencias a `/opt/homebrew`), firma ad-hoc con `codesign`, comprueba que
ninguna imagen queda con rutas foráneas, lanza el binario brevemente para
descartar dylibs ausentes y genera el `.dmg` y el `.tar.gz`. El artefacto
`macos-aarch64` se conserva 14 días.

El `.tar.gz` incluye el ejecutable, documentación de instalación, `LICENSE`,
`NOTICE`, los archivos de licencia suministrados por las dependencias y los
avisos de las fuentes embebidas de Typst. `SHA256SUMS` permite comprobar la
integridad del archivo. El binario requiere GTK ≥ 4.10 y WebKitGTK 6.0; se
compila en Ubuntu 24.04 y no es un binario estático para cualquier distribución.

## Crear una release

1. Actualiza de forma coherente la versión del core, UI y referencias internas,
   así como los lockfiles afectados. Prepara las notas del changelog.
2. Revisa y fusiona la PR en `main` tras superar CI.
3. Desde ese commit, crea y sube una etiqueta que coincida con los manifiestos:

   ```sh
   git switch main
   git pull --ff-only
   git tag -a v0.1.0 -m 'minmark 0.1.0'
   git push origin v0.1.0
   ```

4. [release.yml](../.github/workflows/release.yml) valida que el tag identifique
   un commit de `main` y que las versiones coincidan. Vuelve a ejecutar la matriz
   de CI y utiliza los paquetes de Linux, Windows y macOS que genera.
5. Si todo pasa, crea un **borrador** en
   [Releases](https://github.com/ximocm/minmark/releases), con los archivos de
   las tres plataformas (`.tar.gz`, `.zip`, `.msi`, el bundle de Windows,
   `.dmg`), `SHA256SUMS` y notas generadas. Revisa las notas, prueba la
   aplicación en una sesión gráfica y publica el borrador cuando corresponda.
   Marca como prerelease las versiones de prueba.

El workflow solo da permiso de escritura al job que crea el borrador, después
de validar y compilar. No hace push de código ni modifica las protecciones de
`main`. Un borrador conserva la publicación final bajo control del propietario.

Para repetir una ejecución fallida, usa **Re-run jobs**, o **Run workflow**
seleccionando el mismo tag como referencia y como entrada `tag`. Si ya existe
una release para esa etiqueta, el workflow se detiene: revisa el borrador antes
de decidir si procede eliminarlo y repetir. Nunca sobrescribe artefactos de una
release existente.

## Reproducir localmente

```sh
cargo test --locked --release --manifest-path core/Cargo.toml --workspace
cargo test --locked --release --manifest-path ui/Cargo.toml
cargo build --locked --release --manifest-path ui/Cargo.toml

# Paquete de la plataforma actual (uno por sistema operativo):
python3 scripts/package-linux.py    # Linux x86_64 -> .tar.gz
python scripts/package-windows.py   # Windows x86_64 -> .zip, .msi y bundle
python3 scripts/package-macos.py    # macOS arm64 -> .dmg y .tar.gz

(cd dist && sha256sum --check SHA256SUMS)   # macOS: shasum -a 256 --check SHA256SUMS
```

Cada script necesita Python ≥ 3.11, su propio sistema operativo y las
dependencias de [BUILDING.md](../BUILDING.md) (en Windows además, WiX v6 y
el instalador offline de WebView2; en macOS, las herramientas de línea de
comandos de Xcode). Respeta `CARGO_TARGET_DIR` si está definido. Los tests de
lógica no sustituyen la comprobación manual de ventanas, diálogos o Preview
antes de publicar.
