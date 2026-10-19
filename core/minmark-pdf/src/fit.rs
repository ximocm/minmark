//! Ajuste proporcional de tablas en la exportación PDF (§35.2, ISSUE-027).
//!
//! §35.2: «La tabla se reducirá proporcionalmente hasta un mínimo de 8 pt.
//! Si aun así no cabe, la exportación se detendrá con un error que
//! identifique la tabla problemática. No se recortará.» El mínimo es el
//! ajuste `table_min_font_pt` (defecto 8 pt) y el cuerpo es
//! [`Settings::font_size_pt`].
//!
//! Cada tabla se emite **nativa** dentro de `#layout(size => ...)`: allí se
//! miden los átomos (runs sin quiebre) de cada columna a tamaño base y se
//! elige en runtime el tamaño de fuente `f` con
//! `table_min_font_pt ≤ f ≤ font_size_pt` tal que el ancho mínimo de la
//! tabla cabe en el contenedor local. Si ni el suelo cabe, `panic` detiene la
//! exportación identificando la tabla. La tabla no se envuelve en `box` ni
//! `scale`: el reparto multipágina se conserva y el caption (§28) se emite
//! fuera del envoltorio, sin reducirse.
//!
//! Modelo del ancho mínimo (nunca se infravalora; en duda el ancho va al
//! término fijo B o se sobreestima A): `minw(f) = A·f + B`, con
//! - `A = (Σ máximos escalables por columna + n·2·inset + (n+1)·stroke) / base`:
//!   texto/math/código, inset y stroke escalan con el tamaño de fuente;
//! - `B = Σ máximos fijos por columna`: imágenes (anchos absolutos o `%` del
//!   ancho disponible) y otras anchuras independientes del cuerpo.

use crate::generate::{
    citation_text, color_hex, escape_text, plain_text, render_inlines, typst_string,
};
use minmark_config::Settings;
use minmark_document::{Align, Ast, Block, BlockKind, Inline, Substitution};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// Inset por defecto de `table` en Typst (pt a tamaño base). Se emite
/// explícito para que con `mm-fit-u = 1` el aspecto sea el de hoy.
const TABLE_INSET_PT: usize = 5;
/// Stroke por defecto de `table` en Typst (pt a tamaño base).
const TABLE_STROKE_PT: usize = 1;

/// Envoltorio de estilo `(apertura, cierre)` alrededor de un átomo.
type Wrap = (String, String);

/// Anchos de los átomos por columna: `scaled[j]` se mide a tamaño base y
/// escala con la fuente; `fixed[j]` son expresiones de longitud constantes
/// (o crecientes con `size.width`) evaluadas dentro del envoltorio.
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
}

/// Identidad documental de cada tabla (`span.start` → identidad) para el
/// mensaje de suelo de §35.2: el caption ya expandido (§26.5/§28 por
/// [`minmark_renderer::resolve_cross_references`]), si no el `#id`, y si no
/// `tabla sin ID nº <n>` con el nº 1-based de orden documental de tablas.
pub(crate) fn table_identities(ast: &Ast) -> BTreeMap<u32, String> {
    fn walk(blocks: &[Block], order: &mut u32, out: &mut BTreeMap<u32, String>) {
        for block in blocks {
            match &block.kind {
                BlockKind::Table { attrs, .. } => {
                    *order += 1;
                    let attrs = attrs.as_ref();
                    let identity = attrs
                        .and_then(|a| a.caption.as_ref())
                        .map(String::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            attrs
                                .and_then(|a| a.id.as_deref())
                                .map(|id| format!("#{id}"))
                        })
                        .unwrap_or_else(|| format!("tabla sin ID nº {order}"));
                    out.insert(block.span.start, identity);
                }
                BlockKind::Quote { blocks } | BlockKind::FootnoteDefinition { blocks, .. } => {
                    walk(blocks, order, out)
                }
                BlockKind::Callout { blocks, .. } => walk(blocks, order, out),
                BlockKind::List { items, .. } => {
                    for item in items {
                        walk(&item.blocks, order, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&ast.blocks, &mut 0, &mut out);
    out
}

/// Emite la tabla envuelta en el ajuste de §35.2. Las celdas se renderizan
/// igual que antes (`render_inlines`); lo único nuevo es el envoltorio con
/// las medidas, la decisión de tamaño y el `panic` de suelo.
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
    // Los átomos se agrupan por columna como los reparte Typst: la cabecera
    // llena su fila (fila a fila) y el cuerpo, plano, empieza en la columna 0.
    for (index, cell) in header.iter().enumerate() {
        walk(cell, settings, labels, &[], index % columns, &mut cols);
    }
    for (body_index, cell) in rows.iter().flatten().enumerate() {
        walk(cell, settings, labels, &[], body_index % columns, &mut cols);
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

    let _ = writeln!(out, "#layout(size => {{");
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
    let _ = writeln!(out, "  let mm-fit-base = {base}pt");
    let _ = writeln!(out, "  let mm-fit-floor = {floor}pt");
    out.push_str("  let mm-fit-u = if mm-fit-P + mm-fit-B <= size.width {\n");
    out.push_str("    1.0\n");
    out.push_str("  } else if mm-fit-P * (mm-fit-floor / mm-fit-base) + mm-fit-B > size.width {\n");
    let _ = writeln!(out, "    panic({})", typst_string(&message));
    out.push_str("  } else {\n");
    out.push_str("    (size.width - mm-fit-B) / mm-fit-P\n");
    out.push_str("  }\n");
    // El tamaño nunca supera mm-fit-base: la tabla ya ajustada no se agranda.
    out.push_str("  set text(size: mm-fit-base * mm-fit-u)\n");

    out.push_str("  [#table(\n");
    let _ = writeln!(out, "  columns: {columns},");
    if !aligns.is_empty() {
        let aligns: Vec<&str> = aligns
            .iter()
            .map(|a| match a {
                Align::Center => "center",
                Align::Right => "right",
                Align::Left | Align::None => "left",
            })
            .collect();
        let _ = writeln!(out, "  align: ({},),", aligns.join(", "));
    }
    let _ = writeln!(out, "  inset: {TABLE_INSET_PT}pt * mm-fit-u,");
    let _ = writeln!(out, "  stroke: {TABLE_STROKE_PT}pt * mm-fit-u,");
    out.push_str("  table.header(\n");
    for cell in header {
        out.push_str("    [");
        render_inlines(cell, settings, labels, out);
        out.push_str("],\n");
    }
    out.push_str("  ),\n");
    for row in rows {
        for cell in row {
            out.push_str("    [");
            render_inlines(cell, settings, labels, out);
            out.push_str("],\n");
        }
    }
    // Cierra la tabla `)`, el bloque de contenido `]` y el layout `})`.
    out.push_str(")]\n})\n\n");
}

/// Parte `inlines` en átomos (runs sin quiebre) de `column` conservando el
/// markup de estilo alrededor de cada uno: una palabra en negrita es más
/// ancha que la normal y `measure` debe verla igual que la celda real.
fn walk(
    inlines: &[Inline],
    settings: &Settings,
    labels: &BTreeSet<String>,
    wraps: &[Wrap],
    column: usize,
    cols: &mut Columns,
) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                // Los espacios parten los átomos; el ancho mínimo de prosa
                // es la palabra más ancha (los quiebres descartan el espacio).
                for word in text.split_whitespace() {
                    push_scaled(cols, column, &wrap(wraps, &escape_text(word)));
                }
            }
            Inline::Code(code) => {
                // `raw` hereda el cuerpo (0.8em): escala con la fuente.
                let markup = format!("#raw({})", typst_string(code));
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::Math(latex) => {
                let markup = match minmark_renderer::validated_math(latex) {
                    Ok(translation) => format!("${translation}$"),
                    Err(marker) => escape_text(&marker),
                };
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::FootnoteReference(label) => {
                // El marcador mide lo mismo por su forma directa; la forma de
                // referencia `(<label>)` mide 0 pt fuera del documento.
                let (number, _) = label.split_once('-').unwrap_or((label, "1"));
                let markup = format!("#footnote(mm-note-{number})");
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::Image {
                destination,
                alt,
                attrs,
                ..
            } => {
                // El ancho de la imagen no escala con el cuerpo: va a B. Un
                // `%` es conservador contra el ancho del contenedor local.
                if let Some(percent) = attrs.as_ref().and_then(|a| a.width_percent) {
                    push_fixed(cols, column, format!("{percent}% * size.width"));
                } else {
                    push_fixed(
                        cols,
                        column,
                        format!(
                            "measure([#image({}, alt: {})]).width",
                            typst_string(destination),
                            typst_string(&plain_text(alt))
                        ),
                    );
                }
                // El pie de la imagen se compone dentro de la celda a 0.9em:
                // sus palabras sí escalan y pueden dominar la columna.
                if let Some(caption) = attrs.as_ref().and_then(|a| a.caption.as_deref()) {
                    for word in caption.split_whitespace() {
                        let markup = format!("#text(size: 0.9em)[{}]", escape_text(word));
                        push_scaled(cols, column, &wrap(wraps, &markup));
                    }
                }
            }
            Inline::Color {
                foreground,
                background,
                text,
            } => {
                for word in text.split_whitespace() {
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
                    push_scaled(cols, column, &wrap(wraps, &markup));
                }
            }
            Inline::Citation { items } => {
                let markup = escape_text(&citation_text(items));
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::WikiLink { target } => {
                let markup = escape_text(&format!("[[{target}]]"));
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::Substitution(sub) => {
                let markup = match sub {
                    Substitution::Toc => {
                        format!("#outline(title: none, depth: {})", settings.toc_max_level)
                    }
                    Substitution::PageBreak => "#pagebreak()".to_owned(),
                    Substitution::CrossRef { id } => escape_text(&format!("{{{{ref:{id}}}}}")),
                };
                push_scaled(cols, column, &wrap(wraps, &markup));
            }
            Inline::TaskListMarker(checked) => {
                // En la celda real el marcador es literal; medir el texto
                // escapado sobrestima el «x»/«  » sin riesgo.
                let literal = if *checked { "[x]" } else { "[ ]" };
                push_scaled(cols, column, &wrap(wraps, &escape_text(literal)));
            }
            // Los quiebres de línea terminan átomos: las palabras ya parten.
            Inline::SoftBreak | Inline::HardBreak => {}
            Inline::Emphasis(content) => walk(
                content,
                settings,
                labels,
                &with(wraps, "_", "_"),
                column,
                cols,
            ),
            Inline::Strong(content) => walk(
                content,
                settings,
                labels,
                &with(wraps, "*", "*"),
                column,
                cols,
            ),
            Inline::Strikethrough(content) => walk(
                content,
                settings,
                labels,
                &with(wraps, "#strike[", "]"),
                column,
                cols,
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
                walk(
                    content,
                    settings,
                    labels,
                    &with(wraps, &open, "]"),
                    column,
                    cols,
                );
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
                walk(content, settings, labels, &next, column, cols);
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

fn push_scaled(cols: &mut Columns, column: usize, markup: &str) {
    cols.scaled[column].push(format!("measure([{markup}]).width"));
}

fn push_fixed(cols: &mut Columns, column: usize, expr: String) {
    cols.fixed[column].push(expr);
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

    /// Tabla de una celda con una palabra sin quiebres de `n` caracteres: su
    /// ancho natural (~8 pt por carácter «X» a 12 pt en Liberation Sans) es
    /// ~`8·n` pt frente a los 453.54 pt del contenedor A4 con márgenes 25 mm.
    fn tabla_con_palabra(n: usize) -> String {
        format!("| A |\n| --- |\n| {} |\n", "X".repeat(n))
    }

    /// [1] Una tabla normal se compone sin reducción (mm-fit-u = 1 en runtime)
    /// y su caption renderizado sigue debajo, fuera del envoltorio.
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
        // Borde derecho del área de texto A4 con márgenes de 25 mm + holgura:
        // una tabla sin envolver (natural ≫ 453 pt) se saldría de aquí.
        let limite = (210.0 - 25.0) * 72.0 / 25.4 + 3.0;
        let fuera = textos
            .iter()
            .filter(|t| t.derecha > limite)
            .collect::<Vec<_>>();
        assert!(fuera.is_empty(), "texto fuera del contenedor: {fuera:?}");
    }

    /// [3] Una palabra sin quiebres desborda el cuerpo pero cabe sobre el
    /// suelo: la fuente emitida se reduce y nunca baja de `table_min_font_pt`.
    #[test]
    fn tabla_infinita_se_reducia_sobre_el_suelo() {
        let md = tabla_con_palabra(70);
        let doc = compilar(&md, &Settings::default()).expect("compila reducida");
        let textos = textos(&doc);
        let min = minimo(&textos);
        assert!(min < CUERPO, "debe reducirse por debajo del cuerpo: {min}");
        assert!(min >= SUELO, "nunca por debajo de table_min_font_pt: {min}");
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
    /// páginas y su caption largo (§28) se compone una sola vez, envuelto y
    /// fuera del envoltorio.
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
    }

    /// [7] El `set text` del ajuste no se escapa del envoltorio: la prosa
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
            palabra.tamano < CUERPO && palabra.tamano >= SUELO,
            "la tabla sí se reduce: {palabra:?}"
        );
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
}
