# Cómo citar minmark

Si usas minmark en un trabajo académico, una publicación o un proyecto,
cita el software. La referencia recomendada identifica autor, software,
versión concreta y URL de repositorio.

## Referencia recomendada

APA 7:

> Cabanes Martinez, J. (2026). *minmark* (0.3.0) [Software]. GitHub.
> https://github.com/ximocm/minmark

BibTeX:

```bibtex
@software{minmark2026,
  author  = {Cabanes Martinez, Joaquim},
  title   = {minmark: editor de Markdown de escritorio para Linux},
  year    = {2026},
  version = {0.3.0},
  url     = {https://github.com/ximocm/minmark},
  license = {Apache-2.0}
}
```

## Versiones y fechas

- Cita la **versión exacta** que usaste, no solo la rama `main`. La versión
  actual y sus cambios están en el [changelog](../CHANGELOG.md); los releases
  publicados están en
  [GitHub Releases](https://github.com/ximocm/minmark/releases).
- Sustituye `0.3.0` y el año por los de la versión empleada si es distinta.
- El archivo [`CITATION.cff`](../CITATION.cff) de la raíz es la fuente
  máquina-legible de esta referencia. GitHub muestra un botón
  **Cite this repository** que exporta BibTeX, APA y CSL a partir de él.

## Citar dentro de minmark

Para insertar citas en tus propios documentos con minmark (bibliografías
`.bib`, estilo CSL, citas Pandoc `[@clave]`, notas al pie y numeración),
consulta [Academic](ACADEMIC.md) y el
[documento de demostración](SHOWCASE.md#citas-y-bibliografía).
