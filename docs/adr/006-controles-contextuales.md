# ADR-006: Controles visuales contextuales y comandos compartidos

**Fecha:** 2026-10-27
**Estado:** Aceptada
**Contexto:** Fase 7 (SRS §44, §55; issue #28 «casos no definidos deberán
resolverse explícitamente antes de implementar»), slices #29–#31. Continúa
ADR-001 (GTK4) y el modo Visual de fase 3 sobre el buffer único §6.1.

## Contexto

§44 pide controles editoriales opcionales (H1–H6, bold, italic, strike, code,
links, images, tables, equations, citations, callouts) que «aparezcan solo
cuando aporten utilidad», sin toolbar permanente (§36) ni ribbon. Los casos no
definidos por #28 eran: la superficie de los controles, las vías de entrada de
las acciones y los atajos de formato (§39 fija `Ctrl+B` para Toggle explorer).

## Decisiones

1. **Superficie: popover flotante anclado a la selección/cursor** (patrón del
   popup de autocompletado), no barra transitoria ni solo menú contextual.
   Aparece solo con utilidad: selección no vacía → panel de texto; cursor en
   enlace → editar enlace; cursor en callout → cambiar tipo; cursor en heading
   → cambiar nivel; cursor plano → no se muestra.
2. **Una sola capa de comandos** (`win.*` GAction) para popover, menús,
   atajos y Command Palette: la acción es la misma y su dispatch edita el
   buffer Markdown de la pestaña activa con `Editor::replace_all_selecting`
   (una transacción de undo/redo §8.5 con selección preservada §8.6).
   Cancelar (Escape, diálogo cancelado) no cambia el documento.
3. **Vías de entrada**: menús `Insert` y `Format` (completan la barra §36) y
   Command Palette mínima §38 (`Ctrl+Shift+P`) sobre `ALL_COMMANDS`.
4. **Atajos sin conflicto** (§39 se conserva íntegro; `Ctrl+B` sigue siendo
   Toggle explorer): `Ctrl+I` cursiva, `Ctrl+K` enlace, `Ctrl+Shift+B`
   negrita, `Ctrl+Shift+X` tachado, `Ctrl+Shift+C` código, `Ctrl+1..6`
   headings. Atajos configurables quedan fuera de alcance.
5. **Semántica de edición** (fuente `core/minmark-document/src/format.rs`):
   toggle inline con detección de envoltorio (dentro o fuera de la selección),
   cerco de código proporcional al contenido, headings ATX por línea cubierta,
   enlaces `[texto](url)` (editar solo la URL cuando ya hay enlace) y callouts
   `::: tipo` (§22) conservando título y contenido al retonar. Lo no
   expresable como edición limpia (sintaxis no soportada §8.4) no toca el
   documento.

## Consecuencias

- #29 (texto/enlaces/callouts) crea la infraestructura; #30 (imágenes/tablas)
  y #31 (ecuaciones/citas) la reutilizan sin cambiar el contrato.
- Limitaciones de #29: wiki links y reference links sin controles; código
  inline multilínea no aplicable; los controles de callout solo cambian el
  tipo (el título se edita por escritura directa); el «bloque» que envuelve
  un callout es el párrafo (líneas no vacías contiguas); las imágenes
  `![alt](ruta)` no se tocan con la acción de enlace (controles en #30).
- El popover vigila la pestaña activa: al cambiar de pestaña o de modo se
  cierra y ninguna acción se aplica al documento equivocado.

## Desviaciones de implementación (aceptadas)

- El trait receptor de comandos existente es `menus::actions::Commands` (no
  hay `AppActions`): los comandos nuevos se añaden allí, con no-ops por
  defecto.
- «Command Palette» necesita entrada de menú para el test de coherencia
  §36/§38: se coloca en **View** (convención VS Code).
- `win.insert-callout` es una acción GAction sin parámetros que lleva el tono
  en su estado: los botones del popover fijan el estado y activan el mismo
  `win.*`, de modo que todos los caminos convergen en `apply_format_active`.
- Navegación por teclado de los controles: `Ctrl+.` enfoca el primer control
  cuando el popover está visible (Escape devuelve el foco al editor).
