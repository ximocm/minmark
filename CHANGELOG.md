# Changelog

Cambios notables para usuarios y colaboradores. `Unreleased` reúne el trabajo
que todavía no se ha publicado como release.

El [historial técnico de las fases 0–3](docs/DEVELOPMENT_HISTORY.md) conserva las
entradas originales con decisiones, referencias a la especificación y pruebas.
El estado del trabajo pendiente se mantiene en GitHub Issues.

## [Unreleased]

### Añadido

- Controles editoriales contextuales (fase 7, #29; §44): popover flotante
  anclado al cursor/selección del editor que se muestra solo cuando aporta
  utilidad — panel de texto (Heading 1–6, Bold, Italic, Strikethrough, Code,
  Link, Callout) con selección no vacía, «Edit link» con el cursor dentro de
  un enlace, controles de tono (note, info, tip, warning, danger, success)
  dentro de un callout y H1–H6 bajo el cursor en un heading; cursor plano
  sin selección lo mantiene oculto. Escape (y el clic fuera) lo cierra sin
  tocar el documento, `Ctrl+.` lleva el foco del editor a los controles
  —con foco en ellos, Tab los recorre, Enter/Space los activan (GTK
  nativo) y Escape los cierra devolviendo el foco al editor—, y cambiar de
  pestaña o de modo lo cierra sin aplicar nada a otra pestaña.
- Menús **Insert** (Link, Callout) y **Format** (Heading 1–6, Bold, Italic,
  Strikethrough, Code) en el orden §36 `File Edit View Insert Format Project
  Help`, sin toolbar permanente (§36/§44).
- Command Palette §38 (`Ctrl+Shift+P`): ventana fuzzy sobre las etiquetas de
  todos los comandos; Enter o doble click ejecutan el comando, Escape
  cierra.
- Atajos §39 nuevos, sin conflicto con los existentes: `Ctrl+Shift+B` Bold,
  `Ctrl+I` Italic, `Ctrl+K` Link, `Ctrl+Shift+X` Strikethrough,
  `Ctrl+Shift+C` Code, `Ctrl+1..6` Heading 1–6 y `Ctrl+Shift+P` Command
  Palette (`Ctrl+B` sigue siendo Toggle Explorer y `Ctrl+P` Quick Open).
- Cada acción editorial es UNA transacción de undo/redo (§8.5) que registra
  la selección anterior y posterior (§8.6): undo restaura la selección
  original y redo la resultante. Todas las vías de entrada — menús, atajos,
  Command Palette y popover — ejecutan la misma acción `win.*` con destino
  única `TabManager::apply_format_active`; el diálogo de enlace precarga la
  URL del enlace bajo el cursor y Cancelar (o Escape) no toca el documento.
  La ✕ de la Command Palette la oculta en lugar de destruirla: se reutiliza
  en la siguiente apertura.

### Limitaciones aceptadas (#29)

- Solo enlaces `[texto](url)`: wiki links y reference links no tienen
  controles contextuales.
- Code inline multilínea no es aplicable: `format::plan` lo rechaza y el
  botón Code queda inoperante en ese rango (el documento no se toca).
- Atajos configurables (§39) fuera de alcance: se registran solo los
  defaults sin conflicto; no hay editor de bindings.

### Verificación

- `cargo test` del binario UI: 130 en verde (12 ignorados que requieren
  sesión gráfica) y `cargo clippy --all-targets` sin avisos nuevos respecto
  al HEAD (mismo conjunto de 15 avisos preexistentes del toolchain 1.98).
- Las seis pruebas gráficas nuevas corren bajo Xvfb con `--ignored`:
  Bold vía `win.format-strong` con selección y undo únicos, diálogo de
  enlace cancelado byte a byte idéntico, popover oculto con cursor
  plano/visible con selección, cambio de pestaña y de modo con el popover
  abierto sin aplicar nada, retonado de callout desde el popover y Command
  Palette ejecutando un comando vía `win.*`.

## [0.4.3] — 2026-10-26

### Corregido

- Las ecuaciones inline cuyo primer carácter es Unicode (por ejemplo
  `$α_0$`) podían cerrar la app al escribir, abrir o guardar. El analizador
  conserva los límites UTF-8 y tolera también las ecuaciones incompletas.
- Windows: Guardar un documento nuevo, Guardar como y otros selectores de
  archivos podían cerrar el proceso con «No GSettings schemas are installed
  on the system». El ZIP y el MSI incluyen los schemas de GTK compilados, y
  la app los localiza junto al ejecutable. Un paquete incompleto muestra un
  error recuperable y conserva el documento abierto.
- Preview nativa de Windows y macOS: su tamaño se actualiza al redimensionar
  la ventana y cambiar las vistas; el layout de la caja anfitriona impedía
  que GTK llamara a la actualización. En Windows también se convierte la
  escala lógica de GTK a píxeles físicos, incluido el DPI fraccionario.
- Cerrar una pestaña guardada con Ctrl+W y confirmar una sobrescritura externa
  ya no mantienen préstamos del modelo durante el refresco de la interfaz,
  que podían provocar un cierre por `RefCell`.

### Verificación y limitaciones

- La autocomprobación `--check-preview` comprueba también el guardado y la
  sobrescritura confirmada, cierre de pestaña, redimensionado, cambios
  Split/Preview y barra lateral. Abre y cancela un selector real en
  Windows/Linux; en macOS esa interacción requiere comprobación manual.
  En Windows incluye maximizar/restaurar.
- La regresión de Unicode cubre parseo, highlighting y modo Visual con
  caracteres de dos, tres y cuatro bytes y sus prefijos durante la edición.
  El diagnóstico gráfico edita una ecuación Unicode en el editor real y
  activa el mismo comando de guardado de Ctrl+S.
- Las comprobaciones del paquete Windows eliminan el GTK del entorno de
  compilación para detectar recursos ausentes en una instalación real.
- La build Windows fija la versión de gvsbuild al GTK validado, y la
  instalación de dependencias Linux acota esperas de red y evita preguntas
  interactivas en CI.
- La CI de Linux ejecuta los seis escenarios gráficos de integración en
  Xvfb, además de la lógica de UI.
- Registro y alcance de la validación adelantada de fase 8:
  [estabilidad Windows](docs/qa/estabilidad-windows-2026-10-25.md).
- Sigue pendiente la matriz manual de instalación/desinstalación MSI en
  Windows 10/11, los cambios entre monitores con distinto DPI y la
  interacción del selector nativo de macOS.

## [0.4.2] — 2026-10-25

### Añadido

- `minmark --check-preview`: autocomprobación de la Preview (pinta el
  documento, sigue las ediciones, funciona sobre una «Sin título» recién
  abierta y rindea el tema oscuro con contraste real) para diagnosticar
  instalaciones reales con un comando; los paquetes la ejecutan en CI (#57).

### Corregido

- Windows: la Preview salía en blanco siempre. WebView2 creaba su carpeta de
  datos junto al ejecutable (su comportamiento por defecto) y el MSI instala
  en `Program Files`, donde un usuario normal no puede escribir: el entorno
  del motor no se creaba y nada llegaba a pintarse. Usa ahora la carpeta por
  usuario `%LOCALAPPDATA%\minmark\webview2` (#57).
- Windows y macOS: los fallos de enganche o colocación de la vista nativa del
  Preview dejan rastro en stderr (antes eran retornos silenciosos) y el
  controlador de WebView2 se reintenta si el `realize` llega antes de que
  exista la superficie o el HWND (#57).
- macOS: el `INSTALL.txt` del paquete recomendaba abrir con «clic derecho →
  Abrir», atajo que Apple eliminó en macOS Sequoia; documenta la vía actual
  (Ajustes → Privacidad y seguridad → «Abrir de todas formas») (#57).

### Verificación y limitaciones

- 422 pruebas del core (Linux, Windows Server 2022 y macOS 15), 119 de UI
  (6 ignoradas: requieren sesión gráfica) y 23 de la política de release
  pasan; la suite de UI completa corre ahora también en Windows y macOS.
- La autocomprobación del Preview (`minmark --check-preview`) pasa en los
  tres SO, tanto sobre el binario como sobre los paquetes que genera la CI
  (fichero, edición, «Sin título» y tema oscuro con contraste real).
- El fix de Windows (carpeta de datos de WebView2) está verificado en CI con
  el paquete generado; queda pendiente la confirmación sobre una instalación
  MSI real en el equipo del usuario.
- macOS: el fallo de Preview reportado no se reproduce en CI ni con el
  binario ni con el `.app`; queda pendiente la verificación en el equipo del
  usuario con `minmark --check-preview` (#57).
- Siguen abiertos la lectura de Preview con Orca (#39) y las fuentes CJK de
  paginado/PDF (#40).

## [0.4.1] — 2026-10-22

### Añadido

- La barra de pestañas muestra una «X» en cada pestaña que cierra esa pestaña
  concreta (#52): no cambia la pestaña activa salvo que se cierre ella, pide
  la misma confirmación que Close Tab cuando tiene cambios sin guardar y
  mantiene siempre ≥1 pestaña.

### Corregido

- La pestaña activa usa grises y un subrayado discreto en lugar del color de
  acento del escritorio.
- Los cambios externos de un archivo se explican con un aviso visible y
  acciones para recargar desde disco o guardar una copia. La recarga pide
  confirmación antes de descartar cambios sin guardar; los guardados propios
  ya no generan falsos avisos de cambio externo.
- La compilación de la UI deja de avisar por cuatro métodos usados solo en
  tests. El ejemplo de desarrollo abre `README.md`, incluido en el repositorio.

### Verificación y limitaciones

- 422 pruebas del core (Linux, Windows Server 2022 y macOS 15), 119 de UI
  (5 ignoradas: requieren sesión gráfica) y 23 de la política de release
  pasan; la matriz de CI compila y empaqueta Linux, Windows y macOS.
- Las cinco pruebas gráficas se ejecutan una por proceso sobre una sesión
  X11 real y la revisión visual de la app queda en
  [docs/qa/evidence](docs/qa/evidence): barra con la «X» por pestaña, la
  confirmación de descarte y la barra del binario ejecutándose.
- Siguen pendientes la QA manual de Windows y macOS, la lectura de Preview
  con Orca (#39) y las fuentes CJK de paginado/PDF (#40).

## [0.4.0] — 2026-10-21

### Añadido

- [Cómo citar minmark](docs/CITING.md) y [`CITATION.cff`](CITATION.cff) en la
  raíz, con referencia APA y BibTeX exportable desde GitHub.
- [Documento de demostración](docs/SHOWCASE.md) con todo el dialecto: énfasis,
  listas y tareas, tablas, imágenes, colores inline, callouts, matemáticas,
  citas y bibliografía, notas, referencias cruzadas y front matter.
- Portabilidad del core a Windows y macOS: los siete crates del core compilan
  y pasan sus tests en los tres sistemas operativos. `minmark-fs` vigila con
  `inotify` en Linux, con `ReadDirectoryChangesW`/FSEvents vía `notify` en
  Windows/macOS (§46: notificaciones nativas, sin rescaneo) y adapta symlinks,
  permisos y papelera (XDG en Linux; Papelera de reciclaje y `~/.Trash` vía
  `trash` en Windows/macOS). El descubrimiento de directorios por SO vive en
  `minmark_config::dirs`, con las ubicaciones XDG de Linux byte-idénticas. La
  matriz de CI ejecuta ahora el workspace completo del core también en
  Windows y en macOS 15 ([CI](docs/CI.md),
  [BUILDING](BUILDING.md)).
- Preview multiplataforma
  ([ADR-005](docs/adr/005-preview-multiplataforma.md)): el motor del Preview
  se oculta tras `PreviewEngine` con tres implementaciones — WebKitGTK en
  Linux (sin cambio de comportamiento), WebView2 en Windows y WKWebView en
  macOS — con el mismo HTML/CSS, los mismos scripts de actualización y la
  misma política de enlaces y navegación.
- Ejecutables por sistema operativo en cada release (§50.2): `.tar.gz`
  (Linux), MSI y ZIP portable (Windows 10/11 x86-64, con el prerrequisito
  WebView2 en el bundle) y `.app` en `.dmg`/`.tar.gz` (macOS arm64), con
  `SHA256SUMS` común. Los paquetes se construyen en los runners de GitHub y
  la disponibilidad y sus límites se documentan en [CI](docs/CI.md) y
  [BUILDING](BUILDING.md).

### Corregido

- El parser interpretaba `::: tipo` y `$$` dentro de fences de código como
  callouts y ecuaciones de bloque, en contra de §7.4.1 («código primero»). Los
  fences de `` ` `` y `~` protegen ahora su contenido también frente a las
  extensiones de bloque.
- Portabilidad del core a Windows y macOS: la rama XDG de
  `minmark_config::dirs` evalúa la absolutitud con semántica POSIX («empieza
  por `/`») en cualquier host — antes `Path::is_absolute` de Windows rechazaba
  bases XDG como `/home/ana` —, y `minmark-fs` fija el namespace canónico de
  las raíces vigiladas y de las claves de evento del backend `notify`
  (macOS: `/var/folders/…` frente a `/private/var/folders/…`), incluida la
  síntesis de bajas cuando un directorio desaparece. Las ubicaciones y el
  comportamiento de Linux siguen siendo byte-idénticos.
- Rutas de proyecto en Windows y compilación de la UI en Windows/macOS:
  `minmark-fs` normaliza el namespace canónico eliminando el prefijo verbatim
  de Windows (`\\?\C:\…` → `C:\…`, `\\?\UNC\…` → `\\servidor\…`), de modo que
  las comparaciones por prefijo dejan de clasificar rutas internas del
  proyecto como ajenas (§48); toda canonicalización del core y de la UI pasa
  por esa vía única. El resolutor admite las rutas absolutas de Windows
  (`C:\…`, `\\servidor\…`) como internas del filesystem y `has_scheme` ya no
  toma la unidad `X:` por un esquema de URL. En la UI el watcher pasa a ser el
  nativo multiplataforma, la tabla de dependencias de macOS se activa de verdad
  (`cfg(target_os = "macos")`) y el host de WKWebView implementa el
  subclaseado GTK y los marcadores de hilo principal que exige objc2.

### Verificación y limitaciones

- 422 pruebas del core (Linux, Windows Server 2022 y macOS 15), 115 de UI
  (3 ignoradas, preexistentes) y 23 de la política de release pasan; la
  matriz de CI compila, empaqueta y hace smoke tests de los tres paquetes
  (`.tar.gz`, MSI + bundle Burn + ZIP portable, `.dmg`/`.tar.gz`).
- Pendientes la QA manual de Windows y macOS (instalación y desinstalación
  del MSI, apertura del `.app`, Preview con scroll, edición y enlaces), la
  lectura del contenido de Preview con Orca (#39) y las fuentes CJK de
  paginado/PDF (#40).

## [0.3.0] — 2026-10-20

### Añadido

- Fase 6: composiciones multiarchivo declaradas en `.mdproject.toml`
  (`[[composition]]`), con Preview combinado (continuo o paginado) y exportación
  de un único PDF desde **Project → Preview Composition** y **File → Export
  Composition PDF**. Enlaces, referencias cruzadas y numeración entre archivos
  incluidos se resuelven como internos del documento final; los niveles de
  heading se preservan y `page_break_between_files` separa archivos. Documentación:
  [Composiciones](docs/COMPOSITIONS.md).
- Cabeceras y pies de página configurables (`pdf_header`/`pdf_footer`) con las
  variables `{title}`, `{author}`, `{project}`, `{chapter}`, `{page}`, `{pages}`
  y `{date}`, posición de número de página en las seis ubicaciones de §32.8 y
  tamaño de página Custom en mm configurable.
- Fuentes base incluidas (sans-serif, serif y monoespaciada) sin depender de las
  fuentes del sistema en PDF, Preview paginado y matemáticas; `{{pagebreak}}`
  muestra un separador discreto en el modo Visual.
- Fase 5: ecuaciones LaTeX compuestas, bibliografía BibTeX/CSL local, citas y
  autocompletado, notas al pie, numeración académica y TOC.
- Preview paginado comparte la composición de páginas, notas y TOC del PDF.
- Fixtures, suite y [registro de QA académica](docs/qa/registro-ejecucion.md).
- Exportación PDF con el ajuste de tablas §35.2 (ISSUE-027): una tabla que no
  cabe se reduce proporcionalmente hasta `table_min_font_pt` (defecto 8 pt) sin
  perder el reparto multipágina ni el caption debajo; si ni siquiera el suelo
  cabe, la exportación se detiene nombrando la tabla problemática —caption
  expandido, ID o nº de orden por orden de documento— con el mensaje
  `error de composición PDF: panicked with: "<tabla>: no cabe en el ancho
  disponible con table_min_font_pt=8pt (§35.2)"` en lugar de recortarla.
  Una tabla que ya cabe nunca se amplía, aunque el cuerpo esté por debajo del
  suelo. Los anchos se miden siempre a la tipografía del cuerpo (sin heredar
  la medida de notas al pie) y como runs sin quiebre que solo parten en
  espacio/tab ASCII; el reparto de columnas se emite con anchos calculados en
  Typst que respetan la mínima por columna (Typst 0.13.1 no admite `minmax`),
  de modo que ningún token inseparable sale del área de texto. Documentación:
  [Composiciones](docs/COMPOSITIONS.md).

### Cambiado

- El editor ajusta visualmente las líneas largas al ancho de la vista
  (`WrapMode::WordChar`): el texto se parte solo en pantalla, sin insertar
  saltos de línea en el documento y sin scroll horizontal. Es un cambio solo
  visual sobre el `wrap-mode` `GTK_WRAP_NONE` de §34.2 en la vista de edición;
  Preview y PDF no cambian.

### Corregido

- La exportación PDF se detenía con «No se puede resolver la imagen: …» cuando
  el documento referenciaba un recurso por su ruta absoluta del filesystem
  (p. ej. `/home/…/imagen.png`), aunque el Preview la mostraba bien: el
  resolutor interpreta ahora una ruta absoluta que cae dentro del proyecto
  como su path relativo (§17/§48, symlinks incluidos) y sigue marcando
  `Missing` todo lo que sale de la raíz. La lectura `/x` = `x` desde la raíz
  del proyecto conserva su prioridad; navegación y diagnósticos de enlaces
  aplican el mismo criterio que Preview y PDF.
- El autocompletado expone la sugerencia seleccionada al lector de pantalla,
  conserva el foco del editor y restaura su nombre accesible al cerrar el popup.
- Tabla PDF de varias columnas, composición de fracciones y aislamiento de
  glifos SVG; exportación con los ajustes vigentes del buffer sin guardar.

### Verificación y limitaciones

- 412 pruebas del core y 115 de UI (3 ignoradas, preexistentes) pasan en Linux;
  el core se verifica además en Windows y macOS. Integración gráfica en
  X11/Wayland, capturas y PDF multipágina inspeccionados; Orca anuncia
  sugerencias de citas.
- Pendientes la lectura del contenido de Preview con Orca (#39) y las fuentes
  CJK de paginado/PDF (#40). Clippy de los paquetes modificados pasa con
  warnings; el workspace conserva un error previo en `minmark-fs`.

## [0.2.0] — 2026-10-13

### Añadido

- Navegación de enlaces con Ctrl+Click/Ctrl+Enter en Markdown y Visual y desde
  Preview; autocompletado de archivos y headings, diagnósticos de enlaces rotos,
  wiki links opcionales y vistas temporales de outline y backlinks.
- Referencias cruzadas a IDs de headings, figuras, tablas y ecuaciones,
  coherentes en Preview/PDF, con idioma español/inglés y numeración configurable.
- Pies de figura numerados a partir del texto alternativo. El PDF incorpora
  las imágenes locales mediante recursos en memoria.
- Resolución compartida de destinos y anclas, con buffers sin guardar,
  archivos ignorados, Unicode y límites del proyecto. Índice reconstruible v3
  con IDs de objetos y timestamps con nanosegundos.
- CI del core en Linux, Windows y macOS; compilación y paquete de la app Linux
  x86_64 con licencias y checksums.

### Corregido

- Preview se actualiza sin recargar la página y sigue el bloque editado cuando
  queda fuera de vista, conservando el scroll en otros refrescos.
- Las flechas mantienen visible la selección en Quick Open, outline, backlinks
  y autocompletado, sin quitar el foco al campo de escritura.

### Verificación y limitaciones

- 335 pruebas del core, 89 de UI y una integración gráfica GTK pasan. Revisión
  manual de los siete pasos de Markdown Intelligence y proyecto de 1000 notas.
- Exportación PDF revisada con buffer sin guardar, menú/atajo, cancelación,
  sobrescritura, destino no escribible y recuperación tras un error del motor.
- Pendientes las comprobaciones con lector de pantalla, Wayland y confirmación
  manual específica del seguimiento de Preview al escribir; este último tiene
  cobertura gráfica automatizada. Detalles en
  [la guía de fase 4](docs/MARKDOWN_INTELLIGENCE.md).
- La aplicación distribuida es para Linux x86_64 (Ubuntu 24.04 y compatibles),
  con GTK ≥ 4.10 y WebKitGTK 6.0. Windows/macOS tienen pruebas de core, no binarios
  de escritorio. Funciones académicas ampliadas, composición y controles
  contextuales siguen en desarrollo.

## [0.1.1] — 2026-10-13

### Cambiado

- Exportación PDF desde `File → Export PDF` y `Ctrl+E`, sin botón adicional
  en la barra de menús.
- Core y UI comparten `target/` para reutilizar dependencias compiladas.
  Desarrollo omite información de depuración y caché incremental; release
  elimina los símbolos del ejecutable para reducir el espacio ocupado.
- Guía de compilación e instalación actualizada, con instrucciones para
  liberar los archivos de compilación después de copiar el ejecutable.

### Eliminado

- Prototipos de la Fase 0 y benchmarks históricos, incluidos los corpus locales.
  Las decisiones de arquitectura conservan sus métricas de referencia.

### Corregido

- Error de compilación por la referencia a `gtk::Button` del botón PDF retirado.

## [0.1.0]

### Añadido

- `CODEOWNERS` con `@ximocm` como revisor de todos los archivos y flujo de
  contribución por pull request para el repositorio público.

- Aplicación Linux en Rust con GTK4 y WebKitGTK: pestañas, ventanas múltiples,
  apertura de archivos y carpetas, diálogos nativos y atajos de teclado.
- Edición Markdown con deshacer/rehacer y modo Visual editable sobre el mismo
  documento. Visual admite encabezados, énfasis, enlaces, código, citas, listas,
  tablas, etiquetas de imagen, callouts y colores inline; permite revelar la
  sintaxis del elemento bajo el cursor.
- Parser CommonMark/GFM con extensiones propias, vista previa HTML y temas
  claro, oscuro y del sistema.
- Modo proyecto: explorador, importación de Markdown, operaciones de archivos,
  actualización de referencias al renombrar o mover, índice incremental y
  detección de cambios externos.
- Búsqueda y reemplazo en archivo y proyecto, opciones de expresiones regulares
  y reemplazo masivo deshacible.
- Quick Open con búsqueda fuzzy, archivos recientes, salto a línea y búsqueda
  de encabezados del documento o proyecto.
- Guardado atómico, preservación de LF/CRLF, validación de rutas y
  almacenamiento de snapshots en el core.
- Motor PDF básico del core basado en Typst, fuentes embebidas y traducción
  de un subconjunto de LaTeX a Typst para matemáticas.
- Especificación, decisiones de arquitectura y
  prototipos con resultados de benchmarks.
- Guías de compilación, contribución y publicación en GitHub, índice de
  documentación y plantillas de errores, propuestas y pull requests.

### Cambiado

- Licencia del proyecto cambiada de PolyForm Noncommercial 1.0.0 a Apache-2.0.
  Actualizados los manifiestos, la documentación y el diálogo Acerca de;
  añadido `NOTICE` con la atribución de autoría.

- Seguimiento trasladado a GitHub Issues; eliminados los archivos de issues
  locales y actualizado el flujo de contribución y las plantillas.

- Historial Git reiniciado con un commit inicial limpio; corpus de benchmarks
  conservados localmente y excluidos del repositorio.

- README reorganizado para distinguir funciones disponibles de trabajo futuro.
- Documentación de arquitectura y prototipos actualizada; comandos desde la
  raíz del repositorio con manifiestos explícitos y lockfiles conservados.
- Historial técnico extenso separado de las notas de cambios para usuarios.
- Especificación actualizada hasta SRS v1.3: gramática de extensiones,
  configuración y gestión de límites de snapshots.

### Corregido

- Comandos con diálogos que no respondían por una referencia débil inválida.
- Fallo de arranque de WebKit en sistemas donde no funciona `bwrap`: se detecta
  la situación y se desactiva su sandbox. La limitación de aislamiento se
  explica en [BUILDING.md](BUILDING.md).
