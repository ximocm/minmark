# Registro de QA — fase 5

Fecha: 2026-10-14. Ejecutor: Codex. Alcance: [suite ACA-01–10](README.md),
[fase 5 #13](https://github.com/ximocm/minmark/issues/13), tareas #14–#20.
Código: implementación sobre `12c7a0f`, cierre sobre `3ea56a0` (fase 4 integrada).
Las ejecuciones registradas preceden a la publicación; el seguimiento final
se mantiene en el [plan #13](https://github.com/ximocm/minmark/issues/13).

## Resultado

**349 pruebas del core y 93 de UI pasan.** La integración GTK/WebKit pasa
ejecutada expresamente tanto en X11 como en Wayland; normalmente está ignorada
en la suite sin pantalla. Build correcto. Clippy de los paquetes modificados
pasa con warnings. La campaña conserva dos incidencias: lectura de Preview con
Orca sin verificar y glifos CJK ausentes en paginado/PDF.
También pasan las seis pruebas de política y contenido de paquetes de release.

Se combinaron pruebas ejecutables, interacción nativa con XTest/AT-SPI y revisión
visual de capturas/PDF. Los casos cubiertos por pruebas no se presentan como
recorridos manuales completos. Solo se editaron copias temporales de las fixtures
y perfiles de configuración aislados; no se modificaron documentos reales.

## Entorno

Linux x86_64; Rust 1.98.1; GTK 4.14.5; WebKitGTK 2.52.6. X11 sobre Xvfb.
Wayland: Weston 13.0.0, renderer Pixman, anidado sobre Xvfb, cliente GTK Wayland.
Lector: Orca 46.1, AT-SPI y Speech Dispatcher dedicados en un bus D-Bus privado.
Los recursos de Weston del entorno de fase 4 se reutilizaron sin alterar esa QA.
No equivale a una sesión nativa de GNOME/KDE ni a pruebas Windows/macOS.

## Ejecución por caso

| Caso | Estado | Evidencia y alcance comprobado |
| --- | --- | --- |
| ACA-01 | **Pasado** | Traductor y compilación Typst: fórmulas del subconjunto, grupos con espacios, fracciones compuestas, scripts y errores visibles. GTK comprueba cinco SVG sin recortes, IDs únicos y colores claro/oscuro. PDF y capturas inspeccionados. |
| ACA-02 | **Pasado** | Dos BibTeX, IEEE y CSL autor/año; citas simples/múltiples/locators y formato de bibliografía. GTK comprueba el cambio externo de `.bib` sin dirty. PDF IEEE y alternativo inspeccionados. |
| ACA-03 | **Pasado** | Pruebas de archivos ausentes/malformados, CSL inválido con fallback, claves ausentes/duplicadas, límites de rutas y recuperación tras crear un recurso. Diagnósticos comprobados mediante las funciones usadas por UI/Preview/PDF. |
| ACA-04 | **Pasado** | Búsqueda por clave/autor/título/año; Unicode, CRLF, locator y exclusión de código/escapes. GTK verifica aceptación, Escape y un undo/redo en X11/Wayland. XTest acepta con Tab y verifica el texto tras deshacer en X11; en Wayland se comprueba el anuncio de la sugerencia con Orca. |
| ACA-05 | **Pasado** | Numeración por primera aparición, referencias repetidas y errores de notas. GTK pulsa enlace y vuelta. PDF de dos páginas sitúa notas al pie; nota larga de 220 repeticiones conserva todo el contenido al dividirse. |
| ACA-06 | **Pasado** | Pruebas de numeración continua, por capítulo y reinicio simple, heading numbering y referencias compartidas. Captions de tabla/ecuaciones y tabla de dos columnas inspeccionados en Preview/PDF. |
| ACA-07 | **Pasado** | Token real frente a código/escapes, profundidades y ausencia de headings. GTK verifica cuatro enlaces. PDF inspeccionado: TOC con páginas 1/2 y H4 excluido a profundidad 3. |
| ACA-08 | **Pasado** | GTK verifica cambios de modo, buffer sin guardar, historial y fuente en disco intacta; exportación del buffer vigente. Pruebas de recuperación de recursos y errores matemáticos. |
| ACA-09 | **Parcial** | Integración académica pasa en Wayland. Orca anuncia `Sugerencia: smith2025 · Alice Smith · Energy Research · 2025`. Editor y popup comprobados. Al enfocar Preview, Orca anuncia `panel`; el árbol AT-SPI de esta sesión no expone su contenido. Lectura de notas, fórmulas y diagnósticos de Preview **sin verificar**. |
| ACA-10 | **Fallido (P2)** | `東京` se conserva en fuente y Preview continuo; las fuentes embebidas producen glifos ausentes en paginado/PDF. Visible en `academic-page-1.png`. |

## Correcciones encontradas durante QA

- Matemáticas: fondo negro, recorte de ecuaciones y colisiones de IDs SVG;
  corregidos y repetidos en GTK y PDF. Se preserva el color en tema oscuro.
- Fracciones, espacios en grupos y números tras scripts: traducción corregida
  y validada con composición real, además de las expectativas del traductor.
- Tabla PDF: el número de columnas se calculaba a partir de los inlines de una
  celda; ahora usa las columnas del encabezado/alineación.
- Exportación: actualiza ajustes del front matter antes de capturar el buffer.
- Referencias a notas: no se interpretan como enlaces rotos del proyecto.

La corrección de accesibilidad del popup procedente del cierre de fase 4 se
conserva y está incluida en las validaciones de este árbol de trabajo.

## Evidencia

En [evidence/](evidence/): logs de core/UI/Clippy/build, integración X11/Wayland,
capturas continua/paginada y páginas de PDF, registro de teclado X11 y extracto
de voz de Orca. Los PDF generados están en esa carpeta local; el repositorio
ignora `*.pdf`. Las fixtures reproducibles están en [fixtures/](fixtures/).

Los PDF `academic.pdf`, `academic-author-year.pdf` y `long-notes.pdf` se abrieron
mediante renderizado Poppler. Se contrastó el texto extraído, las notas, el TOC,
las fórmulas y la bibliografía; las capturas completas se inspeccionaron.

## Comandos reproducibles

Desde la raíz; `QA_TARGET` puede apuntar a otro directorio de compilación.
No usar variables de perfil de un usuario real para la integración gráfica.

```sh
QA_TARGET=/tmp/minmark-phase5-target
QA_OUTPUT=/tmp/minmark-phase5-qa/evidence
mkdir -p "$QA_OUTPUT"
MINMARK_QA_OUTPUT_DIR="$QA_OUTPUT" cargo test --locked --offline \
  --manifest-path core/Cargo.toml --workspace --target-dir "$QA_TARGET" -j 2
cargo test --locked --offline --manifest-path ui/Cargo.toml \
  --target-dir "$QA_TARGET" -j 2
env XDG_CONFIG_HOME=/tmp/minmark-phase5-qa/config \
  XDG_DATA_HOME=/tmp/minmark-phase5-qa/data \
  XDG_STATE_HOME=/tmp/minmark-phase5-qa/state \
  MINMARK_QA_OUTPUT_DIR="$QA_OUTPUT" GDK_BACKEND=x11 GSK_RENDERER=cairo \
  xvfb-run -a cargo test --locked --offline --manifest-path ui/Cargo.toml \
  --target-dir "$QA_TARGET" -j 2 \
  gtk_navigation_outline_completion_and_dirty_buffers -- --ignored --test-threads=1
cargo clippy --locked --offline --manifest-path core/Cargo.toml \
  -p minmark-config -p minmark-document -p minmark-math -p minmark-parser \
  -p minmark-renderer -p minmark-pdf --all-targets --no-deps \
  --target-dir "$QA_TARGET" -j 2
cargo clippy --locked --offline --manifest-path ui/Cargo.toml \
  --all-targets --no-deps --target-dir "$QA_TARGET" -j 2
cargo build --locked --offline --manifest-path ui/Cargo.toml \
  --target-dir "$QA_TARGET" -j 2
python3 scripts/test_release.py
git diff --check
```

Sin caché de dependencias, ejecutar primero `cargo fetch --locked` para ambos
manifests. La repetición Wayland usó el mismo comando de integración, dentro del
compositor privado con `GDK_BACKEND=wayland`; el driver local y sus logs se
conservan en `/tmp/minmark-phase5-accessible`.

Clippy de todo el workspace también se intentó: falla por el error preexistente
`clippy::never_loop` en `core/minmark-fs/src/index.rs:210`. No se modificó ese
código ajeno a fase 5. Log: `evidence/workspace-clippy-initial.log`.

## Pendientes de cierre

La exposición del contenido de Preview a Orca se sigue en
[#39](https://github.com/ximocm/minmark/issues/39). La cobertura tipográfica CJK
se sigue en [#40](https://github.com/ximocm/minmark/issues/40), relacionada con
tipografía de fase 6. No se declara la campaña íntegramente verde; el cierre
funcional de las tareas #14–#20 conserva ambas incidencias separadas.

## Decisión de alcance — 2026-10-14

El propietario pospone CJK: de momento no necesita esos caracteres. ACA-10
conserva el fallo histórico, pero deja de ser requisito o bloqueo actual.
[#40](https://github.com/ximocm/minmark/issues/40) es una mejora futura
independiente, sin milestone. No se cambia el comportamiento tipográfico.

## Repetición de ACA-09 — investigación de #39, 2026-10-14

**Resultado: no resuelto; se mantiene Parcial.** Controles mínimos Python y
Rust con las mismas bibliotecas exponen un documento web, headings, fórmulas
y notas. Se contrastaron HTML académico, contenedor, carga sucesiva e identidad
de aplicación. Desactivar el sandbox solo en una instancia de QA no resolvió
el fallo; no se cambia la política del producto.

El código de PreviewPane en un control mínimo deja de exponer el documento
cuando no se pide foco explícito a WebKit. Pedirlo al presentar el control
restaura su árbol. Las variantes probadas en Minmark con foco al entrar/pulsar
Preview y al cambiar de modo no han acreditado la solución: el árbol sigue sin
exponer el documento. Orca registra referencias a una raíz AT-SPI inexistente.
No se ha establecido todavía la causa exacta ni se atribuye al motor.

Los parches de foco no verificados se retiraron. Editor, documentos y servicios
del usuario se conservaron; las instancias privadas se cerraron. Evidencia:
`evidence/orca39-before-tree.json`, `orca39-control-tree.json`,
`orca39-repeat-tree.json` y `orca39-repeat-summary.log`. Los controles y los
logs completos se conservan en `/tmp/minmark-orca39`. La issue #39 sigue abierta.

# Registro de QA — fase 6

Fecha: 2026-10-16 (smoke) y 2026-10-19 (verificación). Alcance:
[plan #21](https://github.com/ximocm/minmark/issues/21), tareas
[#22](https://github.com/ximocm/minmark/issues/22)–[#26](https://github.com/ximocm/minmark/issues/26)
y la revisión del PR #44 (hallazgos B1/B2/S1). Código: PRs #42–#44 integrados
(`e036abb`); la verificación cubre esa base más las correcciones de la revisión.

## Resultado

**391 pruebas del core y 115 de UI pasan.** Las 3 pruebas de integración
GTK/WebKit se ejecutan aparte — una por proceso: `gtk4::init()` solo admite
una inicialización por proceso y libtest ejecuta cada test en su propio hilo —
y pasan las tres. Clippy de los paquetes del core y de UI pasa con warnings
preexistentes, ninguno introducido por la fase. `git diff --check` limpio y las
seis pruebas de política de release pasan. Los casos cubiertos por pruebas no
se presentan como recorridos manuales completos; solo se usaron copias
temporales y perfiles aislados.

## Smoke de la interfaz de composiciones — 2026-10-16

Sesión gráfica aislada (Xvfb/X11, perfil de configuración temporal) sobre un
proyecto de prueba con la composición «Libro» (portada + capítulo).

| Paso | Estado | Evidencia |
| --- | --- | --- |
| Ventana principal del proyecto | Pasado | `evidence/phase6-00-ventana-principal.png` |
| Menú Project con las acciones de composición | Pasado | `evidence/phase6-01-menu-project-abierto.png` |
| Menú File con Export Composition PDF | Pasado | `evidence/phase6-02-menu-file-abierto.png` |
| Preview combinado con los archivos en orden | Pasado | `evidence/phase6-03-preview-combinado.png`; informe `orden_texto` |
| Diálogo de exportación con nombre propuesto | Pasado | `evidence/phase6-04-dialogo-export-composition.png`, `phase6-04b-entry-rellenado.png` |
| Exporta un único PDF (2 páginas, cabeceras y marcadores) | Pasado | `evidence/phase6-05-tras-aceptar-export.png`; informe `c` |
| Cancelar no escribe archivos ni toca el PDF previo | Pasado | `evidence/phase6-06-dialogo-antes-de-cancelar.png`, `phase6-07-tras-cancelar.png`; informe `d` |
| Reexportar pide confirmación y sobrescribe | Pasado | `evidence/phase6-08-dialogo-reexportar.png`, `phase6-09-confirmacion-sobrescritura.png`, `phase6-10-tras-sobrescribir.png`; informe `e` (md5 y mtime cambian) |

Detalle numérico en `evidence/phase6-smoke-informe.json` (fases `0-inicio` a
`e-sobrescribir`). El Preview combinado a Orca sigue el estado de la fase 5
(ACA-09/Parcial, issue #39): `preview_texto_atspi_len` es 0 también aquí.

## Correcciones de la revisión del PR #44 — 2026-10-19

- **B2 (§48)**: una entrada de `files` cuyo padre es un symlink que escapa y
  cuyo fichero final no existe se admitía como dentro del ámbito (la canónica
  no resolvía) y su buffer abierto entraba en el documento combinado.
  `fuera_del_ambito` reutiliza ahora `minmark_fs::resolve_in_project`
  (ancestro existente más cercano). Test unitario
  `files_bajo_symlink_escapado_sin_fichero_diagnostican_y_no_usan_el_buffer`.
- **Ciclo de referencias**: `AppCmds` retenía el grupo «win» con referencia
  fuerte y sus acciones capturan `Rc<AppCmds>`: una ventana cerrada no se
  liberaba nunca. Retención débil (`glib::WeakRef`); mismo arreglo en el
  callback de enlaces del Preview combinado.
- **Comentarios**: la caché de composiciones y la cadena §58 (`load_composition_at`)
  son lecturas próximas del `.mdproject.toml`, no una misma lectura atómica;
  el texto afirmaba lo segundo.
- **Tests B1**: el estado `enabled` se afirma acción a acción (las dos
  composicionales), en el grupo instalado de la ventana GTK y en el unitario.

## Evidencia

`evidence/phase6-smoke-informe.json`, capturas `evidence/phase6-*.png` (15),
suite de integración `gtk_integration_tests` de `ui/src/main.rs` y tests
unitarios de `ui/src/composition.rs`, `ui/src/menus/actions.rs` y
`ui/src/preview/mod.rs`.

## Comandos reproducibles

```sh
QA_TARGET=/tmp/minmark-qa-target
cargo test --locked --offline --manifest-path core/Cargo.toml --workspace \
  --target-dir "$QA_TARGET" -j 2
cargo test --locked --offline --manifest-path ui/Cargo.toml \
  --target-dir "$QA_TARGET" -j 2
# Integración GTK: una prueba por proceso (gtk4::init() es único por proceso).
for t in composiciones_se_redescubren_al_cambiar_mdproject_en_caliente \
         preview_combinado_navega_destinos_internos_con_scroll_in_page \
         gtk_navigation_outline_completion_and_dirty_buffers; do
  env XDG_CONFIG_HOME=/tmp/minmark-phase6-qa/config \
    XDG_DATA_HOME=/tmp/minmark-phase6-qa/data \
    XDG_STATE_HOME=/tmp/minmark-phase6-qa/state \
    GDK_BACKEND=x11 GSK_RENDERER=cairo \
    xvfb-run -a cargo test --locked --offline --manifest-path ui/Cargo.toml \
    --target-dir "$QA_TARGET" -j 2 "$t" -- --ignored --test-threads=1
done
cargo clippy --locked --offline --manifest-path core/Cargo.toml \
  -p minmark-config -p minmark-document -p minmark-math -p minmark-parser \
  -p minmark-renderer -p minmark-pdf --all-targets --no-deps \
  --target-dir "$QA_TARGET" -j 2
cargo clippy --locked --offline --manifest-path ui/Cargo.toml \
  --all-targets --no-deps --target-dir "$QA_TARGET" -j 2
git diff --check
python3 scripts/test_release.py
```

## Pendientes de cierre

- Cierre formal de #22–#26 en GitHub: criterios implementados y cubiertos por
  pruebas y por el smoke; queda la decisión del propietario.
- Siguiente tarea de la fase: [#27](https://github.com/ximocm/minmark/issues/27)
  (captions de figuras, tablas y ecuaciones).
- Los `wiki links` combinados emiten `minmark-wiki:#destino` y el manejador de
  destinos internos no navega ese caso: es previo a S1 y candidato a issue
  propia.
- #39 y #40 conservan las decisiones registradas en la fase 5.

## Captions y ajuste de tablas §35.2 — ISSUE-027, 2026-10-19

Criterios de captions (figura con `fig:` y alt como pie debajo, `{#tbl}` y
`{#eq}` debajo de su bloque, paridad de etiquetas y numeración entre Preview y
PDF, fuente y escaping conservados, imagen ordinaria intacta) estaban
implementados desde la fase 5/6 mediante la expansión compartida
`resolve_cross_references`; se acreditan ahora con pruebas nuevas del lado PDF:
`captions_de_tabla_y_ecuacion_debajo_con_su_etiqueta`,
`ecuacion_sin_caption_solo_etiqueta_e_imagen_ordinaria_intacta` y
`etiquetas_de_caption_coinciden_entre_preview_y_pdf`.

El criterio §35.2 no estaba implementado: `table_min_font_pt` se parseaba pero
nadie lo consumía y la tabla PDF salía con su tipografía natural (una palabra
de 120 caracteres medía 939 pt frente a 453 pt de cuerpo). Se implementó el
ajuste en el motor PDF (módulo `fit` de `minmark-pdf`), reutilizando el layout
existente:

- La tabla sigue siendo nativa y partible entre páginas; el caption va fuera
  del ajuste, debajo, con su tipografía propia.
- Una tabla que ya cabe no se toca (nunca se amplía, aunque el cuerpo esté por
  debajo del suelo).
- Si su ancho mínimo no cabe, la tipografía se reduce proporcionalmente hasta
  el suelo `table_min_font_pt` (8 pt por defecto), con inset/stroke
  proporcionales.
- Si ni el suelo cabe, la exportación se detiene nombrando la tabla (caption
  expandido, `#id` o nº de orden), con la superficie de error
  `error de composición PDF: panicked with: "<tabla>: no cabe en el ancho
  disponible con table_min_font_pt=8pt (§35.2)"`.

Ambigüedades del SRS interpretadas: «cabe» es ajuste horizontal, no a una
página; el suelo aplica a la tipografía base de la tabla; las tablas que ya
caben no se amplían. Verificación: 44 pruebas de `minmark-pdf` (12 nuevas: prosa
que se ajusta por quiebre sin reducir, reducción acotada al suelo, error que
nombra la tabla, consumo de `table_min_font_pt`, tabla larga con caption
repartida en varias páginas, alcance del tamaño reducido y geometría de
contenedor), 406 del core y 115 de UI en verde; clippy sin avisos nuevos y
`scripts/test_release.py` 6/6. La sonda Typst desechable usada para diseñar se
eliminó.
