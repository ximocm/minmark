# Composiciones y salida documental — fase 6

Una composición construye un documento lógico a partir de varios `.md` sin
concatenar ni modificar los archivos originales (§20). Este documento combinado
tiene Preview completo (continuo o paginado) y se exporta como un único PDF
(§20.4, §32.1).

## Declarar composiciones

En `.mdproject.toml`, una tabla `[[composition]]` por composición:

```toml
[[composition]]
name = "Libro"
files = ["portada.md", "capitulos/01.md", "capitulos/02.md"]
page_break_between_files = true
pdf_header = "Libro — {chapter} — {page}/{pages}"
```

- `name` y `files` son obligatorios; `files` declara el orden explícito (§20.1).
  Las rutas que salen de la raíz del proyecto (`../`, absolutas o symlinks que
  escapan) nunca se leen: su hueco muestra «Archivo fuera del ámbito» (§48).
- `page_break_between_files` (defecto `false`, §20.3) fuerza página nueva entre
  archivos en paginado/PDF. `{{pagebreak}}` dentro de un archivo hace lo mismo
  (§31); en Visual se muestra como separador discreto y en Markdown literal.
- Cualquier ajuste de configuración aplicable puede declararse como override de
  la composición (§58.1): `pagination`, `pdf_page_size`, `pdf_margins_mm`,
  `pdf_orientation`, `pdf_page_number_position`, `pdf_header`, `pdf_footer`,
  `body_font`, `font_size_pt`, `line_height`, `heading_numbering`,
  `counter_numbering`, `toc_max_level`, `language`, etc.

Tablas con `name` o `files` vacíos se descartan con aviso no bloqueante; las
claves desconocidas o los valores inválidos se ignoran con aviso (§58.1).

## Precedencia de ajustes

Cadena única (§58.1): **documento** (front matter del documento lógico) >
**composición** (overrides de la tabla) > **proyecto** (`.mdproject.toml`) >
**global** (`$XDG_CONFIG_HOME/minmark/config.toml`) > **defecto** (§58.2).

La bibliografía se une en orden desde el front matter de cada archivo incluido,
resuelta relativa al directorio de su archivo y con los límites de proyecto
habituales. El estilo bibliográfico (`bibliography_style`) no tiene nivel
composición (§25.4).

## Uso

- **Project → Preview Composition** (Command Palette: «Preview Composition»):
  Preview combinado de la composición seleccionada, continuo o paginado según el
  ajuste `pagination` efectivo. Los buffers abiertos sin guardar se usan en
  lugar del disco (§6.1). Un archivo ausente aparece como bloque de diagnóstico
  en su posición, sin abortar el resto.
- **File → Export Composition PDF** (Command Palette: «Export Composition PDF»):
  un único PDF con el mismo flujo de destino, cancelación, sobrescritura y
  errores que la exportación individual (§32.1).

## Referencias y numeración entre archivos

Los enlaces y wiki links entre archivos incluidos se resuelven como destinos
internos del documento final (§20.4): `{{ref:fig:uno}}` puede apuntar a una
figura declarada en otro archivo y expande a su etiqueta numerada. La numeración
de figuras, tablas y ecuaciones es continua por defecto o por capítulo (§27.2,
`counter_numbering`), contando los H1 del documento combinado como capítulos sin
importar el archivo en que estén (§27). Los niveles de heading se preservan
exactamente (§20.2): `#` sigue siendo H1 y un límite de archivo no define
capítulo. Las notas al pie son locales a cada archivo y no colisionan al
combinar. Los IDs duplicados y los destinos ausentes se diagnostican sin tocar
los fuentes.

## Captions

Las figuras con identificador `fig:` usan su texto alternativo como pie de
figura numerado; las tablas `{#tbl:results "Resultados"}` y ecuaciones
`{#eq:energy "..."}` muestran su caption debajo (§28). Caption, número y
referencia cruzada coinciden en Preview y PDF y respetan idioma (`es`/`en`) y
configuración.

## Página, cabeceras y tipografía

- Tamaños A3/A4/A5/A6/Letter/Legal/Executive y `Custom` en milímetros (§32.3):
  `pdf_page_size = { width_mm = 120, height_mm = 240 }`. Márgenes 25 mm por
  defecto (§32.5); orientación Portrait/Landscape (§32.6).
- Número de página solo con paginación activada (§32.7) en una de las seis
  posiciones de §32.8 (`pdf_page_number_position`, defecto `footer_right`).
- Cabeceras y pies (§32.9) como plantillas de texto (`pdf_header`,
  `pdf_footer`) con las variables `{title}`, `{author}`, `{project}`,
  `{chapter}`, `{page}`, `{pages}` y `{date}`. Las variables solo se interpretan
  en cabeceras y pies: en el cuerpo Markdown esos textos son literales (§7.4.5).
  `{chapter}` sigue el H1 más reciente y `{page}`/`{pages}` quedan vacíos sin
  paginación.
- Tipografía (§33): fuentes base incluidas — sans-serif (Liberation Sans),
  serif (Libertinus Serif) y monoespaciada (DejaVu Sans Mono) — sin depender de
  las fuentes del sistema en el pipeline Typst (PDF, paginado y matemáticas).
  El Preview continuo HTML sigue resolviendo familias genéricas con las
  fuentes del sistema.

## Limitaciones

- El Preview HTML continuo no incrusta las fuentes base; el aspecto base del
  PDF y del paginado sí está garantizado.
- Las fuentes `.ttf`/`.otf` del proyecto y los section breaks con cambio de
  orientación siguen en fase posterior (§31, §55).
- La comprobación manual de la app real y el registro de QA de la fase están en
  [qa](qa/README.md).

## Verificación

Los tests cubren el parser de composiciones (`minmark-config`), la construcción
del documento combinado y sus referencias (`minmark-document`), la exportación
completa (PDF, paginado, cabeceras, captions y tablas) y la capa de UI
(orden, buffers sin guardar, archivos ausentes, overrides y assets por archivo).
Véase el [changelog](../CHANGELOG.md) para el estado de cada versión.
