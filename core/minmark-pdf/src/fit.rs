//! Ajuste proporcional de tablas en la exportación PDF (§35.2, ISSUE-027).
//!
//! §35.2: «La tabla se reducirá proporcionalmente hasta un mínimo de 8 pt.
//! Si aun así no cabe, la exportación se detendrá con un error que
//! identifique la tabla problemática. No se recortará.» El mínimo es el
//! ajuste `table_min_font_pt` (defecto 8 pt) y el cuerpo es
//! [`Settings::font_size_pt`].
//!
//! Cada tabla se emite **nativa** dentro de `#layout(size => ...)`:
//!
//! 1. Se miden los **runs** de cada columna (secuencias continuas sin
//!    oportunidad de quiebre: solo el espacio/tab ASCII de `Inline::Text` y
//!    los quiebres de línea parten; NNBSP u otros espacios se quedan dentro
//!    del run) a la tipografía del cuerpo, con el `set` **dentro** del
//!    contenido medido porque un `set` de código no llega a `measure`.
//! 2. Con `minw(f) = A·f + B` (runs escalables + inset/stroke; fijos en B)
//!    se elige `f` con `table_min_font_pt ≤ f ≤ font_size_pt`; si ni el
//!    suelo cabe, `panic` detiene la exportación identificando la tabla.
//! 3. El reparto de columnas se emite con **anchos calculados**
//!    (`columns: mm-fit-cols`): Typst 0.13.1 no admite `minmax`, así que
//!    `mm-fit-track` replica el reparto justo de `auto` (naturales medidos a
//!    la tipografía compuesta) y solo corrige donde el suelo por columna no
//!    cabe — el reparto justo de `auto` puede dar a una columna menos que su
//!    mínima y dejar el token inseparable fuera del área.
//!
//! La tabla no se envuelve en `box` ni `scale`: el reparto multipágina se
//! conserva y el caption (§28) se emite fuera del envoltorio, sin reducirse.

use crate::generate::{
    citation_text, color_hex, escape_text, plain_text, render_inlines, typst_string,
};
use minmark_config::Settings;
use minmark_document::{Align, Ast, Attributes, Block, BlockKind, Inline, Substitution};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// Inset por defecto de `table` en Typst (pt a tamaño base). Se emite
/// explícito para que con `mm-fit-u = 1` el aspecto sea el de hoy.
const TABLE_INSET_PT: usize = 5;
/// Stroke por defecto de `table` en Typst (pt a tamaño base).
const TABLE_STROKE_PT: usize = 1;

/// Helpers Typst (§35.2) emitidos una sola vez en el preámbulo:
/// `mm-fit-track(nats, mins, w)` devuelve el ancho de cada columna — el
/// reparto justo de `auto` cuando todas las mínimas caben, y mínimas +
/// reparto sobre la cabeza restante donde el suelo vincula (validado contra
/// los casos de referencia: naturales caben → naturales; recorte → reparto
/// justo; suelo dominante → suelo + resto).
pub(crate) const TRACK_HELPERS: &str = "\
#let mm-fit-fair(nats, w) = {
  let out = nats.map(_ => 0pt)
  let remaining = w
  let n = nats.len()
  let done = nats.map(_ => false)
  let progress = true
  while progress {
    let k = 0
    let i = 0
    while i < n {
      if not done.at(i) { k = k + 1 }
      i = i + 1
    }
    if k == 0 {
      progress = false
    } else {
      let share = remaining / k
      let changed = false
      let i = 0
      while i < n {
        if not done.at(i) and nats.at(i) <= share {
          out.at(i) = nats.at(i)
          remaining = remaining - nats.at(i)
          done.at(i) = true
          changed = true
        }
        i = i + 1
      }
      if not changed {
        let i = 0
        while i < n {
          if not done.at(i) { out.at(i) = share }
          i = i + 1
        }
        progress = false
      }
    }
  }
  out
}

#let mm-fit-track(nats, mins, w) = {
  let b = mm-fit-fair(nats, w)
  let viol = false
  let i = 0
  while i < b.len() {
    if b.at(i) < mins.at(i) { viol = true }
    i = i + 1
  }
  if not viol {
    b
  } else {
    let head = ()
    let i = 0
    while i < nats.len() {
      head.push(calc.max(0pt, nats.at(i) - mins.at(i)))
      i = i + 1
    }
    let l = w - mins.sum()
    let a = mm-fit-fair(head, l)
    let out = ()
    let i = 0
    while i < mins.len() {
      out.push(mins.at(i) + a.at(i))
      i = i + 1
    }
    out
  }
}
";

/// Envoltorio de estilo `(apertura, cierre)` alrededor de un run.
type Wrap = (String, String);

/// Expresiones de ancho por columna: `scaled[j]` mide el run completo a
/// tamaño base (escala con la fuente); `fixed[j]` suma las piezas fijas del
/// run (imágenes u otras anchuras independientes del cuerpo).
struct Columns {
    scaled: Vec<Vec<String>>,
    fixed: Vec<Vec<String>>,
}

impl Columns {
    fn new(columns: usize) -> Self {
        Self {
            scaled: vec![Vec::new(); columns],
            fixed: vec![Vec::new(); columns],
        }
    }

    /// Vuelca el run acumulado de `cell` a su columna y lo cierra.
    fn push_cell(&mut self, column: usize, mut runs: Runs) {
        runs.close();
        self.scaled[column].extend(runs.scaled);
        self.fixed[column].extend(runs.fixed);
    }
}

/// Run abierto: piezas escalables concatenadas (se miden JUNTAS: no hay
/// quiebre entre ellas) y piezas fijas del mismo run (se suman).
#[derive(Default)]
struct Runs {
    scaled: Vec<String>,
    fixed: Vec<String>,
    open: String,
    open_fixed: Vec<String>,
}

impl Runs {
    fn piece(&mut self, markup: &str) {
        self.open.push_str(markup);
    }

    fn fixed(&mut self, expr: String) {
        self.open_fixed.push(expr);
    }

    /// Cierra el run en curso. La medida lleva el `set` dentro del contenido
    /// y a la tipografía base: un `set` de código no alcanza a `measure` y el
    /// contexto ambiente (p. ej. una nota al pie con otra medida) subestimaría.
    fn brk(&mut self) {
        if !self.open.is_empty() {
            self.scaled.push(format!(
                "measure([\n#set text(size: mm-fit-base)\n{}]).width",
                self.open
            ));
            self.open.clear();
        }
        if !self.open_fixed.is_empty() {
            self.fixed.push(self.open_fixed.join(" + "));
            self.open_fixed.clear();
        }
    }

    fn close(&mut self) {
        self.brk();
    }
}

/// ¿Oportunidad de quiebre real? Solo espacio/tab ASCII: U+00A0/U+202F y
/// demás espacios los prohíben, y medirlos como quiebre subestimaría.
fn breaks(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Parte `text` en piezas de run separadas por quiebres ASCII, aplicando
/// `word_markup` a cada palabra y respetando espacios en los bordes.
fn push_words(runs: &mut Runs, text: &str, mut word_markup: impl FnMut(&str) -> String) {
    let starts = text.chars().next().is_some_and(breaks);
    let ends = text.chars().last().is_some_and(breaks);
    let mut first = true;
    for word in text.split(breaks) {
        if word.is_empty() {
            continue;
        }
        if !first || starts {
            runs.brk();
        }
        runs.piece(&word_markup(word));
        first = false;
    }
    if ends {
        runs.brk();
    }
}

/// Identidad documental de cada tabla (`span.start` → identidad) para el
/// mensaje de suelo de §35.2: el caption ya expandido (§26.5/§28 por
/// [`minmark_renderer::resolve_cross_references`]), si no el `#id`, y si no
/// `tabla sin ID nº <n>` con el nº 1-based por orden de documento. Los
/// ordinales se asignan sobre `span.start` porque `resolve_footnotes` mueve
/// las definiciones y el orden de recorrido dejaría de ser el documental.
pub(crate) fn table_identities(ast: &Ast) -> BTreeMap<u32, String> {
    fn collect<'a>(blocks: &'a [Block], out: &mut Vec<(u32, Option<&'a Attributes>)>) {
        for block in blocks {
            match &block.kind {
                BlockKind::Table { attrs, .. } => out.push((block.span.start, attrs.as_ref())),
                BlockKind::Quote { blocks } | BlockKind::FootnoteDefinition { blocks, .. } => {
                    collect(blocks, out)
                }
                BlockKind::Callout { blocks, .. } => collect(blocks, out),
                BlockKind::List { items, .. } => {
                    for item in items {
                        collect(&item.blocks, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut tables = Vec::new();
    collect(&ast.blocks, &mut tables);
    tables.sort_by_key(|(span, _)| *span);
    tables
        .into_iter()
        .enumerate()
        .map(|(index, (span, attrs))| {
            let identity = attrs
                .and_then(|a| a.caption.as_ref())
                .map(String::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    attrs
                        .and_then(|a| a.id.as_deref())
                        .map(|id| format!("#{id}"))
                })
                .unwrap_or_else(|| format!("tabla sin ID nº {}", index + 1));
            (span, identity)
        })
        .collect()
}

/// Emite la tabla envuelta en el ajuste de §35.2. Las celdas se renderizan
/// igual que antes (`render_inlines`); lo nuevo es el envoltorio con las
/// medidas, la decisión de tamaño, el reparto nativo de columnas y el
/// `panic` de suelo.
pub(crate) fn render_table(
    out: &mut String,
    settings: &Settings,
    labels: &BTreeSet<String>,
    aligns: &[Align],
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    identity: &str,
) {
    let columns = header.len().max(aligns.len()).max(1);
    let mut cols = Columns::new(columns);
    // Los runs se agrupan por columna como los reparte Typst: la cabecera
    // llena su fila y el cuerpo, plano, empieza en la columna 0.
    for (index, cell) in header.iter().enumerate() {
        cols.push_cell(index % columns, cell_runs(cell, settings, labels));
    }
    for (body_index, cell) in rows.iter().flatten().enumerate() {
        cols.push_cell(body_index % columns, cell_runs(cell, settings, labels));
    }

    let base = settings.font_size_pt;
    let floor = settings.table_min_font_pt;
    let message = format!(
        "{identity}: no cabe en el ancho disponible con table_min_font_pt={floor}pt (§35.2)"
    );
    let scaled_terms = (0..columns)
        .map(|j| format!("mm-fit-s{j}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let fixed_terms = (0..columns)
        .map(|j| format!("mm-fit-f{j}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let inset = TABLE_INSET_PT * 2;

    let _ = writeln!(out, "#layout(size => {{");
    let _ = writeln!(out, "  let mm-fit-base = {base}pt");
    let _ = writeln!(out, "  let mm-fit-floor = {floor}pt");
    for (j, (scaled, fixed)) in cols.scaled.iter().zip(&cols.fixed).enumerate() {
        if scaled.is_empty() {
            let _ = writeln!(out, "  let mm-fit-s{j} = 0pt");
        } else {
            let _ = writeln!(out, "  let mm-fit-s{j} = calc.max({})", scaled.join(", "));
        }
        if fixed.is_empty() {
            let _ = writeln!(out, "  let mm-fit-f{j} = 0pt");
        } else {
            let _ = writeln!(out, "  let mm-fit-f{j} = calc.max({})", fixed.join(", "));
        }
    }
    // A·f + B con A normalizado: P = A·base (inset y stroke van × mm-fit-u).
    let _ = writeln!(
        out,
        "  let mm-fit-P = {scaled_terms} + {}pt + {}pt",
        TABLE_INSET_PT * 2 * columns,
        columns + 1
    );
    let _ = writeln!(out, "  let mm-fit-B = {fixed_terms}");
    let _ = writeln!(
        out,
        "  let mm-fit-u = if mm-fit-P + mm-fit-B <= size.width {{"
    );
    out.push_str("    1.0\n");
    out.push_str("  } else if mm-fit-P * (mm-fit-floor / mm-fit-base) + mm-fit-B > size.width {\n");
    let _ = writeln!(out, "    panic({})", typst_string(&message));
    out.push_str("  } else {\n");
    out.push_str("    (size.width - mm-fit-B) / mm-fit-P\n");
    out.push_str("  }\n");

    // Suelo por columna: mínima + inset (el inset vive dentro del track, la
    // celda solo dispone de track − 2·inset; verificado con Typst 0.13.1).
    for j in 0..columns {
        let _ = writeln!(
            out,
            "  let mm-fit-m{j} = mm-fit-s{j} * mm-fit-u + mm-fit-f{j} + {inset}pt * mm-fit-u"
        );
    }
    // Natural por columna a la tipografía compuesta (las celdas se miden con
    // su propio `set` porque el del closure no llega a `measure`), tope del
    // contenedor como el auto de Typst.
    for j in 0..columns {
        let cells: Vec<String> = header
            .iter()
            .enumerate()
            .filter(|(index, _)| index % columns == j)
            .map(|(_, cell)| cell_measure(cell, settings, labels))
            .chain(
                rows.iter()
                    .flatten()
                    .enumerate()
                    .filter(|(index, _)| index % columns == j)
                    .map(|(_, cell)| cell_measure(cell, settings, labels)),
            )
            .collect();
        if cells.is_empty() {
            let _ = writeln!(out, "  let mm-fit-n{j} = 0pt");
        } else {
            let _ = writeln!(
                out,
                "  let mm-fit-n{j} = calc.min(calc.max({}) + {inset}pt * mm-fit-u, size.width)",
                cells.join(", ")
            );
        }
    }
    let nats = (0..columns)
        .map(|j| format!("mm-fit-n{j}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mins = (0..columns)
        .map(|j| format!("mm-fit-m{j}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(
        out,
        "  let mm-fit-cols = mm-fit-track(({nats},), ({mins},), size.width)"
    );
    // El `set` va dentro del contenido: la composición de la tabla nunca
    // hereda la medida ambiente (p. ej. el 0.85em de una nota al pie).
    out.push_str("  [\n");
    out.push_str("    #set text(size: mm-fit-base * mm-fit-u)\n");
    out.push_str("    #table(\n");
    let _ = writeln!(out, "    columns: mm-fit-cols,");
    if !aligns.is_empty() {
        let aligns: Vec<&str> = aligns
            .iter()
            .map(|a| match a {
                Align::Center => "center",
                Align::Right => "right",
                Align::Left | Align::None => "left",
            })
            .collect();
        let _ = writeln!(out, "    align: ({},),", aligns.join(", "));
    }
    let _ = writeln!(out, "    inset: {TABLE_INSET_PT}pt * mm-fit-u,");
    let _ = writeln!(out, "    stroke: {TABLE_STROKE_PT}pt * mm-fit-u,");
    out.push_str("    table.header(\n");
    for cell in header {
        out.push_str("      [");
        render_inlines(cell, settings, labels, out);
        out.push_str("],\n");
    }
    out.push_str("    ),\n");
    for row in rows {
        for cell in row {
            out.push_str("      [");
            render_inlines(cell, settings, labels, out);
            out.push_str("],\n");
        }
    }
    // Cierra la tabla, el bloque de contenido y el layout.
    out.push_str("    )\n  ]\n})\n\n");
}

/// Expresión de `measure` del contenido íntegro de una celda, forzado a la
/// tipografía compuesta (el natural de la columna).
fn cell_measure(cell: &[Inline], settings: &Settings, labels: &BTreeSet<String>) -> String {
    let mut markup = String::new();
    render_inlines(cell, settings, labels, &mut markup);
    format!("measure([\n#set text(size: mm-fit-base * mm-fit-u)\n{markup}]).width")
}

fn cell_runs(cell: &[Inline], settings: &Settings, labels: &BTreeSet<String>) -> Runs {
    let mut runs = Runs::default();
    walk(cell, settings, labels, &[], &mut runs);
    runs
}

/// Parte `inlines` en runs de `column` conservando el markup de estilo: las
/// piezas sin quiebre entre ellas se acumulan en un mismo run (una palabra
/// con énfasis pegado al texto es un átomo inseparable y sus fragmentos se
/// miden juntos), y solo el espacio/tab ASCII y los quiebres de línea abren
/// un run nuevo.
fn walk(
    inlines: &[Inline],
    settings: &Settings,
    labels: &BTreeSet<String>,
    wraps: &[Wrap],
    runs: &mut Runs,
) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_words(runs, text, |word| wrap(wraps, &escape_text(word))),
            Inline::Code(code) => {
                // `raw` hereda el cuerpo (0.8em): escala con la fuente.
                let markup = format!("#raw({})", typst_string(code));
                runs.piece(&wrap(wraps, &markup));
            }
            Inline::Math(latex) => {
                let markup = match minmark_renderer::validated_math(latex) {
                    Ok(translation) => format!("${translation}$"),
                    Err(marker) => escape_text(&marker),
                };
                runs.piece(&wrap(wraps, &markup));
            }
            Inline::FootnoteReference(label) => {
                // El marcador resuelto como superíndice con su número: la
                // forma sintética `#footnote(...)` no garantiza mostrar N.
                let (number, _) = label.split_once('-').unwrap_or((label, "1"));
                runs.piece(&wrap(wraps, &format!("#super[{number}]")));
            }
            Inline::Image {
                destination,
                alt,
                attrs,
                ..
            } => {
                // El ancho de la imagen no escala con el cuerpo: pieza fija
                // del run. Un `%` es conservador contra el contenedor local.
                if let Some(percent) = attrs.as_ref().and_then(|a| a.width_percent) {
                    runs.fixed(format!("{percent}% * size.width"));
                } else {
                    runs.fixed(format!(
                        "measure([\n#set text(size: mm-fit-base)\n#image({}, alt: {})]).width",
                        typst_string(destination),
                        typst_string(&plain_text(alt))
                    ));
                }
                // El pie va tras `#parbreak()`: quiebre; sus palabras a
                // 0.9em sí escalan y pueden dominar la columna.
                if let Some(caption) = attrs.as_ref().and_then(|a| a.caption.as_deref()) {
                    runs.brk();
                    push_words(runs, caption, |word| {
                        wrap(wraps, &format!("#text(size: 0.9em)[{}]", escape_text(word)))
                    });
                    runs.brk();
                }
            }
            Inline::Color {
                foreground,
                background,
                text,
            } => {
                let color = |word: &str| {
                    let mut markup = escape_text(word);
                    if let Some(bg) = background {
                        markup = format!(
                            "#highlight(fill: rgb(\"{}\"))[{markup}]",
                            color_hex(bg, settings)
                        );
                    }
                    if let Some(fg) = foreground {
                        markup = format!(
                            "#text(fill: rgb(\"{}\"))[{markup}]",
                            color_hex(fg, settings)
                        );
                    }
                    wrap(wraps, &markup)
                };
                push_words(runs, text, color);
            }
            Inline::Citation { items } => {
                runs.piece(&wrap(wraps, &escape_text(&citation_text(items))));
            }
            Inline::WikiLink { target } => {
                runs.piece(&wrap(wraps, &escape_text(&format!("[[{target}]]"))));
            }
            Inline::Substitution(sub) => {
                let markup = match sub {
                    Substitution::Toc => {
                        format!("#outline(title: none, depth: {})", settings.toc_max_level)
                    }
                    Substitution::PageBreak => "#pagebreak()".to_owned(),
                    Substitution::CrossRef { id } => escape_text(&format!("{{{{ref:{id}}}}}")),
                };
                runs.piece(&wrap(wraps, &markup));
            }
            Inline::TaskListMarker(checked) => {
                // En la celda real el marcador es literal; medir el texto
                // escapado sobrestima el «x»/«  » sin riesgo.
                let literal = if *checked { "[x]" } else { "[ ]" };
                runs.piece(&wrap(wraps, &escape_text(literal)));
            }
            Inline::SoftBreak | Inline::HardBreak => runs.brk(),
            Inline::Emphasis(content) => {
                walk(content, settings, labels, &with(wraps, "_", "_"), runs)
            }
            Inline::Strong(content) => {
                walk(content, settings, labels, &with(wraps, "*", "*"), runs)
            }
            Inline::Strikethrough(content) => walk(
                content,
                settings,
                labels,
                &with(wraps, "#strike[", "]"),
                runs,
            ),
            Inline::Link {
                destination,
                content,
                ..
            } => {
                let open = if let Some(id) = destination
                    .strip_prefix('#')
                    .filter(|id| labels.contains(*id))
                {
                    format!("#link(label({}))[", typst_string(id))
                } else {
                    format!("#link({})[", typst_string(destination))
                };
                walk(content, settings, labels, &with(wraps, &open, "]"), runs);
            }
            Inline::Styled { content, style } => {
                let mut next = wraps.to_vec();
                if style.italic {
                    next.push(("#text(style: \"italic\")[".into(), "]".into()));
                }
                if style.bold {
                    next.push(("#text(weight: \"bold\")[".into(), "]".into()));
                }
                if style.light {
                    next.push(("#text(weight: \"light\")[".into(), "]".into()));
                }
                if style.small_caps {
                    next.push(("#smallcaps[".into(), "]".into()));
                }
                if style.underline {
                    next.push(("#underline[".into(), "]".into()));
                }
                if style.vertical == 1 {
                    next.push(("#super[".into(), "]".into()));
                }
                if style.vertical == -1 {
                    next.push(("#sub[".into(), "]".into()));
                }
                walk(content, settings, labels, &next, runs);
            }
        }
    }
}

fn with(wraps: &[Wrap], open: &str, close: &str) -> Vec<Wrap> {
    let mut next = wraps.to_vec();
    next.push((open.to_owned(), close.to_owned()));
    next
}

fn wrap(wraps: &[Wrap], inner: &str) -> String {
    let mut out = String::new();
    for (open, _) in wraps {
        out.push_str(open);
    }
    out.push_str(inner);
    for (_, close) in wraps.iter().rev() {
        out.push_str(close);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile_document, DocumentMeta, PdfEngine, PdfError, TypstEngine};
    use minmark_config::Orientation;
    use minmark_parser::parse;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use typst::layout::{FrameItem, PagedDocument};

    /// §33.2 cuerpo por defecto y §35.2 suelo por defecto (pt).
    const CUERPO: f64 = 12.0;
    const SUELO: f64 = 8.0;

    const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" \
                       viewBox=\"0 0 40 20\"><rect width=\"40\" height=\"20\" \
                       fill=\"#999999\"/></svg>";

    fn emitir(md: &str, settings: &Settings) -> String {
        crate::generate(&parse(md), settings, &DocumentMeta::default())
    }

    fn compilar(md: &str, settings: &Settings) -> Result<PagedDocument, PdfError> {
        compilar_con(md, settings, BTreeMap::new())
    }

    fn compilar_con(
        md: &str,
        settings: &Settings,
        assets: BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<PagedDocument, PdfError> {
        let ast = parse(md);
        compile_document(&ast, settings, assets, &DocumentMeta::default())
    }

    /// Fila de texto del documento con su tamaño (pt) y su borde derecho
    /// absoluto (pt) para comprobar que nada se sale del área de texto.
    #[derive(Debug)]
    struct Texto {
        texto: String,
        tamano: f64,
        derecha: f64,
    }

    fn textos(doc: &PagedDocument) -> Vec<Texto> {
        fn recorrer(frame: &typst::layout::Frame, x: f64, out: &mut Vec<Texto>) {
            for (pos, item) in frame.items() {
                match item {
                    FrameItem::Text(t) => out.push(Texto {
                        texto: t.text.to_string(),
                        tamano: t.size.to_pt(),
                        derecha: x + pos.x.to_pt() + t.width().to_pt(),
                    }),
                    FrameItem::Group(g) => recorrer(&g.frame, x + pos.x.to_pt(), out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for page in &doc.pages {
            recorrer(&page.frame, 0.0, &mut out);
        }
        out
    }

    fn minimo(textos: &[Texto]) -> f64 {
        textos.iter().map(|t| t.tamano).fold(f64::MAX, f64::min)
    }

    /// Borde derecho del área de texto A4 con márgenes de 25 mm + holgura:
    /// una tabla que no respeta su contenedor se sale de aquí.
    fn limite_derecho() -> f64 {
        (210.0 - 25.0) * 72.0 / 25.4 + 3.0
    }

    fn assert_dentro(textos: &[Texto]) {
        let fuera = textos
            .iter()
            .filter(|t| t.derecha > limite_derecho())
            .collect::<Vec<_>>();
        assert!(fuera.is_empty(), "texto fuera del contenedor: {fuera:?}");
    }

    /// Tabla de una celda con una palabra sin quiebres de `n` caracteres: su
    /// ancho natural (~8 pt por carácter «X» a 12 pt en Liberation Sans) es
    /// ~`8·n` pt frente a los 453.54 pt del contenedor A4 con márgenes 25 mm.
    fn tabla_con_palabra(n: usize) -> String {
        format!("| A |\n| --- |\n| {} |\n", "X".repeat(n))
    }

    /// [1] Una tabla normal se compone sin reducción (mm-fit-u = 1 en
    /// runtime) y su caption renderizado sigue debajo, fuera del envoltorio.
    #[test]
    fn tabla_normal_conserva_el_cuerpo_y_caption_debajo() {
        let md = "| A | B |\n| --- | --- |\n| uno | dos |\n{#tbl:normal \"Resultados\"}\n";
        let out = emitir(md, &Settings::default());
        let doc = compilar(md, &Settings::default()).expect("compila");
        let textos = textos(&doc);
        assert!(!textos.is_empty(), "sin texto compuesto");
        for texto in &textos {
            assert!(
                (texto.tamano - CUERPO).abs() < 0.01,
                "la tabla que cabe no altera el cuerpo: {texto:?}"
            );
        }
        let tabla = out.find("#table(").expect("tabla");
        let caption = out
            .find("#align(center)[Tabla 1. Resultados]")
            .expect("caption");
        assert!(tabla < caption, "caption debajo de la tabla:\n{out}");
    }

    /// [2] Una tabla de prosa cuyo ancho natural supera el contenedor se
    /// compone a cuerpo porque la envoltura basta (regresión de no-envolver):
    /// nada sobrepasa el área de texto y ninguna tipografía se reduce.
    #[test]
    fn tabla_de_prosa_mas_ancha_que_el_contenedor_envuelve_a_cuerpo() {
        let celda0 = "Esta celda lleva una prosa bastante larga con muchas palabras seguidas \
                      que nunca cabría entera en una columna estrecha del contenedor sin \
                      partirse en varias líneas de composición";
        let celda1 = "Y esta otra celda también trae una prosa larguísima con palabras \
                      suficientes para superar con creces el ancho disponible del contenedor \
                      y forzar varios cortes de línea en la celda";
        assert!(celda0.len() > 100 && celda1.len() > 100);
        let md = format!("| A | B |\n| --- | --- |\n| {celda0} | {celda1} |\n");
        let doc =
            compilar(&md, &Settings::default()).expect("la prosa se envuelve y la tabla compila");
        let textos = textos(&doc);
        for texto in &textos {
            assert!(
                (texto.tamano - CUERPO).abs() < 0.01,
                "envolver no debe reducir el cuerpo: {texto:?}"
            );
        }
        assert_dentro(&textos);
    }

    /// [3] Una palabra sin quiebres desborda el cuerpo pero cabe sobre el
    /// suelo: la fuente emitida se reduce, nunca baja de `table_min_font_pt`
    /// y la tabla compuesta NO se sale del área de texto (bounds renderizados,
    /// no solo el tamaño).
    #[test]
    fn tabla_infinita_se_reducia_sobre_el_suelo() {
        let md = tabla_con_palabra(70);
        let doc = compilar(&md, &Settings::default()).expect("compila reducida");
        let textos = textos(&doc);
        let min = minimo(&textos);
        assert!(min < CUERPO, "debe reducirse por debajo del cuerpo: {min}");
        assert!(min >= SUELO, "nunca por debajo de table_min_font_pt: {min}");
        assert_dentro(&textos);
    }

    /// [4] Sobre el suelo la exportación falla con `PdfError::Compose` cuyo
    /// mensaje identifica la tabla y el suelo (§35.2; contrato de diálogo:
    /// `error de composición PDF: panicked with: "..."`).
    #[test]
    fn error_de_suelo_identifica_la_tabla_35_2() {
        let md = format!(
            "| A |\n| --- |\n| {} |\n{{#tbl:overflow \"Resultados\"}}\n",
            "X".repeat(120)
        );
        let err = TypstEngine
            .export(&parse(&md), &Settings::default(), &DocumentMeta::default())
            .expect_err("la tabla no cabe ni reducida al suelo");
        let msg = err.to_string();
        assert!(
            msg.starts_with("error de composición PDF: panicked with:"),
            "{msg}"
        );
        assert!(
            msg.contains(
                "\"Tabla 1. Resultados: no cabe en el ancho disponible \
                 con table_min_font_pt=8pt (§35.2)\""
            ),
            "{msg}"
        );
    }

    /// [5] `table_min_font_pt` es el que decide: la misma tabla cabe con el
    /// suelo de 8 pt y falla con 30 pt (solo cambia ese ajuste).
    #[test]
    fn table_min_font_pt_condiciona_el_resultado() {
        let md = tabla_con_palabra(70);
        compilar(&md, &Settings::default()).expect("con suelo 8pt la reducción cabe");
        let estricto = Settings {
            table_min_font_pt: 30.0,
            ..Settings::default()
        };
        let err = compilar(&md, &estricto).expect_err("con suelo 30pt no cabe");
        let msg = err.to_string();
        assert!(
            msg.starts_with("error de composición PDF: panicked with:"),
            "{msg}"
        );
        assert!(
            msg.contains(
                "tabla sin ID nº 1: no cabe en el ancho disponible \
                 con table_min_font_pt=30pt (§35.2)"
            ),
            "{msg}"
        );
    }

    /// [6] Una tabla larga dentro de `#layout` sigue repartiéndose en varias
    /// páginas y su caption largo (§28) se compone completo una sola vez,
    /// envuelto y fuera del envoltorio.
    #[test]
    fn tabla_larga_con_caption_largo_se_reparte_fuera_del_layout() {
        let caption = "Resumen ejecutivo del experimento con todas las conclusiones \
                       pendientes de revisión por pares y anexos documentales";
        assert!(caption.len() > 100, "el caption debe envolver");
        let mut md = String::from("| A | B |\n| --- | --- |\n");
        for i in 0..80 {
            md.push_str(&format!("| fila {i} | valor {i} |\n"));
        }
        md.push_str(&format!("{{#tbl:large \"{caption}\"}}\n"));
        let out = emitir(&md, &Settings::default());
        let doc = compilar(&md, &Settings::default()).expect("compila");
        assert!(
            doc.pages.len() >= 2,
            "el layout no debe bloquear el reparto: {} páginas",
            doc.pages.len()
        );
        let pies = out.matches("#align(center)[Tabla 1.").count();
        assert_eq!(pies, 1, "el caption se compone una vez:\n{out}");
        let layout = out.rfind("#layout(").expect("layout");
        let cierre = layout + out[layout..].find("})").expect("cierre");
        let pie = out.find("#align(center)[Tabla 1.").expect("caption");
        assert!(cierre < pie, "el caption va fuera del layout:\n{out}");
        // El texto del caption se compone COMPLETO (envuelto en varias líneas).
        let compuesto: String = textos(&doc)
            .iter()
            .map(|t| t.texto.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let normalizar = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let esperado = normalizar(&format!("Tabla 1. {caption}"));
        assert!(
            normalizar(&compuesto).contains(&esperado),
            "caption completo compuesto:\n{compuesto}"
        );
    }

    /// [7] El estilo del ajuste no se escapa del envoltorio: la prosa
    /// posterior y el pie de la figura vecina conservan su tamaño (el pie es
    /// 0.9em del cuerpo) mientras la tabla sí se reduce.
    #[test]
    fn prosa_y_figura_adyacente_conservan_el_cuerpo() {
        let md = format!(
            "{}\nPárrafo posterior con palabras normales.\n\n![Esquema](esquema.svg){{#fig:one}}\n",
            tabla_con_palabra(70)
        );
        let mut assets = BTreeMap::new();
        assets.insert(PathBuf::from("esquema.svg"), SVG.as_bytes().to_vec());
        let doc = compilar_con(&md, &Settings::default(), assets).expect("compila");
        let textos = textos(&doc);
        let prosa = textos
            .iter()
            .find(|t| t.texto.contains("posterior"))
            .expect("prosa posterior");
        assert!(
            (prosa.tamano - CUERPO).abs() < 0.01,
            "la prosa posterior conserva el cuerpo: {prosa:?}"
        );
        let pie = textos
            .iter()
            .find(|t| t.texto.contains("Figura 1"))
            .expect("pie de figura");
        assert!(
            (pie.tamano - CUERPO * 0.9).abs() < 0.01,
            "el pie de la figura no se altera: {pie:?}"
        );
        let palabra = textos
            .iter()
            .find(|t| t.texto.starts_with('X'))
            .expect("celda de la tabla");
        assert!(
            (SUELO..CUERPO).contains(&palabra.tamano),
            "la tabla sí se reduce: {palabra:?}"
        );
        assert_dentro(&textos);
    }

    /// [8] La geometría del contenedor cambia la decisión: la misma tabla no
    /// cabe a cuerpo en A4 Portrait (453.54 pt) pero sí en Landscape (247 mm
    /// ≈ 700.8 pt), donde se emite sin reducción.
    #[test]
    fn la_geometria_del_contenedor_cambia_la_decision() {
        let md = tabla_con_palabra(70);
        let retrato = compilar(&md, &Settings::default()).expect("retrato compila");
        let reducido = minimo(&textos(&retrato));
        assert!(
            reducido < CUERPO,
            "en retrato la tabla no cabe a cuerpo: {reducido}"
        );
        assert!(reducido >= SUELO, "sin bajar del suelo: {reducido}");
        let paisaje = Settings {
            pdf_orientation: Orientation::Landscape,
            ..Settings::default()
        };
        let doc = compilar(&md, &paisaje).expect("paisaje compila");
        let textos = textos(&doc);
        for texto in &textos {
            assert!(
                (texto.tamano - CUERPO).abs() < 0.01,
                "en paisaje la tabla cabe a cuerpo: {texto:?}"
            );
        }
    }

    /// La identidad de §35.2: caption si existe, si no `#id`, si no
    /// `tabla sin ID nº <n>` con el nº 1-based de orden documental (recorriendo
    /// también bloques anidados).
    #[test]
    fn identidad_de_tablas_por_caption_id_y_orden() {
        let ast = parse(
            "| A |\n| - |\n| B |\n{#tbl:one \"Resultados\"}\n\n\
             | C |\n| - |\n| D |\n{#tbl:two}\n\n\
             | E |\n| - |\n| F |\n",
        );
        let identidades = table_identities(&ast);
        // BTreeMap recorre por `span.start`: orden documental.
        let valores: Vec<&str> = identidades.values().map(String::as_str).collect();
        assert_eq!(
            valores,
            ["Resultados", "#tbl:two", "tabla sin ID nº 3"],
            "caption > #id > nº de orden"
        );
    }

    /// (i) Un run continuo que cruza fronteras de inline sin espacios
    /// (`texto + énfasis + texto`) es UN átomo inseparable: se mide entero,
    /// se reduce sobre el suelo y no se sale del área.
    #[test]
    fn run_que_cruza_fronteras_de_inline_se_mide_entero() {
        let n = "X".repeat(25);
        let md = format!("| A |\n| --- |\n| {n}*{n}*{n} |\n");
        // El parser rinde texto + énfasis + texto (fixture validado).
        let ast = parse(&md);
        let minmark_document::BlockKind::Table { rows, .. } = &ast.blocks[0].kind else {
            panic!("esperaba tabla");
        };
        assert_eq!(
            rows[0][0].len(),
            3,
            "texto, énfasis y texto: {:?}",
            rows[0][0]
        );
        let doc = compilar(&md, &Settings::default()).expect("compila");
        let textos = textos(&doc);
        let min = minimo(&textos);
        assert!(
            (SUELO..CUERPO).contains(&min),
            "el run de 75 X no cabe a cuerpo y se reduce sobre el suelo: {min}"
        );
        assert_dentro(&textos);
    }

    /// (ii) U+00A0 entre palabras no es oportunidad de quiebre: el run de
    /// 60 X + NNBSP se mide entero (se reduce); partirlo en dos runs de 30
    /// no reduciría y dejaría 480 pt desbordados.
    #[test]
    fn nbsp_no_es_oportunidad_de_quiebre() {
        let md = format!(
            "| A |\n| --- |\n| {}\u{00A0}{} |\n",
            "X".repeat(30),
            "X".repeat(30)
        );
        let doc = compilar(&md, &Settings::default()).expect("compila");
        let textos = textos(&doc);
        let min = minimo(&textos);
        assert!(
            min < CUERPO,
            "el run con NNBSP es inseparable y debe reducir: {min}"
        );
        assert!(min >= SUELO, "sin bajar del suelo: {min}");
        assert_dentro(&textos);
    }

    /// (iii) Prosa en la columna 1 + token ancho inseparable en la 2: el
    /// reparto respeta las mínimas calculadas y ningún bounds renderizado
    /// sale del área (el reparto justo de `auto` solo daría ~226 pt al
    /// token de 600 pt y se desbordaría).
    #[test]
    fn prosa_mas_token_sin_desborde_en_bounds() {
        let prosa = "esta celda de prosa envuelve en varias lineas con palabras cortas \
                     que reparten la columna";
        let md = format!(
            "| Prosas | Token |\n| --- | --- |\n| {prosa} | {} |\n",
            "X".repeat(75)
        );
        let doc = compilar(&md, &Settings::default()).expect("compila");
        let textos = textos(&doc);
        let min = minimo(&textos);
        assert!(
            (SUELO..CUERPO).contains(&min),
            "el token de 600 pt exige reducir sobre el suelo: {min}"
        );
        assert_dentro(&textos);
    }

    /// (iv) Una tabla dentro de una definición de nota se compone en el
    /// contexto de la nota (otra medida ambiente): las medidas van forzadas
    /// al tamaño base en la fuente y el resultado no se sale del área.
    #[test]
    fn tabla_en_nota_al_pie_se_mide_a_tamano_base() {
        let md = format!(
            "Texto con nota[^1].\n\n[^1]: Nota con tabla:\n    | A |\n    | --- |\n    | {} |\n",
            "X".repeat(70)
        );
        // La tabla vive DENTRO de la definición (contexto de nota, no el
        // cuerpo del documento): el fixture no puede ser vacío.
        let ast = minmark_parser::parse(&md);
        let def = ast
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::FootnoteDefinition { .. }))
            .expect("definición de nota");
        let BlockKind::FootnoteDefinition { blocks, .. } = &def.kind else {
            panic!("esperaba definición");
        };
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Table { .. })),
            "la tabla debe vivir dentro de la definición: {blocks:?}"
        );
        let out = emitir(md.as_str(), &Settings::default());
        // La medida de los runs lleva el `set` base dentro del contenido.
        assert!(
            out.contains("measure([\n#set text(size: mm-fit-base)\n"),
            "las medidas van forzadas al cuerpo:\n{out}"
        );
        let doc = compilar(&md, &Settings::default()).expect("compila");
        let textos = textos(&doc);
        let palabra = textos
            .iter()
            .find(|t| t.texto.starts_with('X'))
            .expect("celda dentro de la nota");
        assert!(
            (SUELO..CUERPO).contains(&palabra.tamano),
            "la tabla de la nota se reduce sin bajar del suelo: {palabra:?}"
        );
        assert_dentro(&textos);
    }

    /// (v) `resolve_footnotes` mueve las definiciones al final: los ordinales
    /// de identidad siguen el orden de `span.start` (la tabla de la
    /// definición, más temprana en el fuente, es la nº 1 aunque se recorra
    /// la última).
    #[test]
    fn identidad_estable_tras_reordenar_notas() {
        let md = "[^1]: Nota con tabla:\n    | D |\n    | --- |\n    | Dentro |\n\n\
                  Después[^1].\n\n\
                  | B |\n| --- |\n| Cuerpo |\n";
        let ast = minmark_parser::parse(md);
        let (resuelto, _) = minmark_renderer::resolve_footnotes(&ast);
        let identidades = table_identities(&resuelto);
        let valores: Vec<&str> = identidades.values().map(String::as_str).collect();
        assert_eq!(
            valores,
            ["tabla sin ID nº 1", "tabla sin ID nº 2"],
            "los ordinales siguen span.start tras reordenar: {identidades:?}"
        );
    }
}
