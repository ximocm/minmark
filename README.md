# minmark

**Fast. Local. Yours.**

Editor de Markdown de escritorio para Linux, escrito en Rust con GTK4 y
WebKitGTK. Trabaja con archivos y carpetas locales, sin cuentas ni servicios
cloud. Su objetivo es mantener Markdown como fuente de verdad, con edición
visual y vista previa en la misma ventana.

**En desarrollo:** las fases de core Linux, proyectos y edición visual están
completadas. La versión actual es `0.4.0`; consulta las novedades y el trabajo
pendiente en el [changelog](CHANGELOG.md).

Linux es la plataforma principal y es la que publica releases hoy. La CI
también compila y empaqueta la UI para Windows (ZIP portátil, MSI y bundle) y
macOS (`.app` en `.dmg` y `.tar.gz`) desde el mismo código (ADR-005); esos
paquetes son nuevos y su QA manual (ventanas, diálogos, Preview) sigue
pendiente antes de considerarlos listos para publicar.

## Qué puedes hacer

- Editar Markdown en pestañas y abrir varias ventanas de archivos o proyectos.
- Alternar entre Markdown y modo Visual editable sobre el mismo documento,
  con un único historial de deshacer/rehacer.
- Ver una previsualización HTML, con temas claro, oscuro y del sistema.
- Usar CommonMark, tablas y tareas GFM, además de extensiones propias como
  callouts y colores inline.
- Explorar carpetas, importar Markdown, renombrar y mover archivos con
  actualización de referencias compatibles.
- Buscar y reemplazar en un archivo o proyecto; abrir archivos y localizar
  encabezados con Quick Open.
- Navegar enlaces internos, completar destinos y detectar enlaces rotos.
- Consultar outline y backlinks temporales; habilitar wiki links por proyecto.
- Resolver referencias cruzadas en Preview y PDF, con etiquetas y numeración
  derivadas del documento. Consulta [Markdown Intelligence](docs/MARKDOWN_INTELLIGENCE.md).
- Componer ecuaciones LaTeX, usar bibliografía `.bib` y CSL, completar citas,
  generar TOC y notas al pie en Preview continuo/paginado y PDF.
  Consulta [Academic](docs/ACADEMIC.md) y su [registro de QA](docs/qa/registro-ejecucion.md).
- Combinar varios `.md` en una composición con Preview completo, referencias y
  numeración entre archivos, cabeceras/pies y un único PDF. Consulta
  [Composiciones](docs/COMPOSITIONS.md).

La exportación PDF está disponible en **File → Export PDF** (`Ctrl+E`) y la de
composiciones en **File → Export Composition PDF**, con su Preview en
**Project → Preview Composition**. Otras
funciones de la especificación siguen pendientes. Consulta el
[changelog](CHANGELOG.md) para conocer los cambios implementados y la pestaña
[Issues de GitHub](https://github.com/ximocm/minmark/issues) para seguir el trabajo pendiente.

## Compilar y ejecutar

Necesitas Linux, Rust estable reciente, `cargo`, `pkg-config`, GTK **≥ 4.10**
y WebKitGTK **6.0**, con sus paquetes de desarrollo. Los comandos siguientes
se ejecutan desde la raíz del repositorio.

En Ubuntu 24.04:

```sh
sudo apt update
sudo apt install build-essential pkg-config libgtk-4-dev libwebkitgtk-6.0-dev
cargo build --locked --manifest-path ui/Cargo.toml --release
./target/release/minmark
```

La compilación de la UI incluye los siete crates del core. La primera vez
necesita descargar dependencias y compilar Typst y los bindings de GTK/WebKit.
El binario requiere las bibliotecas compartidas del sistema; no es un ejecutable
estático portable.

```sh
./target/release/minmark notas.md
./target/release/minmark mi-proyecto/
./target/release/minmark notas.md otra-nota.md
```

Cada carpeta abre una ventana de proyecto; los archivos se abren en pestañas de
una ventana independiente. También puedes abrirlos desde el menú **File**.

En Windows y macOS la UI se compila con GTK 4 del sistema (gvsbuild y
Homebrew) y su Preview usa los motores nativos WebView2 y WKWebView; los
paquetes para esas plataformas se generan con los scripts incluidos.

Consulta [BUILDING.md](BUILDING.md) para compilación de desarrollo, tests,
instalación local y solución de problemas.

## Atajos esenciales

| Atajo | Acción |
| --- | --- |
| `Ctrl+N` / `Ctrl+O` / `Ctrl+S` | Nuevo / Abrir / Guardar |
| `Ctrl+Shift+S` / `Ctrl+W` | Guardar como / Cerrar pestaña |
| `Ctrl+Shift+N` / `Ctrl+Shift+O` | Nueva ventana / Abrir carpeta |
| `Ctrl+B` | Mostrar u ocultar el explorador |
| `Ctrl+F` / `Ctrl+H` | Buscar / Reemplazar en el archivo |
| `Ctrl+Shift+F` | Buscar en el proyecto |
| `Ctrl+P` | Quick Open |
| `Ctrl+Enter` / `Ctrl+Click` | Abrir enlace bajo el cursor |
| `Ctrl+Alt+O` / `Ctrl+Alt+B` | Outline / Backlinks |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Deshacer / Rehacer |
| `Ctrl+Alt+M` | Alternar Markdown y Visual |

En Quick Open, `:N` salta a una línea, `@heading` busca encabezados del documento
y `#heading` busca encabezados del proyecto. **Help ▸ Atajos** muestra los
atajos disponibles en la aplicación.

## Documentación y desarrollo

- [Compilación, tests e instalación local](BUILDING.md).
- [Cómo contribuir y comunicar errores](CONTRIBUTING.md).
- [Changelog](CHANGELOG.md).
- [CI y releases](docs/CI.md): pruebas del core y paquetes de Linux, Windows y macOS.
- [Índice de documentación](docs/README.md): especificación y decisiones.
- [Cómo citar minmark](docs/CITING.md): referencias APA y BibTeX del software.
- [Documento de demostración](docs/SHOWCASE.md): todo el dialecto con ejemplos
  editables (tablas, imágenes, colores, énfasis, citas, matemáticas…).
- [Arquitectura del core](core/README.md).
- [Preparación para publicar en GitHub](docs/PUBLISHING.md).

```text
core/         Siete crates Rust: documento, configuración, parser,
              renderer, matemáticas, PDF y sistema de archivos
ui/           Aplicación GTK4 + WebKitGTK; binario minmark
docs/         Especificación y decisiones de arquitectura
```

`core/` y `ui/` tienen manifiestos y lockfiles separados. No hay un
`Cargo.toml` en la raíz. Ambos comparten `target/` para reutilizar dependencias
compiladas; consulta [BUILDING.md](BUILDING.md) para liberar ese espacio.

## Archivos y datos locales

Los documentos se guardan como archivos de texto UTF-8. El editor preserva los
saltos de línea LF/CRLF y detecta modificaciones externas para evitar
sobrescrituras silenciosas.

Los índices de proyecto se guardan bajo `$XDG_DATA_HOME/minmark/index/`
(por defecto `~/.local/share/minmark/index/`). El almacenamiento de snapshots
del core usa el mismo directorio de datos, en `minmark/snapshots/`. El estado del
explorador se guarda en `$XDG_STATE_HOME/minmark/ui.ini` (por defecto
`~/.local/state/minmark/ui.ini`).

## Privacidad en Windows

Preview en Windows se apoya en el WebView2 Evergreen de Microsoft, un
componente del sistema que puede enviar a Microsoft sus propios diagnósticos.
minmark no añade telemetría ni acceso a red propio (§3.4). En Linux y macOS
el Preview usa WebKitGTK y WKWebView, sin ese componente. Consulta
[ADR-005](docs/adr/005-preview-multiplataforma.md).

## Licencia

Copyright © 2026 Ximo. El proyecto se distribuye bajo
[Apache License 2.0](LICENSE).

Es software **open source**: permite uso comercial, modificación y
redistribución bajo los términos de Apache-2.0. Consulta [LICENSE](LICENSE) y
[NOTICE](NOTICE) para los términos y avisos que debes conservar al redistribuir.
Las dependencias mantienen sus propias licencias.
