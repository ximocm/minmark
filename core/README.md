# `core/` — núcleo de minmark

Workspace Cargo con el core en Rust (§5.1). Cumple la Lean Dependency Promise
(§4): `std`-first, una responsabilidad por crate y una sola solución por
problema (§4.2: un parser, un renderer, un motor PDF).

## Crates

| Crate | Responsabilidad | Estado |
|---|---|---|
| `minmark-document` | Modelo documental: buffer Markdown único (§6.1), AST compartida, editor Markdown Mode (§8) | editor, undo/redo y búsqueda (**ISSUE-015/027**) |
| `minmark-config` | Modelo único de ajustes (§58.2), precedencia de 5 niveles (§58.1) y temas (§11) | resolución + detección de tema (**ISSUE-017**) |
| `minmark-parser` | pulldown-cmark + dialecto §7 → AST | CommonMark+GFM, extensiones y plan de edición visual (**ISSUE-014/030–034**) |
| `minmark-renderer` | AST → página HTML del Preview | página completa, scroll sync (§9.1) y paletas de tema (**ISSUE-016/017**); WebView en **ISSUE-020** |
| `minmark-math` | Traductor LaTeX → Typst para math (ADR-002) | subconjunto estándar + marcadores NFR-026 (**ISSUE-018**) |
| `minmark-pdf` | Frontera del motor PDF (§5.3) y geometría (§32) | `TypstEngine` sobre `typst` (§4.3): AST → Typst → PDF (**ISSUE-018**) |
| `minmark-fs` | File I/O, guardado atómico, snapshots (§12, §13, §48) | file I/O, snapshots y seguridad (**ISSUE-013/019**); operaciones, importación, índice, watching, búsqueda y Quick Open (**ISSUE-023–029**) |

## Grafo de dependencias

Las flechas van del crate consumidor a sus dependencias internas de producción:

```text
minmark-document    (sin dependencias)
minmark-config      (sin dependencias)
minmark-math        (sin dependencias)
minmark-parser   ──> minmark-document
minmark-renderer ──> minmark-document, minmark-config, minmark-math
minmark-pdf      ──> minmark-document, minmark-config, minmark-math,
                    minmark-renderer
minmark-fs       ──> minmark-document, minmark-parser
```

Las dependencias de desarrollo se declaran en los manifiestos de cada crate.

El grafo de producción no tiene ciclos.

## Interfaces entre crates

- **`minmark_document::Ast`** es la interfaz central: `minmark-parser` la
  produce; `minmark-renderer` y `minmark-pdf` la consumen. Cada `Block` lleva
  su `Span` (offsets de byte en el fuente) para fidelidad (§6.2) y
  sincronización por posición semántica (§9.1).
- **`minmark_config::Settings`** es el modelo único de ajustes; la resolución
  de la cadena documento → composición → proyecto → global → defecto es
  `minmark_config::resolve` (§58.1). Claves desconocidas, niveles no
  aplicables y valores inválidos se ignoran con aviso no bloqueante.
- **`minmark_fs::FileStore`** (`LocalFs` lo implementa, ISSUE-013):
  `create_new`, `open`, `save`, `save_as`. Firma de error `FsError` cubre los
  fallos normativos: UTF-8 (§12.5), E/S, sin path (`MissingPath`), path
  traversal (§48) y modificación externa (§12.7). Filesystem safety
  (**ISSUE-019**): guardado atómico (`write_atomic`, temporal + rename),
  `LocalFs::save_confirmed` para la sobrescritura tras confirmación explícita
  del usuario (§3.7), validación de rutas en modo proyecto
  (`resolve_in_project`, §48), árbol con symlinks sin recursión ni ciclos
  (`walk_tree`, §14.7) y `SnapshotStore` fuera del proyecto (§45.1). La
  notificación de cambios externos (§12.7) se monta sobre `minmark_fs::Watcher`
  (`InotifyWatcher`, §46): solo emite eventos, nunca recarga ni escribe.
- **`minmark_pdf::PdfEngine`** (ISSUE-018 lo implementa con Typst): aísla la
  dependencia estratégica del resto del core (§4.3).

Los traits existen solo en fronteras con sustitución real (almacenamiento,
motor PDF). Dentro del core hay una sola implementación por responsabilidad
(§4.2): `parse` y `render_html` son funciones, no traits.

## Decisiones de diseño

- **HTML nunca crudo** (§7.2): el parser preserva el HTML embebido como texto
  y el renderer escapa todo; el Preview no ejecuta contenido activo.
- **Contenido byte a byte** (§6.2): el buffer no normaliza saltos de línea;
  `LineEnding` detecta el formato para preservarlo al guardar (§12.6).
- **Front matter YAML** (§21) extraído crudo en `Ast::front_matter`: es la
  fuente del nivel documento de configuración (§58.1).
- **Listas tight**: pulldown-cmark no las envuelve en párrafo; el parser genera
  un párrafo implícito para que la AST sea uniforme.
- **Valores fijos no configurables** (§58.2) no son ajustes: UTF-8 solo,
  LF en archivos nuevos, milímetros en PDF.

## Dependencias de producción

| Dependencia | Por qué no `std` (§4.4) | Impacto |
|---|---|---|
| `pulldown-cmark 0.12` (`default-features = false`) | implementar un parser CommonMark+GFM correcto no es razonable a mano; es el parser único (§4.2), versión benchmarkada en ISSUE-010 | features por defecto desactivadas; consultar el árbol resuelto con Cargo (§4.5) |
| `typst 0.13` + `typst-pdf` + `typst-assets` (fonts) | único motor de composición y PDF (§4.3, ADR-003): layout, paginación, tipografía y math; reemplaza cualquier alternativa de composición | dependencia estratégica con árbol transitivo amplio; fuentes embebidas (§33); Cargo reutiliza los artefactos compilados |

Typst (§4.3, ADR-003) es la única dependencia estratégica del core. GTK4/WebKit6
(ADR-001) son del binario de UI (ISSUE-020), no del core. hayagriva (ADR-003)
se añadirá en Fase 5 con las citas reales.

## Build y tests

Desde la raíz del repositorio; no requiere GTK ni WebKit:

```sh
cargo build --locked --manifest-path core/Cargo.toml --workspace
cargo test --locked --manifest-path core/Cargo.toml --workspace
cargo tree --locked --manifest-path core/Cargo.toml -e normal
```

Los requisitos y comandos de la aplicación están en [BUILDING.md](../BUILDING.md).

La suite incluye tests unitarios y de integración: modelo de documento y editor
(operaciones, undo/redo §8.5, posiciones §8.6, latencia NFR-005 sobre 20 MB),
resolución de configuración y temas (detección XDG/GTK/KDE, §11), traductor
math LaTeX→Typst (ADR-002), parser (dialecto §7: spans, GFM, extensiones,
§7.2, front matter), corpus positivo/negativo del dialecto, highlighting,
renderer (escape §7.2, nodos extendidos, scroll sync §9.1, paletas §11),
exportación PDF end-to-end (§32, NFR-030 texto seleccionable), file I/O
(§12.5–12.7) y reglas de snapshots (§13.5), más el pipeline de integración
abrir → parsear → renderizar.
