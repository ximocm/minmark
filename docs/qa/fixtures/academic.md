---
bibliography: [references.bib, extra.bib]
heading_numbering: true
counter_numbering: by_chapter
toc_max_level: 3
language: es
---
{{toc}}

# Energía {#energy}

Cita simple [@smith2025], múltiple [@smith2025; @jones2024] y locator [@smith2025, p. 42].

La energía es $E = mc^2$. Fracción $\frac{1}{2}$ y raíz $\sqrt[3]{x}$.

$$
\int_0^\infty e^{-x} dx
$$
{#eq:integral "Integral impropia"}

Ver {{ref:eq:integral}}. Una nota.[^nota] La misma nota otra vez.[^nota]

## Resultados {#results}

| Caso | Valor |
| --- | ---: |
| A | 10 |

{#tbl:values "Valores"}

Ver {{ref:tbl:values}}.

### Detalle

Texto con **énfasis** y Unicode: áéíóú, β, 東京.

#### Fuera del TOC por defecto

Esta sección aparece con `toc_max_level: 6`.

{{pagebreak}}

# Segundo capítulo

$$
\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}
$$
{#eq:matrix "Matriz"}

Ver {{ref:eq:matrix}}.

Otra nota.[^segunda]

[^nota]: Nota de energía con **negrita** y cita [@jones2024].

[^segunda]: Nota del segundo capítulo.

`[@smith2025] $\frac{1}{2}$ {{toc}} [^literal]`

\{{toc}} y \[@smith2025] se conservan literalmente.
