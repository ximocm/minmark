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
