# Historial técnico anterior a la primera release

Archivo del changelog detallado de las fases 0–3, conservado el 2026-10-13.
Los recuentos de tests, dependencias y estados describen el momento de cada
entrada. Para cambios actuales consulta el [changelog](../CHANGELOG.md) y
GitHub Issues. Los identificadores `ISSUE-NNN` y las menciones al seguimiento
local son históricos: sus archivos se retiraron al adoptar GitHub Issues y sus
números no equivalen a los de GitHub. Las rutas citadas en texto se refieren
a la raíz del repositorio de aquella etapa.

## [Unreleased]

### Añadido

- `core/` + `ui/`: Visual de **callouts y colores inline + §8.4 completo y
  cierre de Fase 3** (`ISSUE-034`, §22/§23/§7.4.4/§8.2/§8.4): el plan visual
  cubre `HighlightKind::Callout` y `Color`. Callouts `::: tipo "título"`:
  `:::` de apertura (con su espacio) y de cierre y las comillas del título son
  sintaxis; `tipo` + título son la línea semántica con el color del tono
  (§23, mismo mapeo que el Preview: note/info/tip/warning/danger/success;
  tipos desconocidos o personalizados §22 extensible → color neutro `muted`)
  y el contenido va con sangrado + fondo de callout. Colores inline §7.4.4
  (`{nombre:}`, `{+nombre:}`, `{a+b:}`): sintaxis `{…: ` y `}` oculta y texto
  coloreado según reglas §23 (texto, fondo o combinación, con tags por slot de
  color); nombre desconocido = texto literal sin ocultar (§7.4.4). §8.4
  completo: math §24, citas §25, wiki links §19.3, sustituciones §7.4.2 y
  front matter §21 quedan como bloque Markdown editable (sintaxis visible) y
  los callouts sin `:::` de cierre son texto literal. Slots de color: `0..=16`
  = paleta §23 (orden de `Palette::colors`), `17`+ = colores personalizados
  (§58.2) con bucket `custom` mientras la config no llegue al parser. UI:
  tags `mm-vis-co-*` y `mm-vis-fg-*`/`mm-vis-bg-*`; `all_styles()` pasa a
  `Vec` para generar los tags de color. 8 tests nuevos en `minmark-parser` y
  2 en UI; verificación visual con la app real. **FASE 3 COMPLETADA**.

- `core/` + `ui/`: Visual de **tablas e imágenes** (`ISSUE-033`,
  §8.2/§8.3/§8.4/§18/§35.1/§7.4.3): el plan visual cubre
  `HighlightKind::Table` e `Image`. Tablas GFM: barras `|` (y escape `\|`)
  ocultas; fila delimitadora (`---`, `:--`, `:-:`, `--:`) como línea
  separadora con estilo de regla (`:` ocultos, `-` visibles con color de
  regla); celdas de cabecera en negrita; alineación GFM preservada en el
  buffer (§6.2). La línea de atributos propia posterior (§26.3, `{#id …}`,
  fuera del span del bloque porque `attach_attributes` la consume) se oculta
  con lookahead tras la tabla. Imágenes: `alt` como chip/etiqueta semántica
  (color `purple` + fondo de código) y ocultos `![`, `](`, ruta, título, `)`
  y los atributos ` {…}` (§18.5/§7.4.3); sin atributos válidos la sintaxis se
  queda visible (§8.4). §8.4: lo que no sea tabla GFM parseable no produce
  tramos. §35.1 verificado con la app real: tabla más ancha que el panel →
  scroll horizontal sin wrap ni reducción de tipografía (TextView
  `wrap-mode` por defecto `GTK_WRAP_NONE`, §34.2). UI: tags
  `mm-vis-thead`/`mm-vis-trule`/`mm-vis-imgalt`. 8 tests nuevos en
  `minmark-parser` y 1 en UI (tabla alineada/desalineada, alineaciones,
  atributos de tabla, imagen con y sin atributos, imagen dentro de link,
  §8.4).

- `core/` + `ui/`: Visual de **listas** (`ISSUE-032`, §8.2/§8.3/§8.4/§7.1):
  el plan visual cubre `HighlightKind::List` (viñetas `-`/`*`/`+` y numeradas
  `N.`/`N)`). Los marcadores con su espacio son sintaxis ocultable; la sangría
  de anidamiento se conserva visible (define la estructura, §6.2) y el
  contenido del ítem recibe `ListItem(nivel)` con sangría visual coherente con
  el nivel (nivel = función pura de la indentación del marcador, para que los
  tokens de listas anidadas no se contradigan al solaparse; las líneas de
  continúan heredan el nivel del ítem). Tareas GFM `[ ]`/`[x]`/`[X]`:
  permanecen **visibles** como marcador semántico con estilo propio (decisión
  ISSUE-032: ocultarlas perdería el estado). El interior de bloques de código
  no se interpreta (§7.4.1: fences rastreados con cualquier indentación, sin
  marcadores falsos). §8.4: un marcador suelto sin contenido no produce tramos
  y queda como bloque Markdown editable. El plan deduplica tramos idénticos
  (solapes exactos entre lista contenedora y anidada). UI: tags
  `mm-vis-li1`..`mm-vis-li6` + `mm-vis-task`. 6 tests nuevos en
  `minmark-parser` y 2 en UI; verificación visual con la app real (simple,
  anidada con continuación, numerada desde N y tareas).

- `core/` + `ui/`: Visual de **enlaces, código y cita** (`ISSUE-031`,
  §8.2/§8.3/§8.4/§7.4.1/§19.1): el plan visual (`minmark-parser::visual`)
  cubre ahora `HighlightKind::Link`, `Code`, `CodeBlock` y `Quote`. Enlaces
  `[texto](destino "título")`: `[`, `](`, destino, título y `)` son sintaxis
  ocultable y el texto se estiliza como enlace (subrayado + color de acento de
  la paleta §23); el cierre del texto se resuelve saltando llaves anidadas,
  code spans §7.4.1 y escapes §7.4.6. Code spans: backticks ocultos y
  contenido monoespaciado con fondo de código. Bloques cercados: fence de
  apertura (info-string incluida) y de cierre ocultos, contenido monoespaciado
  con fondo de código, nunca reinterpretado (§7.4.1); bloques indentados sin
  marcadores. Citas: marcadores `>` de todos los niveles ocultos («ocultar
  gana sobre mostrar» con tokens anidados) y líneas con sangrado + color de
  cita de la paleta. §8.4: constructos sin cerrar (link sin `]`, texto suelto)
  no producen tramos y quedan como bloque Markdown editable. UI: nuevos tags
  `mm-vis-link`/`mm-vis-code`/`mm-vis-codeblock`/`mm-vis-quote`; `TagStyle`
  gana `underline` y `left_margin` (aplicados en `apply_tag_style`). 12 tests
  nuevos en `minmark-parser` (plan por constructo, anidamientos `**` en link y
  code span en párrafo, §7.4.1 sin extensiones interiores, §8.4) y 3 en UI
  (rangos en chars + estilos); verificación visual con la app real (sintaxis
  oculta y estilos semánticos, `reveal-current` §8.3 revelando el elemento
  editado).

- `core/` + `ui/`: **modo Visual editable** sobre el buffer único (`ISSUE-030`,
  §8.2–§8.6/§6.1/§6.2): `minmark-parser::visual` (plan visual con tramos de
  sintaxis ocultable + contenido semántico y elementos por bloque, derivado de
  la misma gramática §4.2) y `ui/src/editor/visual` (traducción a chars + tags
  GTK). Visual es un plan de tags excluyente sobre el MISMO `TextBuffer` del
  Markdown Mode (decisión ISSUE-030): la sintaxis se oculta con tag invisible
  (medido en GTK 4.14: colapsa el ancho a cero) y el contenido se estiliza
  (headings con escala por nivel §27.1, `**negrita**`, `*cursiva*`); descartadas
  la WebView contenteditable (reconciliar HTML→Markdown rompe §6.2) y el editor
  de bloques con widgets (rompe §8.3 «ocultar siempre»). Cubre heading
  (ATX/setext + atributos §7.4.3), paragraph, bold e italic; §8.5 (undo único)
  y §8.6 (cursor/selección) por construcción: mismo buffer, mismo historial,
  mismos offsets. §8.3 configurable (`visual_syntax` = `hidden` |
  `reveal-current`, defecto `reveal-current`, nivel global §58.2) con revelado
  de la sintaxis del elemento (bloque) más interno bajo el cursor e ítem
  `Toggle Visual Syntax` en el menú View (§36). Se retira la vista Visual de
  solo lectura de ISSUE-020 (WebView + `set_visual_html`). 10 tests nuevos en
  `minmark-parser` y 7 en UI (plan visual + lógica §8.3; se elimina el test de
  wiring del `Stack` retirado con su código).

- `core/` + `ui/`: Quick Open (`ISSUE-029`, §41/§45/§39): paleta `Ctrl+P` con
  matcher **fuzzy propio std-only** (`minmark-fs::fuzzy`) y ranking documentado
  (score fuzzy con bonus de palabra/segmento/camel + bonus de nombre de
  archivo + recencia MRU − profundidad, desempate estable por índice) sobre el
  índice persistente §45. Modos en la misma paleta: `:N` salta a la línea N
  del archivo activo/elegido, `@heading` lista los headings del documento
  actual y `#heading` fuzzy sobre los headings de todo el proyecto (la línea
  del heading se resuelve al abrir, nunca por pulsación §47.5; techo de 100
  resultados). Selección con ↑/↓/Enter (doble click y Esc) que abre la pestaña
  vía `OpenTarget` y posiciona cursor/vista §8.6; `Show All Files` §14.4
  enlazado al explorador. 22 tests nuevos (255 core + 67 UI) y verificación
  visual con la app real (fuzzy, MRU en vivo, los tres modos con saltos
  posicionados y smokes de `Ctrl+F`/`Ctrl+Shift+F`).

- `core/` + `ui/`: búsqueda en el proyecto con reemplazo masivo deshacible
  (`ISSUE-028`, §40.2/§14.6/§39): `minmark-fs::search` con scope `.md`
  ampliable (o todos), exclusión `.gitignore` §14.6, resultados con contexto
  de línea por lotes en un hilo aparte con cancelación cooperativa
  (§47.1/§47.5), ficheros no-UTF8 saltados con nota (§12.5) y `MassReplace`
  (reemplazo individual / por fichero / global con expansión `$1..$9`,
  escritura atómica §48, all-or-nothing con rollback y **undo masivo de todos
  los ficheros en una sola acción**, §40.2). Panel lateral estilo VS Code
  (`ui/src/search`, `Ctrl+Shift+F`) con resultados agrupados por fichero,
  contexto, expandir/contraer, exclusión individual de coincidencias,
  Match Case / Whole Word / Regex y «Deshacer reemplazo»; búsqueda
  incremental sin bloquear UI ni escritura. Sincronización de pestañas tras
  reemplazos (buffers limpios recargados, modificados solo marcados §12.7)
  con `TabManager::reload_from_disk`. 22 tests nuevos (255 core + 63 UI) y
  verificación visual con la app real (alcance, gitignore, exclusiones,
  grupos `$2-$1`, undo masivo en disco y abort §12.7 sin tocar ficheros).

- `ui/`: barra vertical gris/negra en medio del split §9.1 (a petición del
  usuario): separador `mm-split` de 4 px entre editor y Preview, gris
  `#767676` en tema light y negro `#050505` en dark (generado desde la
  paleta resuelta), con el handle de redimensionado intacto.

- `core/` + `ui/`: búsqueda y reemplazo en el archivo actual (`ISSUE-027`,
  §40.1/§39): motor regex **propio std-only** en `minmark-document` (NFA
  Thompson con capturas, sin backtracking; subconjunto documentado; regex
  inválida o demasiado compleja → `SearchError`, sin panics) y `Matcher` con
  case sensitive / whole word / regex y navegación siguiente/anterior **con
  wrap** sobre posiciones semánticas §8.6. Barra de búsqueda/reemplazo
  (`Ctrl+F` / `Ctrl+H`, Edit ▸ Find/Replace) con marcado discreto de todas
  las coincidencias y resaltado de la activa, contador `n/total`, toggles
  Aa/palabra/`.*`, y `Replace`/`Replace All` integrados en el undo/redo
  único §8.5 (`Editor::replace_all` = una sola transacción: un `Ctrl+Z`
  revierte el reemplazo masivo completo). Regex inválida → error no
  bloqueante que conserva la última búsqueda válida. 255 tests core + 53 UI
  y verificación visual con la app real (wrap, toggles, replace-all + undo,
  regex inválida).

- `core/` + `ui/`: watching de proyecto (`ISSUE-026`, §46/§12.7):
  `Watcher::watch_tree` vigila el subárbol completo (std-only, sin rescan
  continuo) con `ChangeKind::Created` y auto-registro de los directorios que
  aparecen; bucle de eventos en la UI (cola de inotify coalescida cada
  250 ms) que refresca el árbol (ISSUE-022) y el índice (ISSUE-025) y marca
  las pestañas afectadas con `⚠` en el título (§3.7) **sin recargar el
  buffer** (§12.7). Los cambios de nuestras propias operaciones §14.8 se
  silencian (`SelfOps`).

- `core/` + `ui/`: índice persistente incremental del proyecto (`ISSUE-025`,
  §45): `ProjectIndex` con formato propio std-only `minmark-index 1`
  (decisión §4.4: sin dependencias), entries por archivo con stamp
  (mtime + tamaño), headings §27.1 y referencias §16; `refresh()` incremental
  (solo lo con stamp cambiado; borrados purgados) y completamente
  reconstruible; persistencia **fuera del proyecto** §45.1 en
  `$XDG_DATA_HOME/minmark/index/` (escritura atómica §48, validación de raíz);
  `.gitignore` excluido §14.6; construcción en un hilo aparte al abrir la
  ventana de proyecto (§47.1/§47.5). Base para búsqueda (ISSUE-028) y Quick
  Open (ISSUE-029).

- `core/` + `ui/`: Import Markdown (`ISSUE-024`, §15): copia `.md` externos
  dentro del proyecto (el original queda intacto) desde la pestaña standalone
  activa o el picker, con destino elegido en el explorador, un prompt
  relativo o un diálogo de carpeta (validación §48); colisión de nombres →
  renombrado §18.3, nunca sobrescritura (§3.7). La pestaña del externo se
  re-objetiva a la copia y el árbol se refresca (el archivo pasa a pertenecer
  al proyecto). File ▸ `Import Markdown` y menú contextual del explorador.

- `core/` + `ui/`: operaciones de archivo y rename/movimiento inteligente
  (`ISSUE-023`, §14.8/§14.9/§16): menú contextual del explorador (New
  Markdown, New Folder, `Rename`, Move, Duplicate, Delete, Copy Relative
  Path §17), `Delete` a la **papelera del sistema** (§14.9; implementación
  propia XDG trash std-only §4.1 — decisión §4.4 — con `Trash/info` y
  respaldo copia-borra) y rename/movimiento con actualización **all-or-nothing**
  de referencias conocidas (enlaces/imágenes Markdown, definiciones, wiki
  links activos §19.3, rutas `.bib`/`.csl` en front matter §21, composiciones
  y `.mdproject.toml` §16): validación previa sin efectos, rollback real ante
  fallo y formas de destino sin ambigüedad o abort (§16). Archivos ignorados
  por `.gitignore` fuera del análisis (§14.6, matcher propio mínimo). Las
  pestañas afectadas se re-objetivan; las borradas se cierran o conservan
  como «Sin título N» (§3.7). 33 tests nuevos (core + ui) y verificación
  visual con enlaces reescritos y papelera real.

- `ui/`: explorador de archivos (`ISSUE-022`, §14.3–14.7) en las ventanas de
  proyecto: árbol perezoso por directorio (`read_dir_entries` en un hilo
  aparte, integrado con `glib::idle_add_once`; §47.1/§47.5 — la UI jamás hace
  E/S y nada se precarga), tipos documentales por defecto (Markdown,
  imágenes §18.1, `.bib`, `.csl`, `.toml`) con `Show All Files` (§14.4) y
  ocultos ocultos con `Show Hidden Files` (§14.5), ignorados por `.gitignore`
  visibles (§14.6), symlinks visibles sin recorrer carpetas enlazadas
  (§14.7). Panel ocultable con `Ctrl+B` (§39, View → Toggle Explorer),
  ancho redimensionable y estado (visibilidad/ancho) restaurable entre
  sesiones (`~/.local/state/minmark/ui.ini`, fuera del proyecto §14.2).
  Activar un archivo lo abre como pestaña vía `OpenTarget` (ISSUE-021).
  15 tests nuevos en `ui/` y verificación visual de la app real bajo Xvfb
  (expansión, apertura de pestañas, `Ctrl+B`, filtros y restauración).

- `docs/issues/`: planificación de la Fase 2 (Project Mode) con `ISSUE-022`–
  `ISSUE-029`: explorador §14.3–14.7, operaciones de archivo +
  rename/movimiento inteligente §14.8–14.9/§16, Import Markdown §15, índice
  persistente §45, filesystem watching de proyecto §46, búsqueda
  archivo/proyecto §40 y Quick Open §41. Open Folder (021) y pestañas §37
  (020) ya cubiertos.

- `ui/`: multi-ventana estilo Notepad++ (`ISSUE-021`): ventanas standalone
  (archivos nuevos o existentes fuera de carpeta) y ventanas de proyecto, una
  por carpeta (`Open Folder` → ventana nueva; rutas validadas contra la raíz
  §48). Abrir un archivo fuera del ámbito de una ventana lo lleva a una
  ventana standalone nueva (nunca se rechaza); `New Window` (Ctrl+Shift+N) y
  `Open Folder` (Ctrl+Shift+O) en File; título de ventana observable (§3.7).
  CLI: carpetas → ventanas de proyecto, archivos → ventana standalone con sus
  pestañas. 27 tests en `ui/` y verificación visual de las dos ventanas.

- `ui/`: aplicación GTK4 + WebKitGTK 6.0 (`ISSUE-020`, ADR-001): crate
  `minmark-ui` (binario `minmark`) que integra el core — menús §36 (File,
  Edit, View, Help) con atajos §39, editor Markdown Mode (TextView
  monoespaciado con highlighting de `minmark-parser` puenteado al `Editor`
  del core, buffer único §6.1, posiciones semánticas §8.6) y vista Visual §8.2
  del mismo buffer (lectura; edición Visual = Fase 3), Preview WebView
  continuo (§10.2) con temas (§11), pestañas §37 con indicador ●, diálogos
  GTK4 y confirmaciones explícitas (§3.7; flujo de sobrescritura con
  confirmación ante cambios externos §12.7 vía `save_confirmed`). 24 tests de
  modelo en `ui/` y verificación visual de la app real (Split, Visual,
  pestañas, Preview) bajo Xvfb.

- `core/`: filesystem safety (`ISSUE-019`): guardado atómico en `minmark-fs`
  (temporal en el mismo directorio + rename con `sync_all`, permisos
  preservados; crash simulado con proceso real: el original queda intacto),
  `LocalFs::save_confirmed` para sobrescribir cambios externos solo tras
  confirmación explícita del usuario (§3.7, §12.7), watcher nativo
  `InotifyWatcher` (inotify vía FFI std-only, §46) que notifica
  `Modified`/`Deleted` sin recargar nunca el buffer, validación de rutas en
  modo proyecto (`resolve_in_project`, §48: path traversal y symlinks que
  resuelven fuera del root rechazados), árbol con symlinks visibles sin
  recursión ni ciclos (§14.7) y `SnapshotStore` fuera del proyecto (§3.6,
  §45.1) con límite §13.5 y gestión §13.6–13.7. Sin dependencias nuevas
  (§4.1). Workspace: 158 tests verdes.

- `core/`: exportación PDF con Typst (`ISSUE-018`): crate `minmark-math`
  (traductor LaTeX→Typst del subconjunto ADR-002, std-only, con marcador
  `⚠ [math no soportado: …]` en Preview y PDF — NFR-026) y `minmark-pdf`
  (`TypstEngine` implementa `PdfEngine`: AST → código Typst → PDF, config
  A4/25mm/Portrait §58.2, fuentes embebidas `typst-assets`). NFR-030
  verificado (texto seleccionable, `/ToUnicode`). Dependencia estratégica
  única: `typst` (§4.3). Workspace: 128 tests verdes.
- `core/`: sistema de temas (`ISSUE-017`): paleta única `Palette` Light/Dark
  (§23 completa) que genera el CSS del Preview y alimenta la UI GTK4,
  detección del tema del sistema (XDG/gsettings, GTK env, KDE `kdeglobals`)
  y `resolve_theme` (System con fallback Light). Workspace: 111 tests verdes.
- `core/minmark-renderer`: Preview rendering (`ISSUE-016`): página completa
  con CSS de temas Light/Dark/System (§11), ancho máximo configurable (§10.2,
  defecto 800 px), `data-mm-span` + `ScrollMap` para scroll sync
  bidireccional por posición semántica y doble clic (§9.1). Workspace: 104
  tests verdes.
- `core/`: Markdown Mode editor (`ISSUE-015`): `Editor` en `minmark-document`
  con operaciones insert/delete/replace, cursor y selección como posición
  semántica línea/columna (§8.6), Undo/Redo único compartido entre modos
  (§8.5) e índice de líneas incremental; highlighting CodeMirror-style en
  `minmark-parser::highlight()` derivado de la misma gramática (§4.2).
  NFR-005 verificado (<16 ms por interacción sobre 20 MB). Workspace: 103
  tests verdes.
- `core/minmark-parser`: dialecto Markdown §7 completo (`ISSUE-014`):
  extensiones §7.3 (callouts `:::`, colores, math LaTeX, citas Pandoc,
  cross-refs `{{ref:}}`, TOC, pagebreaks, atributos de imagen, wiki links) y
  gramática §7.4 (vocabulario cerrado, posiciones de atributos, escapes).
  Corpus positivo/negativo (`tests/dialecto.rs`) y benchmark vs baseline
  ISSUE-010 (54 MB → 2.73 s; 270 MB → 25.4 s). Workspace: 89 tests verdes.
- `core/`: file I/O en `minmark-fs` (`ISSUE-013`): `LocalFs` implementa
  `FileStore` (create/open/save/save_as) con UTF-8 estricto (§12.5),
  preservación byte a byte de LF/CRLF (§12.6) y detección de cambios externos
  con `FileStamp` (§12.7: sin sobrescrituras silenciosas). Test de integración
  abrir → parsear → renderizar. Workspace: 47 tests verdes.
- `core/`: workspace Cargo del núcleo en Rust (`ISSUE-012`): `minmark-document`
  (buffer §6.1, AST compartida, LF/CRLF §12.6), `minmark-config` (ajustes §58.2
  y cadena de precedencia §58.1), `minmark-parser` (pulldown-cmark → AST),
  `minmark-renderer` (AST → HTML con escape §7.2), `minmark-pdf` (frontera
  Typst, geometría §32) y `minmark-fs` (file I/O, snapshots §13). 38 tests;
  única dependencia de producción: `pulldown-cmark 0.12` (§4.5, ISSUE-010).
  Arquitectura en `core/README.md`.
- `prototypes/`: código de prototipos Fase 0.
  - `ui-rust-native/`: compilado (iced v0.12.1, 364 crates), métricas pendientes de display.
  - `parser/`: benchmark ejecutado (51.58MB→0.926s, 100MB→1.885s).
  - `ui-webview/`: bloqueado por dependencias GTK4 del sistema.
- `docs/benchmarks/resultados-parser-2026-09-28.csv`: métricas parser.
- `docs/benchmarks/corpus-{20,100}MB.md`: corpus de prueba (semilla 42).
- `docs/benchmarks/resultados-ui-comparativa-2026-09-28.csv`: benchmarks comparativos UI.
- `docs/adr/001-tecnologia-ui.md`: decisión tecnología UI (webview vs iced).

### Arreglado

- `ui/`: crash al arrancar en sistemas sin user namespaces sin privilegios
  (`Failed to fully launch dbus-proxy` + `bwrap: setting up uid map: Permission
  denied` → SIGTRAP). El sandbox de bubblewrap de WebKit no puede funcionar en
  esos kernels; minmark lo sondea al arrancar y lo desactiva explícitamente
  (la alternativa era un crash en toda ventana con Preview; minmark es local
  y sin red, §3.4). Registrado en «Problemas conocidos» del README.

- `ui/`: el enlace tardío `CmdSlot` guardaba un `Weak` inválido (el `Rc`
  fuerte moría al terminar `build_window`) y los comandos con diálogo no
  hacían nada (`Open`, `Open Folder`, `Save As`, confirmaciones de cierre y
  sobrescritura §3.7/§12.7, click en pestañas). La celda la posee ahora
  `AppCmds` y guarda un `Weak<AppCmds>`, sin ciclos de `Rc`.

### Completado

- **Fase 2 completada** ✅ (2026-10-02): Project Mode — Open Folder
  multi-ventana (ISSUE-021), explorador §14.3–14.7 (022), operaciones y
  rename/movimiento inteligente §14.8/§16 (023), Import Markdown §15 (024),
  índice persistente §45 (025), filesystem watching §46 (026), búsqueda y
  reemplazo §40.1 (027), búsqueda de proyecto con reemplazo masivo §40.2
  (028) y Quick Open §41 (029). Issues cerradas: ISSUE-021 … ISSUE-029 (9/9).

- **Fase 0 completada** ✅ (2026-09-28): benchmark tecnológico UI y parser, selección por datos.
  - UI elegida: GTK4 + WebKit6 (webview) — ADR-001
  - Parser elegido: pulldown-cmark (ISSUE-006)
  - Issues cerradas: ISSUE-001, 006, 007, 008, 009, 010, 011 (7/7)

- Repositorio inicial: `README.md`, `CHANGELOG.md`, `.gitignore`.
- Licencia inicial: PolyForm Noncommercial License 1.0.0 (source-available,
  sin uso comercial). Sustituida posteriormente por Apache-2.0; véase la
  [licencia vigente](../LICENSE).
- `docs/custom_markdown_app_SRS_v1.0.md`: plan / especificación de producto y software (renombrada a v1.1, ver *Cambiado*).
- Tracking de issues local en `docs/issues/` (log maestro, plantilla y carpetas
  `open/`, `ongoing/`, `done/`).
- Issues iniciales abiertas a partir de la revisión de la SRS
  (`ISSUE-001` … `ISSUE-006`).
- Organización por milestones (= fases de la SRS §55) en el tracking de issues.
- Issues de la Fase 0 (`ISSUE-007` … `ISSUE-011`): protocolo de benchmark,
  microprototipos UI WebView y Rust-native, benchmark del parser y decisión de
  tecnología UI.
- `docs/benchmarks/PROTOCOLO.md`: máquina de referencia (Ryzen 5 8400F, 30 GiB,
  NVMe, Ubuntu 24.04.5) y protocolo de medición de la Fase 0 (cierre de
  `ISSUE-007`).

### Cambiado

- SRS v1.0 → v1.1 → v1.2 → v1.3 (`docs/custom_markdown_app_SRS_v1.3.md`):
  §7.4 gramática de extensiones (`ISSUE-001`), §23 colores con `:`, §58 cadena
  única de precedencia y tabla centralizada de defaults (`ISSUE-002`), §13.5–13.7
  gestión de snapshots al alcanzar límite (`ISSUE-003`). Añadidos FR-081 a
  FR-088.
