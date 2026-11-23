# Sincronización y desplazamiento del editor

## Entradas y contratos

- [`ui/src/editor/pane.rs`](../../../ui/src/editor/pane.rs): `EditorPane` posee
  el `TextBuffer`, el `TextView` y su `ScrolledWindow`. Las ediciones de GTK se
  propagan al modelo mediante `EditSink`; `load` sincroniza el camino inverso.
- [`ui/src/main.rs`](../../../ui/src/main.rs): `AppCmds::reload_active` pasa el
  editor, el modo y la posición de desplazamiento de la pestaña activa a `load`.
  Lo usan guardado, deshacer/rehacer, formato, reemplazos, apertura, cierre,
  cambio de pestaña/modo y recarga desde disco.
- [`ui/src/tabs/manager.rs`](../../../ui/src/tabs/manager.rs): cada `Tab` tiene
  un `scroll_position: Rc<Cell<f64>>` en píxeles verticales. Empieza en cero y
  acompaña a la pestaña aunque cambie su ruta o su índice. Es estado de la
  sesión; no se persiste en disco.

`load` guarda la posición del documento saliente y prepara la del entrante.
Solo reescribe el buffer cuando cambia el contenido; la comparación incluye
la sintaxis invisible de Visual. La guardia existente evita que una carga
programática altere el historial o emita ediciones al modelo. Cursor y selección
se restauran desde `Editor`, independientemente del desplazamiento.

GTK recalcula las alturas de texto de forma diferida. La posición se aplica
en un callback idle posterior: restaurarla inmediatamente después de `set_text`
permite que GTK vuelva a llevarla al inicio. Varias cargas consecutivas usan la
última posición pendiente. GTK limita el valor al espacio disponible si el
documento se acorta.

## Navegación explícita

`focus_find_match` y `scroll_to_cursor` cancelan la restauración pendiente.
Usan `scroll_to_mark` para esperar a la validación de líneas de GTK. Así,
búsquedas, enlaces, outline y resultados de búsqueda del proyecto pueden
mostrar el destino solicitado sin que un callback de una recarga lo desplace.
Los repintados de resaltado y configuración no reescriben el texto.

## Seguimiento del editor en Split

`EditorPane::connect_scroll_changed` agrupa en un idle los cambios del
desplazamiento, del tamaño del viewport y del mapeo del editor. Evita notificar
una posición intermedia mientras `load` está restaurando el desplazamiento.
`preview_position` devuelve una posición semántica (línea y columna Unicode):
el centro visible, o el cursor si se solicita y está dentro de la vista. Incluye
el ajuste de líneas de GTK y la sintaxis invisible del modo Visual.

En `ui/src/main.rs`, `request_preview_sync` solo actúa en Split. El seguimiento
de una edición o de un cambio de selección mantiene visible el cursor; el
desplazamiento y el regreso a Split centran la zona visible y comprueban también
que el cursor visible no quede fuera del Preview por la diferencia de alturas.
El editor conserva su desplazamiento y su selección.

Si hay un refresco diferido o un render en curso, `preview_follow` conserva la
petición. `flush_preview_sync` la aplica después de `PreviewPane::update_html`
en la entrega del render vigente, usando la posición actual del editor. Así no
se intenta seguir texto nuevo sobre el DOM anterior. Un seguimiento de edición
pendiente tiene prioridad sobre las notificaciones de geometría de ese refresco
para evitar recentrar con cada tecla. En Editor o Preview no se impone el
desplazamiento de la otra vista.

[`ui/src/preview/mod.rs`](../../../ui/src/preview/mod.rs) convierte bytes del
fuente en elementos `data-mm-span`. Los elementos pequeños se centran o se
mantienen visibles; dentro de un bloque más alto que la ventana se reutiliza
la interpolación por bytes de su span. Esta posición interna es aproximada:
no existe un mapa DOM por línea. Se conserva el transporte agnóstico del motor
y la cola de scroll durante una carga del WebView.

## Comprobación

La prueba `gtk_integration_tests::gtk_editor_operations_preserve_scroll` en
`ui/src/main.rs` usa una ventana GTK real y un documento largo. Comprueba
guardado con cursor fuera de pantalla, formato, deshacer/rehacer con y sin
historial, ambos modos, sintaxis Visual, repintado, recargas consecutivas,
posiciones independientes por pestaña, apertura/cierre, recarga externa,
Save As y navegación explícita al principio y a una línea posterior.
Desactiva las animaciones y espera frames para medir posiciones estables.

`gtk_integration_tests::gtk_split_preview_follows_editor_viewport_and_new_text`
verifica en una ventana real el desplazamiento sin mover el cursor, Markdown y
Visual, texto Unicode, una inserción de treinta párrafos al final, el regreso
a Split y la navegación interna en un bloque de código largo. La regresión
`gtk_navigation_outline_completion_and_dirty_buffers` mantiene sincronizado
el espejo GTK antes de comprobar una edición programática visible.

```sh
cargo test --locked --manifest-path ui/Cargo.toml --bin minmark
xvfb-run -a cargo test --locked --manifest-path ui/Cargo.toml --bin minmark gtk_integration_tests::gtk_editor_operations_preserve_scroll -- --ignored --exact
xvfb-run -a cargo test --locked --manifest-path ui/Cargo.toml --bin minmark gtk_integration_tests::gtk_split_preview_follows_editor_viewport_and_new_text -- --ignored --exact
```

En Linux se necesita GTK ≥4.10, WebKitGTK 6.0 y una sesión gráfica o Xvfb.
Véanse los requisitos por plataforma en [BUILDING](../../../BUILDING.md).
