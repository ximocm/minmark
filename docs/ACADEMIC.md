# Funciones académicas — fase 5

Ecuaciones LaTeX, bibliografía local, citas Pandoc, notas, numeración y TOC.
Markdown sigue siendo la fuente; los resultados son derivados y no entran en el
historial de edición.

## Preparar un documento

```yaml
---
bibliography: [references.bib, extra.bib]
bibliography_style: author-year.csl
heading_numbering: true
counter_numbering: by_chapter
toc_max_level: 3
language: es
pagination: false
---
```

`bibliography` admite una ruta, una lista inline o una lista YAML indentada.
IEEE viene incluido y es el estilo por defecto. Otro estilo requiere un archivo
CSL independiente local; los CSL dependientes deben resolverse previamente a su
estilo independiente. No se descargan recursos automáticamente.

Las rutas del documento parten de su carpeta; las del proyecto, de su raíz;
las globales, de la carpeta de `config.toml`. Los recursos deben estar dentro del
ámbito correspondiente. En un documento aún sin ruta dentro de un proyecto se
usa su raíz. Los archivos ausentes, inválidos y las claves duplicadas muestran
avisos; el documento continúa siendo editable. Un CSL inválido vuelve a IEEE
con aviso. Los cambios en `.bib` y CSL refrescan el resultado sin marcar el
Markdown como modificado.

Configuración de proyecto (`.mdproject.toml`) y global (`minmark/config.toml`):

```toml
bibliography = ["references.bib", "extra.bib"]
bibliography_style = "author-year.csl"
heading_numbering = true
counter_numbering = "by_chapter"
toc_max_level = 3
pagination = false
```

Precedencia: documento → proyecto → global → defecto. Bibliografía y estilo no
aceptan overrides del nivel composición.

## Escribir y consultar

- Matemáticas: `$E=mc^2$` y bloques `$$`. Fracciones, raíces, índices, griegas,
  operadores, matrices, funciones y acentos del [ADR-002](adr/002-latex-a-typst.md).
  Preview compone SVG con Typst; PDF usa el mismo traductor y fuentes. Sintaxis
  incompleta o no admitida produce un marcador; PDF registra el aviso.
- Citas: `[@smith2025]`, `[@smith2025; @jones2024]`,
  `[@smith2025, p. 42]`. Locators: `p.`, `pp.`, `ch.`, `sec.`, `fig.`, `tbl.`, `eq.`.
  La bibliografía de las citas se añade al final con el mismo motor CSL en
  Preview y PDF.
- Autocompletado: escribir `[@` y buscar por clave, autor, título o año.
  Flechas seleccionan; Tab/Enter aceptan; Escape cancela. Aceptar cambia solo
  la clave en una operación de undo y conserva los locators.
- Notas: `Texto.[^nota]` y `[^nota]: Contenido.`. Numeración por primera aparición;
  referencias repetidas comparten número. En continuo, notas al final con
  enlaces de ida y vuelta; en paginado/PDF, al pie de la página. Las notas
  ausentes, duplicadas o anidadas producen diagnósticos.
- Numeración: capítulos definidos por H1. Headings desactivados por defecto;
  `heading_numbering: true` añade `1`, `1.1`, `1.1.1` a la representación.
  `counter_numbering` acepta `continuous` (defecto), `by_chapter` (`2.1`)
  y `by_chapter_reset` (reinicio simple). Referencias y captions comparten números.
- TOC: únicamente mediante `{{toc}}`; H1–H3 por defecto y profundidad 1–6.
  En continuo muestra enlaces; en paginado/PDF muestra páginas calculadas por
  Typst. Código y escapes conservan el texto literal.

`pagination: true` activa Preview paginado. La composición de páginas, notas y
TOC es la misma que en PDF. La edición y carga de recursos se procesan en el
worker existente de la ventana; los resultados obsoletos no sustituyen el buffer
actual. Exportar mediante **File → Export PDF** / **Ctrl+E** usa el buffer sin guardar.

## Verificación y límites

[Suite y registro de QA](qa/README.md). Fixtures incluidas, pruebas de lógica,
integración GTK/WebKit, capturas y revisión de PDF multipágina.

El alcance matemático es el subconjunto del ADR-002, no LaTeX completo.
El contenido y formato inline CSL (cursiva, negrita, versalitas, subrayado y
super/subíndices), sangría francesa, alineación de campos y espaciado de entradas
se comparten entre HTML y PDF. Las fuentes embebidas no contienen todos los glifos CJK: el fuente
y Preview continuo conservan `東京`, pero paginado/PDF puede mostrar glifos
ausentes. La accesibilidad con lector de pantalla y Wayland requiere un entorno
con esos servicios; el registro distingue esas comprobaciones de las realizadas.
