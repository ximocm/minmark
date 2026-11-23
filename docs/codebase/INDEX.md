# Índice del código documentado

Cobertura parcial: sincronización del editor GTK con el modelo, conservación
del desplazamiento y seguimiento Editor → Preview en Split. El resto de la UI
y del core no se documenta en este índice.
La [documentación general](../README.md) y [BUILDING](../../BUILDING.md)
conservan la arquitectura y los comandos de desarrollo existentes.

| Código | Documento |
| --- | --- |
| `ui/src/editor/pane.rs` — `EditorPane::load`, navegación y desplazamiento | [Editor](modules/editor.md) |
| `ui/src/tabs/manager.rs` — `Tab::scroll_position` | [Editor](modules/editor.md) |
| `ui/src/main.rs` — `AppCmds::reload_active`, navegación y regresión GTK | [Editor](modules/editor.md) |
| `ui/src/preview/mod.rs` — seguimiento de anclas y scroll dentro de bloques | [Editor](modules/editor.md) |
