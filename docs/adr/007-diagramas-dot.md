# ADR-007: Diagramas DOT con motor Rust y paridad Preview/PDF

**Fecha:** 2026-10-28
**Estado:** Aceptada
**Contexto:** Issue #68: «mermaid compatibility or similar». El SRS v1.3 no
contempla diagramas; la petición exige renderizar diagramas desde el fuente
Markdown. Revisión obligatoria de dependencias (§4.4) y criterios §56.

## Contexto

Las opciones evaluadas:

1. **Mermaid real** (mermaid.js vendoreado en el webview): compatibilidad
   total de sintaxis, pero el PDF sale de Typst, que no ejecuta JavaScript:
   la paridad exigida obligaría a capturar SVG desde el webview durante la
   exportación (tres backends, temporización, headless) o a degradar el PDF
   a bloque de código. Además ~3 MB de JS vendoreado.
2. **Motor DOT Rust puro** (sintaxis Graphviz «similar»): SVG generado en el
   core, embebible igual en el HTML de Preview y como recurso SVG en el PDF
   (el pipeline Typst ya embebe SVG, §18.1).
3. **Subconjunto de mermaid implementado a medida** (flowcharts → SVG):
   sintaxis mermaid parcial, pero exige un motor de layout propio: no es una
   solución pequeña ni mantenible.

Decisión del propietario (2026-10-28): **opción 2**, con **paridad
Preview/PDF obligatoria en v1**.

## Decisión

- Dialecto: bloques cercados ` ```dot ` con sintaxis **Graphviz (DOT)**
  (§7.3.1). Extensiones futuras posibles (p. ej. otros lenguajes) sin tocar
  el contrato de render.
- Motor: **`layout-rs`** (MIT, Rust puro, port de Graphviz con parser DOT +
  layout + backend SVG; sin binarios externos, sin red, sin engine de JS).
  Es la única dependencia nueva de la función (§4.2: una responsabilidad,
  una solución; §56-Q7: sí justifica una dependencia por toda la función).
  Se encapsula en el crate propio `minmark-diagram` (patrón `minmark-math`):
  el resto del workspace no conoce `layout-rs`.
- Paridad: el mismo SVG alimenta el HTML de Preview (SVG embebido con
  `namespace_svg`) y el PDF (recurso SVG con las mismas reglas de escala de
  las imágenes, §18.6: sin ampliar, sin recortar, reducción proporcional).
- Errores (patrón ADR-002/NFR-026, nunca silenciar ni deformar): DOT
  inválido → marcador visible `⚠ [diagrama no válido: …]` idéntico en
  Preview y PDF; el bloque de código permanece visible y editable (§8.4).
- Paridad tipográfica: los glifos del SVG se convierten a **contornos** con
  las fuentes empaquetadas (Liberation Sans §33 + las de `typst-assets`, el
  mismo set que el world del PDF). El SVG resultante no contiene `<text>` ni
  familias externas: Preview y PDF dibujan exactamente lo mismo, con
  independencia de las fuentes del host y del `body_font` del documento.
- Subconjunto validado: antes de renderizar se comprueba que los atributos
  y valores pertenecen al conjunto que el motor aplica sin deformar
  (claves: `label`, `shape`, `color`, `fillcolor`, `style`, `fontsize`,
  `penwidth`, `width`, `rankdir`; `shape` ∈ elipse/círculo, caja, record,
  doble círculo; `rankdir` ∈ LR/TB; `style` ∈ solid/dashed/filled). Lo que
  el motor sustituiría en silencio (p. ej. `shape=diamond` → elipse,
  `rankdir=BT` → TB, clusters) produce el marcador visible.
- Tema: colores neutros legibles en claro y oscuro (el DOT del usuario puede
  fijar los suyos; el área del diagrama usa fondo claro).

## Consecuencias y limitaciones

- La sintaxis no es mermaid: los diagramas mermaid existentes se reescriben
  en DOT. La decisión prioriza paridad PDF, proceso 100% local y promesa de
  dependencias lean.
- `layout-rs` no soporta clusters anidados ni todas las atributos de
  Graphviz; lo no soportado se documenta y el error sigue el patrón de
  marcador visible.
- Rendimiento (§56-Q6): el layout corre en cada refresco de render; con
  diagramas de tamaño normal es imperceptible y queda verificado; si fuera
  necesario, caché por hash del bloque en una iteración posterior.
- El editor sigue mostrando el fuente DOT como bloque de código (resaltado
  genérico): el diagrama renderizado vive en Preview/PDF, no se sustituye el
  fuente (§6.1).

## Alternativas descartadas

- Mermaid.js vendoreado y captura de SVG por webview (ver Contexto).
- `graphviz-rust`: construye/exporta DOT pero no hace layout; su vía de
  render invoca el binario externo `dot` — incompatible con el producto.
- Layout propio sobre un subconjunto de mermaid (ver Contexto).
