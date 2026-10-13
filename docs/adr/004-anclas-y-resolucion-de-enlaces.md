# ADR-004: Política única de anclas y resolución de destinos

**Fecha:** 2026-10-13
**Estado:** Aceptada
**Contexto:** ISSUE (Fase 4 #4) - Unificar resolución de destinos y anclas en el índice; SRS §17, §19, §26, §27.1, §45

## Contexto

La Fase 4 necesita responder «¿adónde apunta este enlace?» desde navegación
(§19.4), autocompletado, enlaces rotos (§19.5), wiki links (§19.3), backlinks
(§19.6) y referencias cruzadas (§26.5). El índice (§45) ya guardaba headings y
destinos, pero sin posiciones ni anclas, y cada consumidor resolvía a su
manera (o no resolvía).

Restricciones:
- Preview e índice deben producir los mismos destinos: un `#fragmento`
  resuelto por el índice debe apuntar al `id` que emite el Preview.
- El índice es std-only, incremental, reconstruible y vive fuera del
  proyecto (§45, §45.1); sin dependencias nuevas (§4.4).
- Markdown como fuente de verdad: resolver no escribe (§6.1, §6.2).
- Los buffers abiertos con cambios sin guardar priorizan sobre el disco.

## Opciones evaluadas

1. **Anclas derivadas del AST renderizado**: el parser (pulldown-cmark) produce
   los headings reales, pero `minmark-fs` no depende del parser (§4.1) y la
   AST no distingue ATX de setext ni headings dentro de citas; habría dos
   extracciones que podrían divergir.
2. **Solo IDs explícitos `{#id}`** (comportamiento previo): `#fragmento` solo
   funcionaría para headings con atributos, insuficiente para §19.2.
3. **Slug estilo GitHub calculado aparte en cada lado**: cualquier matices
   (marcas inline, duplicados) rompe la paridad silenciosamente.

## Decisión

**Un único escáner y una única política de IDs en `minmark-document`, y un
único resolutor sobre el `ProjectIndex` en `minmark-fs`.**

### Política de IDs (§26)

1. ID explícito `{#id}` (§26.1) tal cual (charset `[A-Za-z0-9:._-]+`).
2. Sin explícito: slug del texto plano del heading (marcas inline eliminadas),
   minúsculas Unicode, letras/dígitos/`-` conservados, espacios → `-`; texto
   vacío → `seccion`.
3. Duplicados en orden de documento con sufijo `-1`, `-2`, … sobre su propia
   base (también para IDs explícitos repetidos).
4. El fragmento debe coincidir exactamente con el ancla (sin plegado).

### Arquitectura

- `minmark_document::anchors`: `find_heading_nodes` (ATX, setext con párrafo
  completo, `{#id}`, citas, líneas y spans) + `document_anchors`. El Preview
  emite `id` por línea de inicio del heading y el índice guarda el ancla y la
  línea (`minmark-index 2`); ambos comparten la función, luego la paridad es
  por construcción y está testada.
- `minmark_document::attrs::parse_attributes` es la gramática `{#id ...}`
  única (§7.4.3), compartida por el parser y el escáner.
- `minmark_fs::resolve::Resolver` clasifica cada destino en Local, External,
  Missing o Ambiguous (§19) sobre el índice + buffers abiertos; `ops` (§16)
  reutiliza `split_fragment`/`has_scheme`/`wiki_name_matches` del resolutor.

### Razones

1. Un solo lugar que define «qué es un destino y qué es un ancla»: los
   consumidores de la Fase 4 no crean índices ni resolutores paralelos.
2. std-only en `minmark-document` y `minmark-fs`; sin dependencias nuevas.
3. El formato de índice se amplía con ancla y línea (`minmark-index 2`): los
   índices antiguos se descartan y reconstruyen (§45: siempre reconstruible).

## Consecuencias

- Navegar y diagnosticar enlaces (Fase 4) resuelve contra el índice sin
  releer todo el proyecto; los buffers abiertos mandan sobre el disco.
- Frontera documentada: los atajos `[texto]` no son enlaces por referencia
  (indistinguibles de texto entre corchetes); los fragmentos distinguen
  mayúsculas; los archivos ignorados (§14.6) resuelven como inexistentes.
- La reescritura de renombrados (§16) comparte la regla de nombre de los
  wiki links: `[[nombre]]` resuelve por tallo o por ruta sin `.md`/`.markdown`.
