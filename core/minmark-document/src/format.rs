//! Acciones editoriales contextuales sobre el fuente Markdown (§44, fase 7).
//!
//! Controles editoriales opcionales sobre el MISMO buffer (§6.1): toggle de
//! estilos inline, niveles de heading (H1–H6), enlaces y callouts (§22). Cada
//! acción se planifica como una lista de ediciones disjuntas sobre el fuente
//! original más la selección resultante (§8.6); la UI las aplica con
//! [`crate::Editor::replace_all_selecting`] para que sean UNA transacción de
//! undo/redo (§8.5).
//!
//! Todo es puro sobre `&str`: sin estado, sin dependencias (§4.1). Lo que no
//! se puede expresar como edición limpia (sintaxis no soportada §8.4, rango
//! inválido, no-op) devuelve `None` y el documento queda intacto.

use crate::document::LineEnding;
use crate::position::{self, Range, Selection};

/// Acción editorial contextual (§44; issues #29–#31).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatAction {
    /// Negrita `**texto**`.
    Strong,
    /// Cursiva `*texto*`.
    Emphasis,
    /// Tachado `~~texto~~`.
    Strikethrough,
    /// Código inline `` `texto` `` (cerco proporcional al contenido).
    Code,
    /// Nivel de heading ATX `#`..`######` (1..=6) sobre los bloques cubiertos.
    Heading(u8),
    /// Enlace `[texto](url)`; con cursor/selección dentro de un enlace
    /// existente solo sustituye la URL.
    Link { url: String },
    /// Callout `::: tipo` (§22); dentro de un callout existente solo cambia
    /// el tipo del opener.
    Callout { callout_type: String },
    /// Insertar imagen `![alt](ruta) {#id width=N%}` (§18, §26.2); con
    /// selección no vacía, el texto seleccionado pasa a ser el alt.
    ImageInsert {
        destination: String,
        alt: String,
        id: Option<String>,
        width_percent: Option<u8>,
    },
    /// Editar la imagen que contiene la selección (§18.5, §26.2): solo los
    /// campos dados; título, caption y `clave=valor` extra intactos.
    ImageEdit {
        destination: String,
        alt: String,
        id: Option<String>,
        width_percent: Option<u8>,
    },
    /// Insertar tabla GFM (§35) con `rows` filas de datos y `cols` columnas.
    TableInsert { rows: usize, cols: usize },
    /// Editar los atributos `{#id "caption"}` de la tabla que contiene la
    /// selección (§26.3, §28.2): la tabla no se toca.
    TableAttrs {
        id: Option<String>,
        caption: Option<String>,
    },
}

/// Plan de una acción: ediciones disjuntas sobre el buffer original (sin
/// solaparse, en cualquier orden) y la selección que debe quedar tras
/// aplicarlas (§8.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatPlan {
    pub edits: Vec<(Range, String)>,
    pub selection: Selection,
}

/// Enlace `[texto](url)` que contiene la selección (para edición contextual).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkContext {
    /// Rango del enlace completo, marcadores incluidos.
    pub range: Range,
    /// Rango del texto entre `[` y `]`.
    pub text_range: Range,
    /// URL actual.
    pub url: String,
}

/// Callout (§22) que contiene la selección.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalloutContext {
    pub open_line: u32,
    pub close_line: u32,
    pub callout_type: String,
    pub title: Option<String>,
}

/// Imagen `![alt](ruta "título") {attrs}` que contiene la selección (§18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageContext {
    /// Rango del constructo completo, atributos incluidos.
    pub range: Range,
    pub destination: String,
    pub alt: String,
    /// Título `"..."` tras la ruta (§18): se conserva siempre.
    pub title: Option<String>,
    pub id: Option<String>,
    pub width_percent: Option<u8>,
}

/// Tabla GFM (§35) que contiene la selección.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableContext {
    /// Rango del bloque de filas (sin la línea de atributos).
    pub range: Range,
    pub columns: usize,
    pub id: Option<String>,
    pub caption: Option<String>,
}

/// Contexto editorial bajo la selección (§44): lo que los controles
/// contextuales muestran como utilidad. No muta nada.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditContext {
    /// Nivel del heading del bloque bajo la selección, si lo hay.
    pub heading_level: Option<u8>,
    pub link: Option<LinkContext>,
    pub callout: Option<CalloutContext>,
    pub image: Option<ImageContext>,
    pub table: Option<TableContext>,
    /// La selección ya está envuelta por el marcador correspondiente.
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
}

/// Edición cruda en offsets de bytes sobre el fuente original.
type RawEdit = (usize, usize, String);
/// Plan crudo: ediciones en offsets y selección resultante (también offsets).
type RawPlan = (Vec<RawEdit>, (usize, usize));

/// Planifica `action` sobre `source` para la `selection` dada.
///
/// `None` = acción no aplicable (§8.4: sintaxis no soportada, rango inválido
/// o no-op): el documento no debe tocarse.
pub fn plan(source: &str, selection: Selection, action: &FormatAction) -> Option<FormatPlan> {
    let starts = position::line_starts(source);
    let range = selection.range();
    let a = position::offset_of(source, &starts, range.start)?;
    let b = position::offset_of(source, &starts, range.end)?;

    let (edits, after): RawPlan = match action {
        FormatAction::Strong => inline_plan(source, a, b, Inline::Strong)?,
        FormatAction::Emphasis => inline_plan(source, a, b, Inline::Emphasis)?,
        FormatAction::Strikethrough => inline_plan(source, a, b, Inline::Strikethrough)?,
        FormatAction::Code => inline_plan(source, a, b, Inline::Code)?,
        FormatAction::Heading(level) => heading_plan(source, &starts, a, b, *level)?,
        FormatAction::Link { url } => link_plan(source, a, b, url)?,
        FormatAction::Callout { callout_type } => callout_plan(source, &starts, a, b, callout_type)?,
        FormatAction::ImageInsert {
            destination,
            alt,
            id,
            width_percent,
        } => image_insert_plan(
            source,
            a,
            b,
            ImageFields {
                destination,
                alt,
                id: id.as_deref(),
                width_percent: *width_percent,
            },
        )?,
        FormatAction::ImageEdit {
            destination,
            alt,
            id,
            width_percent,
        } => image_edit_plan(
            source,
            &starts,
            a,
            b,
            ImageFields {
                destination,
                alt,
                id: id.as_deref(),
                width_percent: *width_percent,
            },
        )?,
        FormatAction::TableInsert { rows, cols } => table_insert_plan(source, a, b, *rows, *cols)?,
        FormatAction::TableAttrs { id, caption } => {
            table_attrs_plan(source, &starts, a, b, id.as_deref(), caption.as_deref())?
        }
    };

    let result = apply_edits(source, &edits);
    let result_starts = position::line_starts(&result);
    let edits = edits
        .into_iter()
        .map(|(start, end, text)| {
            (
                Range::new(
                    position::position_of(source, &starts, start),
                    position::position_of(source, &starts, end),
                ),
                text,
            )
        })
        .collect();
    // La selección se preserva con su dirección original (§8.6): `anchor` y
    // `head` se remapean por separado, no por orden creciente.
    let start = position::position_of(&result, &result_starts, after.0);
    let end = position::position_of(&result, &result_starts, after.1);
    let (anchor, head) = if selection.anchor <= selection.head {
        (start, end)
    } else {
        (end, start)
    };
    Some(FormatPlan {
        edits,
        selection: Selection { anchor, head },
    })
}

/// Contexto editorial bajo la `selection` (§44).
pub fn context(source: &str, selection: Selection) -> EditContext {
    let starts = position::line_starts(source);
    let range = selection.range();
    let (Some(a), Some(b)) = (
        position::offset_of(source, &starts, range.start),
        position::offset_of(source, &starts, range.end),
    ) else {
        return EditContext::default();
    };

    let line = position::position_of(source, &starts, a).line as usize;
    let heading_level = {
        let (start, end) = line_span(source, &starts, line);
        heading_prefix(&source[start..end]).map(|prefix| prefix.level)
    };

    EditContext {
        heading_level,
        link: link_at(source, a, b).map(|found| LinkContext {
            range: found.range(source, &starts),
            text_range: found.text_range(source, &starts),
            url: found.url,
        }),
        callout: callout_at(source, &starts, a, b),
        image: image_at(source, &starts, a, b),
        table: table_at(source, &starts, a, b),
        strong: a < b && wrapped(source, a, b, Inline::Strong),
        emphasis: a < b && wrapped(source, a, b, Inline::Emphasis),
        strikethrough: a < b && wrapped(source, a, b, Inline::Strikethrough),
        code: a < b && wrapped(source, a, b, Inline::Code),
    }
}

// ---------------------------------------------------------------------------
// Estilos inline
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inline {
    Strong,
    Emphasis,
    Strikethrough,
    Code,
}

impl Inline {
    /// (marcador canónico, byte del marcador).
    fn marker(self) -> (&'static str, u8) {
        match self {
            Inline::Strong => ("**", b'*'),
            Inline::Emphasis => ("*", b'*'),
            Inline::Strikethrough => ("~~", b'~'),
            Inline::Code => ("`", b'`'),
        }
    }
}

/// Cerco de código proporcional al contenido: mayor tanda de backticks + 1.
fn code_fence(inner: &str) -> String {
    let mut run = 0;
    let mut max = 0;
    for byte in inner.bytes() {
        if byte == b'`' {
            run += 1;
            max = max.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(max + 1)
}

/// Marcadores de código para `selected`, con acolchado cuando el contenido
/// empieza/termina en backtick (si no, se fundiría con el cerco) o en espacios
/// por ambos lados (si no, el parser los recortaría).
fn code_wrap_markers(selected: &str) -> (String, String) {
    let fence = code_fence(selected);
    let all_spaces = !selected.is_empty() && selected.bytes().all(|b| b == b' ');
    let pad = selected.starts_with('`')
        || selected.ends_with('`')
        || (selected.starts_with(' ') && selected.ends_with(' ') && !all_spaces);
    let open = if pad { format!("{fence} ") } else { fence.clone() };
    let close = if pad { format!(" {fence}") } else { fence };
    (open, close)
}

/// Al deshacer, un espacio de acolchado por lado se elimina solo si ambos
/// están y el interior no es todo espacios (misma regla que el parser).
fn strip_code_padding(inner: &str) -> &str {
    let all_spaces = !inner.is_empty() && inner.bytes().all(|b| b == b' ');
    if !all_spaces && inner.starts_with(' ') && inner.ends_with(' ') && inner.len() >= 2 {
        &inner[1..inner.len() - 1]
    } else {
        inner
    }
}

fn run_len_before(source: &str, at: usize, byte: u8) -> usize {
    source[..at].bytes().rev().take_while(|&b| b == byte).count()
}

fn run_len_after(source: &str, at: usize, byte: u8) -> usize {
    source[at..].bytes().take_while(|&b| b == byte).count()
}

/// Los marcadores abren/cierran DENTRO de la selección (`**foo**` con los
/// marcadores seleccionados también). Devuelve `(apertura, cierre)` en bytes,
/// exactos y sin tanda mayor (para que `*` no confunda con `**`): la tanda
/// debe estar completa, no continuar fuera de la selección (seleccionar
/// `*hola*` de `**hola**` no es una cursiva de verdad).
fn inner_markers(source: &str, a: usize, b: usize, inline: Inline) -> Option<(usize, usize)> {
    let (marker, byte) = inline.marker();
    let open = run_len_after(source, a, byte);
    let close = run_len_before(source, b, byte);
    let exact = if inline == Inline::Code {
        open >= 1 && open == close
    } else {
        open == marker.len() && close == open
    };
    let bounded = (a == 0 || source.as_bytes()[a - 1] != byte)
        && (b == source.len() || source.as_bytes()[b] != byte);
    let inner = b.saturating_sub(a).saturating_sub(open + close);
    (exact && bounded && inner > 0).then_some((open, close))
}

/// Los marcadores están justo FUERA de la selección (`**foo**` con solo «foo»
/// seleccionado). Devuelve los bytes a borrar antes de `a` y después de `b`
/// (en código incluyen el acolchado). La tanda completa debe tocar la
/// selección por su extremo: si la selección empieza/termina dentro de la
/// misma tanda, no hay envoltorio que deshacer.
fn outer_markers(source: &str, a: usize, b: usize, inline: Inline) -> Option<(usize, usize)> {
    if b <= a {
        return None;
    }
    if inline == Inline::Code {
        return code_outer(source, a, b);
    }
    let (marker, byte) = inline.marker();
    let open = run_len_before(source, a, byte);
    let close = run_len_after(source, b, byte);
    let bounded = source.as_bytes()[a] != byte && source.as_bytes()[b - 1] != byte;
    (open == marker.len() && close == open && bounded).then_some((open, close))
}

/// Cerco de código fuera de la selección, con su acolchado opcional:
/// `(bytes a borrar antes de a, bytes a borrar después de b)`.
fn code_outer(source: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    let mut open = run_len_before(source, a, b'`');
    let mut padded_open = false;
    if open == 0 && a > 0 && source.as_bytes()[a - 1] == b' ' {
        open = run_len_before(source, a - 1, b'`');
        if open > 0 {
            open += 1; // el acolchado forma parte del marcador.
            padded_open = true;
        }
    }
    let mut close = run_len_after(source, b, b'`');
    let mut padded_close = false;
    if close == 0 && b < source.len() && source.as_bytes()[b] == b' ' {
        close = run_len_after(source, b + 1, b'`');
        if close > 0 {
            close += 1;
            padded_close = true;
        }
    }
    let bounded = (padded_open || source.as_bytes()[a] != b'`')
        && (padded_close || source.as_bytes()[b - 1] != b'`');
    (open >= 1 && open == close && bounded).then_some((open, close))
}

/// La selección ya está envuelta por el marcador de `inline` (con los
/// marcadores dentro o justo fuera de sus límites).
fn wrapped(source: &str, a: usize, b: usize, inline: Inline) -> bool {
    inner_markers(source, a, b, inline).is_some() || outer_markers(source, a, b, inline).is_some()
}

/// Toggle inline sobre `[a, b)`: deshacer si ya está envuelto, envolver si no
/// y, con selección vacía, insertar el par de marcadores con el cursor dentro.
fn inline_plan(source: &str, a: usize, b: usize, inline: Inline) -> Option<RawPlan> {
    if a == b {
        // Selección vacía: par de marcadores con el cursor entre ellos.
        let open = match inline {
            Inline::Code => "`".to_string(),
            _ => inline.marker().0.to_string(),
        };
        let close = open.clone();
        return Some((
            vec![(a, a, format!("{open}{close}"))],
            (a + open.len(), a + open.len()),
        ));
    }
    if inline == Inline::Code && source[a..b].contains('\n') {
        return None; // código inline multilínea: sintaxis no soportada (§8.4).
    }

    if let Some((open, close)) = inner_markers(source, a, b, inline) {
        // Deshacer con los marcadores seleccionados: el interior queda tal
        // cual (sin el acolchado de código).
        let inner = &source[a + open..b - close];
        let replacement = if inline == Inline::Code {
            strip_code_padding(inner)
        } else {
            inner
        };
        return Some((
            vec![(a, b, replacement.to_string())],
            (a, a + replacement.len()),
        ));
    }
    if let Some((open, close)) = outer_markers(source, a, b, inline) {
        // Deshacer con los marcadores fuera: se borran y el texto no se toca.
        return Some((
            vec![(a - open, a, String::new()), (b, b + close, String::new())],
            (a - open, b - open),
        ));
    }

    // Envolver.
    let selected = &source[a..b];
    let (open, close) = match inline {
        Inline::Code => code_wrap_markers(selected),
        _ => {
            let marker = inline.marker().0;
            (marker.to_string(), marker.to_string())
        }
    };
    Some((
        vec![(a, b, format!("{open}{selected}{close}"))],
        (a + open.len(), a + open.len() + selected.len()),
    ))
}

// ---------------------------------------------------------------------------
// Headings (H1–H6)
// ---------------------------------------------------------------------------

struct HeadingPrefix {
    /// Rango del prefijo a sustituir, relativo al inicio de la línea.
    start: usize,
    end: usize,
    level: u8,
}

/// Reconoce un heading ATX `^ {0,3}#{1,6}( |$)` y devuelve su prefijo.
fn heading_prefix(line: &str) -> Option<HeadingPrefix> {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &rest[hashes..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let end = indent + hashes + usize::from(after.starts_with(' '));
    Some(HeadingPrefix {
        start: indent,
        end,
        level: hashes as u8,
    })
}

/// Fija el nivel de heading de cada línea cubierta por `[a, b)`.
fn heading_plan(
    source: &str,
    starts: &[usize],
    a: usize,
    b: usize,
    level: u8,
) -> Option<RawPlan> {
    if !(1..=6).contains(&level) {
        return None;
    }
    let (first, last) = covered_lines(source, starts, a, b);
    let replacement = format!("{} ", "#".repeat(level as usize));
    let mut edits = Vec::new();
    for line in first..=last {
        let (start, end) = line_span(source, starts, line as usize);
        let text = &source[start..end];
        if text.is_empty() && first != last {
            continue; // líneas vacías de un rango multi-bloque no se convierten.
        }
        match heading_prefix(text) {
            // No-op: todas las líneas ya están en el nivel pedido (sin cambios
            // que aplicar → `None` al final).
            Some(prefix) if prefix.level == level => {}
            Some(prefix) => edits.push((start + prefix.start, start + prefix.end, replacement.clone())),
            None => {
                let indent = text.bytes().take_while(|&b| b == b' ').count();
                edits.push((start + indent, start + indent, replacement.clone()))
            }
        }
    }
    if edits.is_empty() {
        return None;
    }
    let after = (map_offset(&edits, a), map_offset(&edits, b));
    Some((edits, after))
}

// ---------------------------------------------------------------------------
// Enlaces `[texto](url)`
// ---------------------------------------------------------------------------

struct LinkFound {
    /// `[` inicial y `)` final + 1, en offsets del fuente.
    start: usize,
    end: usize,
    /// Límites del texto entre `[` y `]`.
    text_start: usize,
    text_end: usize,
    /// Límites del destino (URL) entre `(` y `)`, sin título.
    dest_start: usize,
    dest_end: usize,
    url: String,
}

impl LinkFound {
    fn range(&self, source: &str, starts: &[usize]) -> Range {
        Range::new(
            position::position_of(source, starts, self.start),
            position::position_of(source, starts, self.end),
        )
    }

    fn text_range(&self, source: &str, starts: &[usize]) -> Range {
        Range::new(
            position::position_of(source, starts, self.text_start),
            position::position_of(source, starts, self.text_end),
        )
    }
}

/// Construcción `[texto](destino)` que contiene `[a, b]`, sea enlace o
/// imagen: `(construcción, es_imagen)`.
fn bracket_at(source: &str, a: usize, b: usize) -> Option<(LinkFound, bool)> {
    let bytes = source.as_bytes();
    let mut search = 0;
    while let Some(found) = source[search..].find("](") {
        let close_bracket = search + found;
        search = close_bracket + 2;
        let Some((dest_start, dest_end, end)) = parse_destination(source, close_bracket + 2) else {
            continue;
        };
        let Some(text_start) = label_open(source, close_bracket) else {
            continue;
        };
        let is_image = text_start > 0 && bytes[text_start - 1] == b'!';
        let found = LinkFound {
            start: text_start,
            end,
            text_start: text_start + 1,
            text_end: close_bracket,
            dest_start,
            dest_end,
            url: source[dest_start..dest_end].to_string(),
        };
        if a >= found.start && b <= found.end {
            return Some((found, is_image));
        }
    }
    None
}

/// Enlace `[texto](destino)` que contiene `[a, b]` (wiki links, reference
/// links e imágenes quedan fuera del alcance de #29).
fn link_at(source: &str, a: usize, b: usize) -> Option<LinkFound> {
    match bracket_at(source, a, b) {
        Some((found, false)) => Some(found),
        _ => None,
    }
}

/// Destino de un enlace desde justo después de `](`: `(inicio, fin)` del
/// destino y fin del enlace (tras el `)`). Soporta paréntesis equilibrados,
/// escapes, destino entre `<>` (el span es el interior) y título opcional.
fn parse_destination(source: &str, at: usize) -> Option<(usize, usize, usize)> {
    let bytes = source.as_bytes();
    let mut i = at;
    while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\n')) {
        i += 1;
    }
    let mut dest_start = i;
    let dest_end;
    if bytes.get(i) == Some(&b'<') {
        i += 1;
        dest_start = i;
        let mut escaped = false;
        loop {
            let b = *bytes.get(i)?;
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'>' {
                break;
            } else if b == b'\n' {
                return None;
            }
            i += 1;
        }
        dest_end = i;
        i += 1;
    } else {
        let mut depth = 0usize;
        let mut escaped = false;
        loop {
            let &b = bytes.get(i)?;
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'(' {
                depth += 1;
            } else if b == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if b == b' ' || b == b'\t' || b == b'\n' {
                break;
            }
            i += 1;
        }
        dest_end = i;
    }
    // Título opcional entre comillas o paréntesis.
    let mut j = i;
    while matches!(bytes.get(j), Some(b' ' | b'\t' | b'\n')) {
        j += 1;
    }
    if matches!(bytes.get(j), Some(b'"' | b'\'' | b'(')) {
        let open = bytes[j];
        let close = if open == b'(' { b')' } else { open };
        j += 1;
        let mut escaped = false;
        loop {
            let b = *bytes.get(j)?;
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == close {
                break;
            }
            j += 1;
        }
        j += 1;
        while matches!(bytes.get(j), Some(b' ' | b'\t' | b'\n')) {
            j += 1;
        }
    }
    (bytes.get(j) == Some(&b')')).then_some((dest_start, dest_end, j + 1))
}

/// `[` de apertura que corresponde al `]` de `close_bracket`, con corchetes
/// balanceados (los labels pueden contener `[` anidados o escapados).
fn label_open(source: &str, close_bracket: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut i = close_bracket;
    while i > 0 {
        i -= 1;
        let escaped = i > 0 && bytes[i - 1] == b'\\';
        if escaped {
            continue;
        }
        match bytes[i] {
            b']' => depth += 1,
            b'[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Serializa un destino de enlace/imagen: con espacios, paréntesis o
/// angulares se envuelve en `<>` (CommonMark) para que la construcción siga
/// siendo sintaxis detectable, no texto literal.
fn serialize_destination(destination: &str) -> String {
    let bare_safe = !destination.is_empty()
        && !destination
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>'));
    if bare_safe {
        destination.to_string()
    } else {
        format!("<{}>", destination.replace('\\', "\\\\").replace('>', "\\>"))
    }
}

fn link_plan(source: &str, a: usize, b: usize, url: &str) -> Option<RawPlan> {
    if url.trim().is_empty() {
        return None;
    }
    if image_found(source, a, b).is_some() {
        return None; // imagen: sus controles son los de #30.
    }
    let serialized = serialize_destination(url);
    if let Some(found) = link_at(source, a, b) {
        // Edición de enlace existente: solo el destino; texto, título y resto
        // intactos.
        if found.url == url {
            return None; // no-op.
        }
        let angle = source.as_bytes().get(found.dest_start.wrapping_sub(1)) == Some(&b'<');
        let (start, end) = if angle {
            (found.dest_start - 1, found.dest_end + 1)
        } else {
            (found.dest_start, found.dest_end)
        };
        let edits = vec![(start, end, serialized.clone())];
        let after = (start, start + serialized.len());
        return Some((edits, after));
    }
    if a < b {
        let selected = &source[a..b];
        return Some((
            vec![(a, b, format!("[{selected}]({serialized})"))],
            (a + 1, a + 1 + selected.len()),
        ));
    }
    Some((
        vec![(a, a, format!("[texto]({serialized})"))],
        (a + 1, a + 1 + "texto".len()),
    ))
}

// ---------------------------------------------------------------------------
// Callouts (§22): `::: tipo` / `::: tipo "Título"` … `:::`
// ---------------------------------------------------------------------------

struct CalloutOpen {
    /// Rango del token tipo en la línea del opener.
    type_start: usize,
    type_end: usize,
    callout_type: String,
    title: Option<String>,
}

/// Opener `::: tipo ["Título"]`; `None` para líneas que no abren callout
/// (incluido el closer suelto `:::`).
fn callout_open(line: &str) -> Option<CalloutOpen> {
    let trimmed = line.trim();
    if !trimmed.starts_with(":::") {
        return None;
    }
    let rest = &trimmed[3..];
    let name_offset = rest.len() - rest.trim_start().len();
    let rest = rest.trim_start();
    let name = rest
        .split(|c: char| c.is_whitespace() || c == '"')
        .next()
        .unwrap_or("");
    if name.is_empty() {
        return None;
    }
    let base = line.len() - line.trim_start().len();
    let type_start = base + 3 + name_offset;
    let tail = rest[name.len()..].trim();
    let title = tail
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .map(str::to_string);
    Some(CalloutOpen {
        type_start,
        type_end: type_start + name.len(),
        callout_type: name.to_string(),
        title,
    })
}

fn is_callout_close(line: &str) -> bool {
    line.trim() == ":::"
}

/// Líneas dentro de bloques de código cercados (``` o ~~~), delimitadores
/// incluidos: sus `:::` son literales, no estructura de callout (§8.4).
fn fenced_mask(lines: &[&str]) -> Vec<bool> {
    let mut mask = vec![false; lines.len()];
    let mut open: Option<(u8, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let run = trimmed.bytes().take_while(|&b| b == b'`' || b == b'~').count();
        let fence = trimmed
            .as_bytes()
            .first()
            .copied()
            .filter(|b| *b == b'`' || *b == b'~');
        if let (Some(ch), true) = (fence, run >= 3) {
            match open {
                None => {
                    open = Some((ch, run));
                    mask[index] = true;
                }
                Some((och, olen)) if ch == och && run >= olen && trimmed[run..].trim().is_empty() => {
                    mask[index] = true;
                    open = None;
                }
                _ => mask[index] = true,
            }
            continue;
        }
        if open.is_some() {
            mask[index] = true;
        }
    }
    mask
}

/// Línea del closer que cierra el opener de `open_line` (anidamiento contado;
/// las líneas de código cercado no cuentan).
fn callout_close_line(lines: &[&str], mask: &[bool], open_line: usize) -> Option<usize> {
    let mut depth = 0;
    for (index, line) in lines.iter().enumerate().skip(open_line) {
        if mask[index] {
            continue;
        }
        if callout_open(line).is_some() {
            depth += 1;
        } else if is_callout_close(line) {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// Callout más interno que contiene `[a, b]` (límites del bloque incluidos).
fn callout_at(source: &str, starts: &[usize], a: usize, b: usize) -> Option<CalloutContext> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mask = fenced_mask(&lines);
    let mut open = position::position_of(source, starts, a).line as usize;
    loop {
        let line = lines.get(open)?;
        if !mask[open] {
            if let Some(opener) = callout_open(line) {
                if let Some(close) = callout_close_line(&lines, &mask, open) {
                    let block_start = starts[open];
                    let block_end = line_content_end(source, starts, close as u32);
                    if a >= block_start && b <= block_end {
                        return Some(CalloutContext {
                            open_line: open as u32,
                            close_line: close as u32,
                            callout_type: opener.callout_type,
                            title: opener.title,
                        });
                    }
                }
            }
        }
        if open == 0 {
            return None;
        }
        open -= 1;
    }
}

fn callout_plan(
    source: &str,
    starts: &[usize],
    a: usize,
    b: usize,
    callout_type: &str,
) -> Option<RawPlan> {
    if callout_type.trim().is_empty() {
        return None;
    }
    if let Some(found) = callout_at(source, starts, a, b) {
        // Retonar el callout existente: solo cambia el tipo del opener.
        if found.callout_type == callout_type {
            return None; // no-op.
        }
        let (line_start, line_end) = line_span(source, starts, found.open_line as usize);
        let opener = callout_open(&source[line_start..line_end])?;
        let edits = vec![(
            line_start + opener.type_start,
            line_start + opener.type_end,
            callout_type.to_string(),
        )];
        let after = (
            line_start + opener.type_start,
            line_start + opener.type_start + callout_type.len(),
        );
        return Some((edits, after));
    }

    // Envolver los bloques que cruza la selección (o el bloque del cursor):
    // cada bloque es un párrafo (líneas no vacías contiguas). Un rango que
    // toca los delimitadores de un callout sin estar dentro de él no se toca
    // (§8.4) y las líneas de código cercado no son delimitadores.
    let lines: Vec<&str> = source.split('\n').collect();
    let mask = fenced_mask(&lines);
    let blank = |line: u32| {
        lines
            .get(line as usize)
            .is_none_or(|text| text.trim().is_empty())
    };
    let (mut first, mut last) = covered_lines(source, starts, a, b);
    if !blank(first) {
        while first > 0 && !blank(first - 1) {
            first -= 1;
        }
    }
    if !blank(last) {
        while !blank(last + 1) {
            last += 1;
        }
    }
    for line in first..=last {
        if mask.get(line as usize) == Some(&true) {
            continue;
        }
        let (start, end) = line_span(source, starts, line as usize);
        let text = &source[start..end];
        if callout_open(text).is_some() || is_callout_close(text) {
            return None;
        }
    }
    let nl = LineEnding::detect(source).as_str();
    let (block_start, _) = line_span(source, starts, first as usize);
    let block_end = line_content_end(source, starts, last);
    let opener_text = format!("::: {callout_type}{nl}");
    let edits = vec![
        (block_start, block_start, opener_text.clone()),
        (block_end, block_end, format!("{nl}:::")),
    ];
    let content_len = block_end - block_start;
    let after = (block_start + opener_text.len(), block_start + opener_text.len() + content_len);
    Some((edits, after))
}

// ---------------------------------------------------------------------------
// Imágenes (§18, §26.2) y tablas GFM (§35, §26.3)
// ---------------------------------------------------------------------------

/// Tokens de un bloque de atributos `{...}` (§7.4.3) con sus rangos: cada
/// span es el token completo (`#id`, `"caption"`, `width=N%`).
#[derive(Debug, Default)]
struct AttrSpans {
    /// `[inicio, fin)` del bloque completo `{...}`.
    range: (usize, usize),
    id: Option<(usize, usize)>,
    caption: Option<(usize, usize)>,
    width: Option<(usize, usize)>,
}

fn is_id_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'.' | b'_' | b'-')
}

fn is_key_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn is_value_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'%')
}

/// Bloque de atributos `{...}` que empieza en `open`, con los spans de sus
/// tokens. `None` si no es sintaxis de atributos (§7.4.1: texto literal).
fn attrs_spans(source: &str, open: usize) -> Option<AttrSpans> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    // `}` de cierre, respetando comillas y escapes (el caption puede
    // contener casi cualquier cosa).
    let mut i = open + 1;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let close = loop {
        let Some(&b) = bytes.get(i) else {
            return None; // bloque sin cerrar: texto literal.
        };
        if escaped {
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if let Some(active) = quote {
            if b == active {
                quote = None;
            }
        } else if b == b'"' {
            quote = Some(b);
        } else if b == b'}' {
            break i;
        }
        i += 1;
    };
    // La gramática manda: sin ella, la construcción completa es literal.
    crate::attrs::parse_attributes(&source[open + 1..close])?;

    let mut spans = AttrSpans {
        range: (open, close + 1),
        ..AttrSpans::default()
    };
    // Los tokens se delimitan con las mismas clases que `parse_attributes`
    // (§7.4.3) para que los spans no consuman atributos ajenos.
    let mut i = open + 1;
    while i < close {
        let c = source[i..].chars().next().unwrap_or('\0');
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        match bytes[i] {
            b'#' => {
                i += 1;
                while i < close && is_id_char(bytes[i]) {
                    i += 1;
                }
                spans.id = Some((start, i));
            }
            b'"' => {
                i += 1;
                let mut escaped = false;
                while i < close {
                    match bytes[i] {
                        b'\\' if !escaped => escaped = true,
                        b'"' if !escaped => {
                            i += 1;
                            break;
                        }
                        _ => escaped = false,
                    }
                    i += 1;
                }
                spans.caption = Some((start, i));
            }
            _ => {
                while i < close && is_key_char(bytes[i]) {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'=') {
                    i += 1;
                    while i < close && is_value_char(bytes[i]) {
                        i += 1;
                    }
                }
                if source[start..i].starts_with("width=") {
                    spans.width = Some((start, i));
                }
            }
        }
    }
    Some(spans)
}

fn attr_id_value(source: &str, span: (usize, usize)) -> String {
    source[span.0 + 1..span.1].to_string()
}

fn attr_caption_value(source: &str, span: (usize, usize)) -> String {
    let inner = &source[span.0 + 1..span.1 - 1];
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
}

fn attr_width_value(source: &str, span: (usize, usize)) -> Option<u8> {
    source[span.0..span.1]
        .strip_prefix("width=")?
        .strip_suffix('%')?
        .parse()
        .ok()
}

fn quote_caption(caption: &str) -> String {
    format!(
        "\"{}\"",
        caption.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Token válido para insertar como `#id`.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(is_id_char)
}

/// Amplía cada borrado de token de atributos con un único espacio
/// adyacente, sin solaparse con los otros borrados del mismo bloque: se
/// prefiere el espacio posterior y, si el token está al final del bloque, el
/// anterior.
fn extend_attr_deletes(source: &str, spans: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut spans = spans.to_vec();
    spans.sort_unstable();
    let mut out = Vec::with_capacity(spans.len());
    for &(start, end) in &spans {
        let mut start = start;
        let mut end = end;
        let prev_touch = out.iter().any(|&(_, e)| e == start);
        let next_touch = spans.iter().any(|&(s, _)| s == end);
        if !next_touch && source.as_bytes().get(end) == Some(&b' ') {
            end += 1;
        } else if !prev_touch && start > 0 && source.as_bytes()[start - 1] == b' ' {
            start -= 1;
        }
        debug_assert!(
            out.last().is_none_or(|&(_, e)| e <= start),
            "borrados de atributos solapados"
        );
        out.push((start, end));
    }
    out
}

/// Estado detectado de una imagen: el constructo `![alt](ruta "título")`
/// más el bloque de atributos pegado, si existe.
struct ImageFound {
    link: LinkFound,
    attrs: Option<AttrSpans>,
    /// Fin del constructo completo (atributos incluidos).
    end: usize,
}

/// Imagen que contiene `[a, b]` (la selección puede abarcar desde el `!`
/// inicial hasta los atributos, o estar dentro de cualquiera de esos
/// extremos). Imágenes según sintaxis canónica §26.2.
fn image_found(source: &str, a: usize, b: usize) -> Option<ImageFound> {
    let mut search = 0;
    while let Some(found) = source[search..].find("](") {
        let close_bracket = search + found;
        search = close_bracket + 2;
        let Some((dest_start, dest_end, end)) = parse_destination(source, close_bracket + 2) else {
            continue;
        };
        let Some(text_start) = label_open(source, close_bracket) else {
            continue;
        };
        if text_start == 0 || source.as_bytes()[text_start - 1] != b'!' {
            continue; // no es imagen.
        }
        let link = LinkFound {
            start: text_start,
            end,
            text_start: text_start + 1,
            text_end: close_bracket,
            dest_start,
            dest_end,
            url: source[dest_start..dest_end].to_string(),
        };
        // Atributos pegados tras la construcción (§7.4.3).
        let mut i = link.end;
        while matches!(source.as_bytes().get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        let (attrs, full_end) = match attrs_spans(source, i) {
            Some(spans) => {
                let end = spans.range.1;
                (Some(spans), end)
            }
            None => (None, link.end),
        };
        // Contención sobre el constructo completo: `!`, alt, destino y
        // atributos.
        let full_start = text_start - 1;
        if a >= full_start && b <= full_end {
            return Some(ImageFound {
                link,
                attrs,
                end: full_end,
            });
        }
    }
    None
}

/// Imagen que contiene `[a, b]` para el contexto (§44).
fn image_at(source: &str, starts: &[usize], a: usize, b: usize) -> Option<ImageContext> {
    let found = image_found(source, a, b)?;
    let pos = |offset| position::position_of(source, starts, offset);
    let attrs = found.attrs.as_ref();
    Some(ImageContext {
        range: Range::new(pos(found.link.start), pos(found.end)),
        destination: found.link.url.clone(),
        alt: source[found.link.text_start..found.link.text_end].to_string(),
        title: {
            let gap = source[found.link.dest_end..found.link.end.saturating_sub(1)].trim();
            (gap.len() >= 2 && gap.starts_with('"') && gap.ends_with('"'))
                .then(|| attr_caption_value(gap, (0, gap.len())))
        },
        id: attrs.and_then(|s| s.id).map(|span| attr_id_value(source, span)),
        width_percent: attrs
            .and_then(|s| s.width)
            .and_then(|span| attr_width_value(source, span)),
    })
}

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 2 && trimmed.starts_with('|') && trimmed.ends_with('|')
}

/// Fila de alineación GFM: celdas de `-` con `:` opcionales.
fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    if !is_table_row(trimmed) {
        return false;
    }
    trimmed[1..trimmed.len() - 1].split('|').all(|cell| {
        let cell = cell.trim();
        !cell.is_empty()
            && cell.contains('-')
            && cell.bytes().all(|b| b == b'-' || b == b':')
    })
}

fn table_columns(line: &str) -> usize {
    let trimmed = line.trim();
    trimmed[1..trimmed.len() - 1].split('|').count()
}

/// Estado detectado de una tabla: bloque de filas más la línea de
/// atributos posterior (§26.3), si existe.
struct TableFound {
    block: (usize, usize),
    attrs_line: Option<usize>,
    attrs: Option<AttrSpans>,
    columns: usize,
}

fn table_found(source: &str, starts: &[usize], a: usize, b: usize) -> Option<TableFound> {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_at = |offset: usize| position::position_of(source, starts, offset).line as usize;
    let mut anchor = line_at(a);
    // Cursor en la línea de atributos: la tabla es la del bloque anterior.
    if !is_table_row(lines.get(anchor).copied().unwrap_or("")) {
        if anchor == 0 {
            return None;
        }
        anchor -= 1;
    }
    if !is_table_row(lines.get(anchor).copied().unwrap_or("")) {
        return None;
    }
    let mut first = anchor;
    while first > 0 && is_table_row(lines[first - 1]) {
        first -= 1;
    }
    let mut last = anchor;
    while last + 1 < lines.len() && is_table_row(lines[last + 1]) {
        last += 1;
    }
    if first + 1 > last || !is_table_separator(lines[first + 1]) {
        return None; // sin fila de alineación no hay tabla (§35).
    }
    let (block_start, _) = line_span(source, starts, first);
    let block_end = line_content_end(source, starts, last as u32);
    // Línea de atributos propia posterior (§26.3). El offset se calcula solo
    // con la sangre inicial: los `\r`/espacios finales no desplazan el `{`.
    let (attrs_line, attrs) = lines
        .get(last + 1)
        .and_then(|text| {
            let trimmed = text.trim();
            let indent = text.len() - text.trim_start().len();
            let offset = starts.get(last + 1)? + indent;
            let spans = attrs_spans(source, offset)?;
            (spans.range.1 == offset + trimmed.len()).then_some((last + 1, spans))
        })
        .unzip();
    // La selección debe estar en el bloque de filas o en su línea de atributos.
    let in_block = a >= block_start && b <= block_end;
    let in_attrs = attrs
        .as_ref()
        .is_some_and(|spans| a >= spans.range.0 && b <= spans.range.1);
    if !in_block && !in_attrs {
        return None;
    }
    Some(TableFound {
        block: (block_start, block_end),
        attrs_line,
        attrs,
        columns: table_columns(lines[first]),
    })
}

/// Tabla que contiene `[a, b]` para el contexto (§44).
fn table_at(source: &str, starts: &[usize], a: usize, b: usize) -> Option<TableContext> {
    let found = table_found(source, starts, a, b)?;
    let pos = |offset| position::position_of(source, starts, offset);
    let attrs = found.attrs.as_ref();
    Some(TableContext {
        range: Range::new(pos(found.block.0), pos(found.block.1)),
        columns: found.columns,
        id: attrs.and_then(|s| s.id).map(|span| attr_id_value(source, span)),
        caption: attrs
            .and_then(|s| s.caption)
            .map(|span| attr_caption_value(source, span)),
    })
}

/// Campos de una imagen para insertar/editar (§18): `None` en `id` o
/// `width_percent` significa «sin ese atributo».
#[derive(Debug, Clone, Copy)]
struct ImageFields<'a> {
    destination: &'a str,
    alt: &'a str,
    id: Option<&'a str>,
    width_percent: Option<u8>,
}

fn image_insert_plan(source: &str, a: usize, b: usize, fields: ImageFields<'_>) -> Option<RawPlan> {
    let ImageFields {
        destination,
        alt,
        id,
        width_percent,
    } = fields;
    if destination.is_empty() {
        return None;
    }
    if let Some(width) = width_percent {
        if !(1..=100).contains(&width) {
            return None;
        }
    }
    let mut tokens = Vec::new();
    if let Some(id) = id {
        if !valid_id(id) {
            return None;
        }
        tokens.push(format!("#{id}"));
    }
    if let Some(width) = width_percent {
        tokens.push(format!("width={width}%"));
    }
    let attrs = if tokens.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", tokens.join(" "))
    };
    let (alt, selected) = if a < b {
        (&source[a..b], true)
    } else {
        (alt, false)
    };
    let _ = selected;
    let text = format!("![{alt}]({}){attrs}", serialize_destination(destination));
    Some((
        vec![(a, b, text)],
        (a + 2, a + 2 + alt.len()), // selección sobre el alt (§8.6).
    ))
}

fn image_edit_plan(
    source: &str,
    starts: &[usize],
    a: usize,
    b: usize,
    fields: ImageFields<'_>,
) -> Option<RawPlan> {
    let ImageFields {
        destination,
        alt,
        id,
        width_percent,
    } = fields;
    if destination.is_empty() {
        return None;
    }
    if let Some(width) = width_percent {
        if !(1..=100).contains(&width) {
            return None;
        }
    }
    if let Some(id) = id {
        if !valid_id(id) {
            return None;
        }
    }
    let found = image_found(source, a, b)?;
    let mut edits: Vec<RawEdit> = Vec::new();
    let mut deleted: Vec<(usize, usize)> = Vec::new();

    if destination != found.link.url {
        let serialized = serialize_destination(destination);
        let angle = source.as_bytes().get(found.link.dest_start.wrapping_sub(1)) == Some(&b'<');
        let (start, end) = if angle {
            (found.link.dest_start - 1, found.link.dest_end + 1)
        } else {
            (found.link.dest_start, found.link.dest_end)
        };
        edits.push((start, end, serialized));
    }
    let alt_span = (found.link.text_start, found.link.text_end);
    if alt != &source[alt_span.0..alt_span.1] {
        edits.push((alt_span.0, alt_span.1, alt.to_string()));
    }

    let mut new_block_tokens: Vec<String> = Vec::new();
    match (&found.attrs, id) {
        (Some(spans), Some(id)) => match spans.id {
            Some(span) if attr_id_value(source, span) != id => {
                edits.push((span.0, span.1, format!("#{id}")));
            }
            None => new_block_tokens.push(format!("#{id} ")),
            _ => {}
        },
        (Some(spans), None) => {
            if let Some(span) = spans.id {
                deleted.push(span);
            }
        }
        (None, Some(id)) => new_block_tokens.push(format!("#{id}")),
        (None, None) => {}
    }
    match (&found.attrs, width_percent) {
        (Some(spans), Some(width)) => match spans.width {
            Some(span) if attr_width_value(source, span) != Some(width) => {
                edits.push((span.0, span.1, format!("width={width}%")));
            }
            None => new_block_tokens.push(format!("width={width}% ")),
            _ => {}
        },
        (Some(spans), None) => {
            if let Some(span) = spans.width {
                deleted.push(span);
            }
        }
        (None, Some(width)) => new_block_tokens.push(format!("width={width}%")),
        (None, None) => {}
    }

    for (start, end) in extend_attr_deletes(source, &deleted) {
        edits.push((start, end, String::new()));
    }
    if let Some(spans) = &found.attrs {
        // Tokens nuevos dentro del bloque existente, justo tras `{`.
        if !new_block_tokens.is_empty() {
            let text = new_block_tokens.join("");
            edits.push((spans.range.0 + 1, spans.range.0 + 1, text));
        }
        // Si tras los borrados (y sin tokens nuevos) solo quedan espacios,
        // el bloque desaperece.
        if new_block_tokens.is_empty() {
            let deletes: Vec<(usize, usize)> = edits
                .iter()
                .filter(|(s, e, t)| {
                    t.is_empty() && *s >= spans.range.0 && *e <= spans.range.1
                })
                .map(|(s, e, _)| (*s, *e))
                .collect();
            let mut inner = source[spans.range.0 + 1..spans.range.1 - 1].to_string();
            for (start, end) in deletes.iter().rev() {
                inner.replace_range(start - spans.range.0 - 1..end - spans.range.0 - 1, "");
            }
            if inner.trim().is_empty() {
                edits.retain(|(s, e, t)| !(t.is_empty() && *s >= spans.range.0 && *e <= spans.range.1));
                let mut start = spans.range.0;
                if start > 0 && source.as_bytes()[start - 1] == b' ' {
                    start -= 1;
                }
                edits.push((start, spans.range.1, String::new()));
            }
        }
    } else if !new_block_tokens.is_empty() {
        let tokens: Vec<&str> = new_block_tokens.iter().map(|t| t.trim_end()).collect();
        edits.push((found.end, found.end, format!(" {{{}}}", tokens.join(" "))));
    }

    if edits.is_empty() {
        return None; // no-op.
    }
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    let _ = starts;
    let after = (map_offset(&edits, a), map_offset(&edits, b));
    Some((edits, after))
}

fn table_insert_plan(
    source: &str,
    a: usize,
    b: usize,
    rows: usize,
    cols: usize,
) -> Option<RawPlan> {
    if !(1..=50).contains(&rows) || !(1..=8).contains(&cols) {
        return None;
    }
    let nl = LineEnding::detect(source).as_str();
    let header = format!("|{}", "     |".repeat(cols));
    let separator = format!("|{}", " --- |".repeat(cols));
    let empty_row = header.clone();
    let mut text = format!("{header}{nl}{separator}{nl}");
    for _ in 0..rows {
        text.push_str(&empty_row);
        text.push_str(nl);
    }
    text.truncate(text.len() - nl.len()); // sin salto final colgante.
    let after = (a + 2, a + 2); // primera celda de cabecera.
    Some((vec![(a, b, text)], after))
}

fn table_attrs_plan(
    source: &str,
    starts: &[usize],
    a: usize,
    b: usize,
    id: Option<&str>,
    caption: Option<&str>,
) -> Option<RawPlan> {
    if let Some(id) = id {
        if !valid_id(id) {
            return None;
        }
    }
    let caption = caption.filter(|text| !text.trim().is_empty());
    let found = table_found(source, starts, a, b)?;
    let mut edits: Vec<RawEdit> = Vec::new();
    let mut deleted: Vec<(usize, usize)> = Vec::new();

    let mut new_tokens: Vec<String> = Vec::new();
    match (&found.attrs, id) {
        (Some(spans), Some(id)) => match spans.id {
            Some(span) if attr_id_value(source, span) != id => {
                edits.push((span.0, span.1, format!("#{id}")));
            }
            None => new_tokens.push(format!("#{id} ")),
            _ => {}
        },
        (Some(spans), None) => {
            if let Some(span) = spans.id {
                deleted.push(span);
            }
        }
        (None, Some(id)) => new_tokens.push(format!("#{id}")),
        (None, None) => {}
    }
    match (&found.attrs, caption) {
        (Some(spans), Some(caption)) => match spans.caption {
            Some(span) if attr_caption_value(source, span) != caption => {
                edits.push((span.0, span.1, quote_caption(caption)));
            }
            None => new_tokens.push(format!("{} ", quote_caption(caption))),
            _ => {}
        },
        (Some(spans), None) => {
            if let Some(span) = spans.caption {
                deleted.push(span);
            }
        }
        (None, Some(caption)) => new_tokens.push(quote_caption(caption)),
        (None, None) => {}
    }

    for (start, end) in extend_attr_deletes(source, &deleted) {
        edits.push((start, end, String::new()));
    }
    if let Some(spans) = &found.attrs {
        if !new_tokens.is_empty() {
            let text = new_tokens.join("");
            edits.push((spans.range.0 + 1, spans.range.0 + 1, text));
        }
        // La línea de atributos que queda vacía (sin tokens nuevos) se
        // elimina entera.
        if new_tokens.is_empty() {
            let deletes: Vec<(usize, usize)> = edits
                .iter()
                .filter(|(s, e, t)| t.is_empty() && *s >= spans.range.0 && *e <= spans.range.1)
                .map(|(s, e, _)| (*s, *e))
                .collect();
            let mut inner = source[spans.range.0 + 1..spans.range.1 - 1].to_string();
            for (start, end) in deletes.iter().rev() {
                inner.replace_range(start - spans.range.0 - 1..end - spans.range.0 - 1, "");
            }
            if inner.trim().is_empty() {
                edits.retain(|(s, e, t)| !(t.is_empty() && *s >= spans.range.0 && *e <= spans.range.1));
                let line = found.attrs_line?;
                let line_start = starts[line];
                let mut end = line_content_end(source, starts, line as u32);
                if source.as_bytes().get(end) == Some(&b'\r') {
                    end += 1;
                }
                if source.as_bytes().get(end) == Some(&b'\n') {
                    end += 1;
                }
                edits.push((line_start, end, String::new()));
            }
        }
    } else if !new_tokens.is_empty() {
        let tokens: Vec<&str> = new_tokens.iter().map(|t| t.trim_end()).collect();
        let line = format!("{{{}}}", tokens.join(" "));
        let nl = LineEnding::detect(source).as_str();
        edits.push((found.block.1, found.block.1, format!("{nl}{line}")));
    }

    if edits.is_empty() {
        return None; // no-op.
    }
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    let after = (map_offset(&edits, a), map_offset(&edits, b));
    Some((edits, after))
}

// ---------------------------------------------------------------------------
// Utilidades de líneas y offsets
// ---------------------------------------------------------------------------

/// Primera y última línea cubiertas por `[a, b)` (si `b` cae justo al inicio
/// de una línea, esa línea no está cubierta).
fn covered_lines(source: &str, starts: &[usize], a: usize, b: usize) -> (u32, u32) {
    let first = position::position_of(source, starts, a).line;
    let mut last = position::position_of(source, starts, b).line;
    if b > a && starts.get(last as usize) == Some(&b) && last > first {
        last -= 1;
    }
    (first, last)
}

/// `[inicio, fin)` de bytes de la línea `line`, sin su terminador.
fn line_span(source: &str, starts: &[usize], line: usize) -> (usize, usize) {
    let start = *starts.get(line).unwrap_or(&source.len());
    (start, line_content_end(source, starts, line as u32))
}

fn line_content_end(source: &str, starts: &[usize], line: u32) -> usize {
    let start = *starts.get(line as usize).unwrap_or(&source.len());
    let mut end = starts
        .get(line as usize + 1)
        .copied()
        .unwrap_or(source.len())
        .min(source.len());
    if end > start && source.as_bytes()[end - 1] == b'\n' {
        end -= 1;
    }
    if end > start && source.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    end
}

/// Resultado de aplicar `edits` (ordenados, disjuntos) a `source`.
fn apply_edits(source: &str, edits: &[RawEdit]) -> String {
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    for (start, end, text) in edits {
        out.push_str(&source[at..*start]);
        out.push_str(text);
        at = *end;
    }
    out.push_str(&source[at..]);
    out
}

/// Mapea un offset del buffer original al buffer resultante de aplicar
/// `edits` (ordenados, disjuntos). Un offset dentro de un rango sustituido
/// cae al final del texto insertado.
fn map_offset(edits: &[RawEdit], offset: usize) -> usize {
    let mut delta: isize = 0;
    for (start, end, text) in edits {
        if offset <= *start {
            break;
        }
        if offset <= *end {
            return (*start as isize + delta + text.len() as isize) as usize;
        }
        delta += text.len() as isize - (end - start) as isize;
    }
    (offset as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::position::Position;
    use crate::Editor;

    fn pos(line: u32, column: u32) -> Position {
        Position::new(line, column)
    }

    /// Selección `[start, end)` de bytes del texto, convertida a posiciones.
    fn sel_of(text: &str, start: usize, end: usize) -> Selection {
        let starts = position::line_starts(text);
        Selection {
            anchor: position::position_of(text, &starts, start),
            head: position::position_of(text, &starts, end),
        }
    }

    /// Aplica el plan con el Editor real (una transacción) y devuelve el
    /// contenido resultante.
    fn applied(source: &str, selection: Selection, action: &FormatAction) -> Option<String> {
        let plan = plan(source, selection, action)?;
        let mut editor = Editor::new(Document::from_text(None, source.to_string()));
        editor.set_selection(selection).unwrap();
        editor
            .replace_all_selecting(&plan.edits, plan.selection)
            .unwrap();
        assert_eq!(editor.selection(), plan.selection, "selección registrada");
        Some(editor.document().content().to_string())
    }

    #[test]
    fn seleccion_vacia_inserta_par_de_marcadores() {
        let selection = Selection::collapsed(pos(0, 1));
        let plan = plan("ab", selection, &FormatAction::Strong).unwrap();
        assert_eq!(
            applied("ab", selection, &FormatAction::Strong),
            Some("a****b".to_string())
        );
        // Cursor entre los marcadores.
        assert_eq!(plan.selection, Selection::collapsed(pos(0, 3)));
    }

    #[test]
    fn envuelve_y_deshace_los_cuatro_estilos() {
        for (action, wrapped) in [
            (FormatAction::Strong, "**hola**"),
            (FormatAction::Emphasis, "*hola*"),
            (FormatAction::Strikethrough, "~~hola~~"),
            (FormatAction::Code, "`hola`"),
        ] {
            let text = "hola";
            assert_eq!(
                applied(text, sel_of(text, 0, 4), &action),
                Some(wrapped.to_string())
            );
            // Deshacer con los marcadores FUERA de la selección.
            let open = wrapped.len() - wrapped.trim_start_matches(['*', '~', '`']).len();
            assert_eq!(
                applied(wrapped, sel_of(wrapped, open, open + 4), &action),
                Some(text.to_string())
            );
        }
    }

    #[test]
    fn deshace_con_marcadores_incluidos_en_la_seleccion() {
        let text = "**hola**";
        let plan = plan(text, sel_of(text, 0, text.len()), &FormatAction::Strong).unwrap();
        assert_eq!(plan.edits.len(), 1);
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &FormatAction::Strong),
            Some("hola".to_string())
        );
        // La selección queda sobre el texto interior.
        assert_eq!(plan.selection, sel_of("hola", 0, 4));
    }

    #[test]
    fn cursiva_no_confunde_con_negrita() {
        // `**hola**` con cursiva se envuelve (queda negrita+cursiva), no se deshace.
        let text = "**hola**";
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &FormatAction::Emphasis),
            Some("***hola***".to_string())
        );
    }

    #[test]
    fn codigo_con_backticks_usa_cerco_mayor() {
        let text = "a`b";
        assert_eq!(
            applied(text, sel_of(text, 0, 3), &FormatAction::Code),
            Some("``a`b``".to_string())
        );
    }

    #[test]
    fn codigo_con_backtick_en_el_borde_acolcha() {
        // Sin acolchado, el backtick inicial se fundiría con el cerco.
        let text = "x `a";
        let start = text.find('`').unwrap();
        assert_eq!(
            applied(text, sel_of(text, start, start + 2), &FormatAction::Code),
            Some("x `` `a ``".to_string())
        );
    }

    #[test]
    fn codigo_quita_el_acolchado_al_deshacer() {
        // Ida y vuelta sobre el mismo contenido devuelve el texto original.
        let text = "`` `a ``";
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &FormatAction::Code),
            Some("`a".to_string())
        );
    }

    #[test]
    fn codigo_no_funde_espacios_de_contenido_con_el_acolchado() {
        // Contenido con backtick inicial y espacio final: el cerco acolcha por
        // ambos lados y el deshacer restaura el contenido exacto.
        let text = "x `a ";
        let start = text.find('`').unwrap();
        assert_eq!(
            applied(text, sel_of(text, start, start + 3), &FormatAction::Code),
            Some("x `` `a  ``".to_string())
        );
        let wrapped = "`` `a  ``";
        assert_eq!(
            applied(wrapped, sel_of(wrapped, 0, wrapped.len()), &FormatAction::Code),
            Some("`a ".to_string())
        );
    }

    #[test]
    fn tanda_parcial_de_marcador_no_es_envoltorio() {
        // Seleccionar `*hola*` de `**hola**` no es una cursiva: la cursiva
        // envuelve (queda negrita+cursiva) en vez de deshacer la negrita.
        let text = "**hola**";
        assert_eq!(
            applied(text, sel_of(text, 1, 7), &FormatAction::Emphasis),
            Some("***hola***".to_string())
        );
    }

    #[test]
    fn codigo_multilinea_no_aplica() {
        assert_eq!(plan("uno\ndos", sel_of("uno\ndos", 0, 7), &FormatAction::Code), None);
    }

    #[test]
    fn seleccion_unicode_conserva_columnas() {
        let text = "ñandú";
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &FormatAction::Strong),
            Some("**ñandú**".to_string())
        );
    }

    #[test]
    fn heading_convierte_y_cambia_nivel() {
        let text = "titulo";
        assert_eq!(
            applied(text, sel_of(text, 0, 5), &FormatAction::Heading(2)),
            Some("## titulo".to_string())
        );
        assert_eq!(
            applied("### titulo", sel_of("### titulo", 0, 5), &FormatAction::Heading(1)),
            Some("# titulo".to_string())
        );
        assert_eq!(
            applied(text, sel_of(text, 0, 5), &FormatAction::Heading(6)),
            Some("###### titulo".to_string())
        );
    }

    #[test]
    fn heading_multilinea_omite_lineas_vacias() {
        let text = "uno\n\ndos";
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &FormatAction::Heading(1)),
            Some("# uno\n\n# dos".to_string())
        );
    }

    #[test]
    fn heading_en_el_mismo_nivel_es_no_op() {
        assert_eq!(
            plan("## titulo", sel_of("## titulo", 0, 8), &FormatAction::Heading(2)),
            None
        );
    }

    #[test]
    fn heading_fuera_de_rango_no_aplica() {
        assert_eq!(plan("x", sel_of("x", 0, 1), &FormatAction::Heading(0)), None);
        assert_eq!(plan("x", sel_of("x", 0, 1), &FormatAction::Heading(7)), None);
    }

    #[test]
    fn heading_preserva_la_seleccion_tras_cambiar_prefijos() {
        let text = "uno\ndos";
        let selection = sel_of(text, 0, text.len());
        let plan = plan(text, selection, &FormatAction::Heading(3)).unwrap();
        assert_eq!(
            applied(text, selection, &FormatAction::Heading(3)),
            Some("### uno\n### dos".to_string())
        );
        // La selección cubre las mismas líneas, prefijos incluidos.
        assert_eq!(plan.selection, sel_of("### uno\n### dos", 0, 15));
    }

    #[test]
    fn seleccion_hacia_atras_conserva_la_direccion() {
        // Selección invertida (anchor al final, head al inicio): la acción
        // preserva ambos extremos y su orden (§8.6).
        let text = "abc";
        let selection = Selection {
            anchor: pos(0, 3),
            head: pos(0, 0),
        };
        let plan = plan(text, selection, &FormatAction::Heading(2)).unwrap();
        assert_eq!(plan.selection.anchor, pos(0, 6));
        assert_eq!(plan.selection.head, pos(0, 0));
    }

    #[test]
    fn enlace_envuelve_la_seleccion() {
        let text = "hola mundo";
        let action = FormatAction::Link { url: "https://x".into() };
        let plan = plan(text, sel_of(text, 0, 4), &action).unwrap();
        assert_eq!(
            applied(text, sel_of(text, 0, 4), &action),
            Some("[hola](https://x) mundo".to_string())
        );
        // Selección sobre el texto del enlace.
        assert_eq!(plan.selection, sel_of("[hola](https://x) mundo", 1, 5));
    }

    #[test]
    fn enlace_con_cursor_inserta_marcador_de_texto() {
        let text = "ab";
        let action = FormatAction::Link { url: "https://x".into() };
        assert_eq!(
            applied(text, Selection::collapsed(pos(0, 2)), &action),
            Some("ab[texto](https://x)".to_string())
        );
    }

    #[test]
    fn enlace_edita_solo_la_url() {
        let text = "ver [hola](https://vieja) ya";
        let cursor = text.find("vieja").unwrap() + 1;
        let action = FormatAction::Link { url: "https://nueva".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("ver [hola](https://nueva) ya".to_string())
        );
    }

    #[test]
    fn enlace_vacio_o_igual_es_no_op() {
        let text = "ver [hola](https://x) ya";
        let cursor = text.find("hola").unwrap() + 1;
        let selection = Selection::collapsed(sel_of(text, cursor, cursor).anchor);
        assert_eq!(plan(text, selection, &FormatAction::Link { url: "  ".into() }), None);
        assert_eq!(
            plan(text, selection, &FormatAction::Link { url: "https://x".into() }),
            None
        );
    }

    #[test]
    fn enlace_con_parentesis_equilibrados_edita_solo_destino() {
        let text = "ver [x](https://host/a(b)) ya";
        let cursor = text.find("x]").unwrap();
        let action = FormatAction::Link { url: "new".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("ver [x](new) ya".to_string())
        );
    }

    #[test]
    fn enlace_con_titulo_conserva_titulo_y_texto() {
        let text = "ver [x](old \"título\") ya";
        let cursor = text.find("x]").unwrap();
        let action = FormatAction::Link { url: "new".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("ver [x](new \"título\") ya".to_string())
        );
    }

    #[test]
    fn enlace_con_corchetes_anidados_en_el_label() {
        let text = "[outer [inner] text](old)";
        let cursor = text.find("outer").unwrap() + 1;
        let action = FormatAction::Link { url: "new".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("[outer [inner] text](new)".to_string())
        );
    }

    #[test]
    fn imagen_no_se_toca_con_la_accion_de_enlace() {
        let text = "![alt](img.png)";
        let cursor = text.find("alt").unwrap();
        assert_eq!(
            plan(
                text,
                Selection::collapsed(sel_of(text, cursor, cursor).anchor),
                &FormatAction::Link { url: "new".into() }
            ),
            None
        );
    }

    #[test]
    fn callout_envuelve_los_bloques_cubiertos() {
        let text = "hola\nadios";
        let action = FormatAction::Callout { callout_type: "note".into() };
        // La línea cubierta pertenece a un párrafo: se envuelve el bloque.
        assert_eq!(
            applied(text, sel_of(text, 0, 4), &action),
            Some("::: note\nhola\nadios\n:::".to_string())
        );
    }

    #[test]
    fn callout_envuelve_el_bloque_del_cursor_sin_partirlo() {
        // Cursor en la segunda línea del párrafo: no se parte el bloque.
        let text = "primera\nsegunda";
        let cursor = text.find("segunda").unwrap();
        let action = FormatAction::Callout { callout_type: "note".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("::: note\nprimera\nsegunda\n:::".to_string())
        );
    }

    #[test]
    fn callout_respeta_los_limites_entre_parrafos() {
        // Dos párrafos separados por línea en blanco: solo se envuelve el del cursor.
        let text = "uno\n\ndos";
        let cursor = text.find("dos").unwrap();
        let action = FormatAction::Callout { callout_type: "tip".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("uno\n\n::: tip\ndos\n:::".to_string())
        );
    }

    #[test]
    fn callout_ignora_delimitadores_en_codigo_cercado() {
        // El `::: tip` literal está dentro de código cercado: el callout real
        // es el externo y su contenido no se toca.
        let text = "::: note\n```\n::: tip\n```\ncuerpo\n:::\n";
        let cursor = text.find("cuerpo").unwrap();
        let action = FormatAction::Callout { callout_type: "danger".into() };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("::: danger\n```\n::: tip\n```\ncuerpo\n:::\n".to_string())
        );
    }

    #[test]
    fn callout_retona_conservando_titulo_y_contenido() {
        let text = "::: warning \"Atención\"\nNo borres.\n:::\n";
        let action = FormatAction::Callout { callout_type: "danger".into() };
        let cursor = text.find("No borres").unwrap();
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("::: danger \"Atención\"\nNo borres.\n:::\n".to_string())
        );
    }

    #[test]
    fn callout_del_mismo_tipo_es_no_op() {
        let text = "::: note\nx\n:::";
        let cursor = text.find('x').unwrap();
        assert_eq!(
            plan(
                text,
                Selection::collapsed(sel_of(text, cursor, cursor).anchor),
                &FormatAction::Callout { callout_type: "note".into() }
            ),
            None
        );
    }

    #[test]
    fn callout_cruzado_por_el_borde_no_se_toca() {
        let text = "antes\n::: note\nx\n:::";
        // Selección que entra en el callout desde fuera.
        let selection = sel_of(text, 0, text.find('x').unwrap());
        assert_eq!(
            plan(text, selection, &FormatAction::Callout { callout_type: "tip".into() }),
            None
        );
    }

    #[test]
    fn callout_vacio_es_no_op() {
        assert_eq!(
            plan("x", sel_of("x", 0, 1), &FormatAction::Callout { callout_type: " ".into() }),
            None
        );
    }

    #[test]
    fn contexto_detecta_constructos_y_envoltorios() {
        let text = "## Titulo\n\nver [hola](https://x) **fuerte**\n\n::: warning \"Ojo\"\ntexto\n:::\n";

        // Cursor dentro del enlace.
        let cursor = text.find("hola").unwrap() + 1;
        let ctx = context(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor));
        assert_eq!(ctx.heading_level, None);
        assert_eq!(ctx.link.expect("enlace").url, "https://x");
        assert!(!ctx.strong);

        // Selección «fuerte» ya envuelta.
        let start = text.find("fuerte").unwrap();
        let ctx = context(text, sel_of(text, start, start + 6));
        assert!(ctx.strong, "strong detectado");
        assert!(!ctx.emphasis);

        // Heading.
        let ctx = context(text, Selection::collapsed(pos(0, 1)));
        assert_eq!(ctx.heading_level, Some(2));

        // Callout con título.
        let inside = text.find("texto").unwrap();
        let callout = context(text, Selection::collapsed(sel_of(text, inside, inside).anchor))
            .callout
            .expect("callout");
        assert_eq!(callout.callout_type, "warning");
        assert_eq!(callout.title.as_deref(), Some("Ojo"));
        assert_eq!(callout.open_line, 4);
        assert_eq!(callout.close_line, 6);

        // Fuera de todo: sin utilidad.
        let empty = text.find("ver").unwrap();
        assert_eq!(
            context(text, Selection::collapsed(sel_of(text, empty, empty).anchor)),
            EditContext::default()
        );
    }

    #[test]
    fn apply_undo_redo_es_una_sola_transaccion() {
        let text = "hola";
        let selection = sel_of(text, 0, 4);
        let plan = plan(text, selection, &FormatAction::Strong).unwrap();
        let mut editor = Editor::new(Document::from_text(None, text.to_string()));
        editor.set_selection(selection).unwrap();
        editor
            .replace_all_selecting(&plan.edits, plan.selection)
            .unwrap();
        assert_eq!(editor.document().content(), "**hola**");

        assert!(editor.undo());
        assert_eq!(editor.document().content(), text);
        assert_eq!(editor.selection(), selection, "undo restaura la selección previa");
        assert!(!editor.undo());

        assert!(editor.redo());
        assert_eq!(editor.document().content(), "**hola**");
        assert_eq!(
            editor.selection(),
            plan.selection,
            "redo restaura la selección posterior"
        );
    }

    // ---- Imágenes (§18, §26.2) ----

    #[test]
    fn imagen_insertar_basica_y_con_atributos() {
        let action = FormatAction::ImageInsert {
            destination: "assets/a.png".into(),
            alt: "Foto".into(),
            id: Some("fig:a".into()),
            width_percent: Some(50),
        };
        assert_eq!(
            applied("", Selection::collapsed(pos(0, 0)), &action),
            Some("![Foto](assets/a.png) {#fig:a width=50%}".to_string())
        );
        let action = FormatAction::ImageInsert {
            destination: "a.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied("", Selection::collapsed(pos(0, 0)), &action),
            Some("![x](a.png)".to_string())
        );
    }

    #[test]
    fn imagen_insertar_con_seleccion_usa_el_texto_como_alt() {
        let text = "hola mundo";
        let action = FormatAction::ImageInsert {
            destination: "a.png".into(),
            alt: "otro".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied(text, sel_of(text, 0, 4), &action),
            Some("![hola](a.png) mundo".to_string())
        );
    }

    #[test]
    fn imagen_insertar_parametros_invalidos_no_aplican() {
        let at = Selection::collapsed(pos(0, 0));
        let invalid = [
            FormatAction::ImageInsert {
                destination: String::new(),
                alt: "x".into(),
                id: None,
                width_percent: None,
            },
            FormatAction::ImageInsert {
                destination: "a.png".into(),
                alt: "x".into(),
                id: None,
                width_percent: Some(0),
            },
            FormatAction::ImageInsert {
                destination: "a.png".into(),
                alt: "x".into(),
                id: None,
                width_percent: Some(101),
            },
            FormatAction::ImageInsert {
                destination: "a.png".into(),
                alt: "x".into(),
                id: Some("no válido".into()),
                width_percent: None,
            },
        ];
        for action in invalid {
            assert_eq!(plan("", at, &action), None, "{action:?}");
        }
    }

    #[test]
    fn imagen_editar_conserva_titulo_y_atributos_ajenos() {
        let text = "ver ![Foto](assets/a.png \"Título\") {#fig:a \"cap\" width=50% k=v} ya";
        let cursor = text.find("Foto").unwrap() + 1;
        let action = FormatAction::ImageEdit {
            destination: "assets/a.png".into(),
            alt: "Otra".into(),
            id: Some("fig:a".into()),
            width_percent: Some(50),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("ver ![Otra](assets/a.png \"Título\") {#fig:a \"cap\" width=50% k=v} ya".to_string())
        );
    }

    #[test]
    fn imagen_editar_ruta_y_ancho() {
        let text = "![x](old.png) {width=50%}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "new.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: Some(30),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](new.png) {width=30%}".to_string())
        );
    }

    #[test]
    fn imagen_editar_id_inserta_y_borra_bloque() {
        // Sin atributos: pedir id añade el bloque.
        let text = "![x](a.png)";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: Some("fig:x".into()),
            width_percent: None,
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](a.png) {#fig:x}".to_string())
        );
        // Quitar id y ancho deja el bloque vacío: se elimina entero.
        let text = "![x](a.png) {#fig:x}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](a.png)".to_string())
        );
    }

    #[test]
    fn imagen_editar_desde_los_atributos() {
        // El cursor en el bloque `{...}` también está «en la imagen».
        let text = "![x](a.png) {#fig:x}";
        let cursor = text.find("fig").unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "y".into(),
            id: Some("fig:x".into()),
            width_percent: None,
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![y](a.png) {#fig:x}".to_string())
        );
    }

    #[test]
    fn imagen_editar_fuera_de_constructo_o_igual_no_aplica() {
        let text = "hola ![x](a.png)";
        let cursor = text.find("hola").unwrap();
        let action = FormatAction::ImageEdit {
            destination: "b.png".into(),
            alt: "y".into(),
            id: None,
            width_percent: None,
        };
        let at = Selection::collapsed(sel_of(text, cursor, cursor).anchor);
        assert_eq!(plan(text, at, &action), None);

        // Sin cambios respecto al estado actual: no-op.
        let text = "![x](a.png) {#fig:x width=50%}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: Some("fig:x".into()),
            width_percent: Some(50),
        };
        assert_eq!(
            plan(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            None
        );
    }

    #[test]
    fn imagen_alt_unicode() {
        let text = "ñandú";
        let action = FormatAction::ImageInsert {
            destination: "a.png".into(),
            alt: "ñandú".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied(text, sel_of(text, 0, text.len()), &action),
            Some("![ñandú](a.png)".to_string())
        );
    }

    // ---- Tablas (§35, §26.3, §28.2) ----

    #[test]
    fn tabla_insertar_esqueleto() {
        let action = FormatAction::TableInsert { rows: 1, cols: 2 };
        assert_eq!(
            applied("", Selection::collapsed(pos(0, 0)), &action),
            Some("|     |     |\n| --- | --- |\n|     |     |".to_string())
        );
        // Filas o columnas fuera de rango.
        for action in [
            FormatAction::TableInsert { rows: 0, cols: 2 },
            FormatAction::TableInsert { rows: 51, cols: 2 },
            FormatAction::TableInsert { rows: 1, cols: 0 },
            FormatAction::TableInsert { rows: 1, cols: 9 },
        ] {
            assert_eq!(plan("", Selection::collapsed(pos(0, 0)), &action), None);
        }
    }

    #[test]
    fn tabla_atributos_crea_linea_y_conserva_la_tabla() {
        let text = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let cursor = text.find('1').unwrap();
        let action = FormatAction::TableAttrs {
            id: Some("tbl:x".into()),
            caption: Some("Resultados".into()),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("| A | B |\n| --- | --- |\n| 1 | 2 |\n{#tbl:x \"Resultados\"}\n".to_string())
        );
    }

    #[test]
    fn tabla_atributos_edita_conservando_lo_no_tocado() {
        let text = "| A |\n| --- |\n{#tbl:x \"Res\"}\n";
        let cursor = text.find('A').unwrap();
        let at = Selection::collapsed(sel_of(text, cursor, cursor).anchor);
        // Solo cambia el id.
        let action = FormatAction::TableAttrs {
            id: Some("tbl:y".into()),
            caption: Some("Res".into()),
        };
        assert_eq!(
            applied(text, at, &action),
            Some("| A |\n| --- |\n{#tbl:y \"Res\"}\n".to_string())
        );
        // Solo se borra la caption.
        let action = FormatAction::TableAttrs {
            id: Some("tbl:x".into()),
            caption: None,
        };
        assert_eq!(
            applied(text, at, &action),
            Some("| A |\n| --- |\n{#tbl:x}\n".to_string())
        );
    }

    #[test]
    fn tabla_atributos_vacios_eliminan_la_linea() {
        let text = "| A |\n| --- |\n{#tbl:x \"Res\"}\n";
        let cursor = text.find('A').unwrap();
        let action = FormatAction::TableAttrs {
            id: None,
            caption: None,
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("| A |\n| --- |\n".to_string())
        );
    }

    #[test]
    fn tabla_atributos_desde_la_linea_de_atributos() {
        let text = "| A |\n| --- |\n{#tbl:x \"Res\"}\n";
        let cursor = text.find("tbl").unwrap();
        let action = FormatAction::TableAttrs {
            id: Some("tbl:z".into()),
            caption: Some("Res".into()),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("| A |\n| --- |\n{#tbl:z \"Res\"}\n".to_string())
        );
    }

    #[test]
    fn tabla_atributos_fuera_de_tabla_o_sin_cambios_no_aplican() {
        let action = FormatAction::TableAttrs {
            id: Some("tbl:x".into()),
            caption: None,
        };
        assert_eq!(plan("hola", sel_of("hola", 0, 4), &action), None);
        let text = "| A |\n| --- |\n{#tbl:x \"Res\"}\n";
        let cursor = text.find('A').unwrap();
        let action = FormatAction::TableAttrs {
            id: Some("tbl:x".into()),
            caption: Some("Res".into()),
        };
        assert_eq!(
            plan(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            None
        );
    }

    #[test]
    fn contexto_imagen_y_tabla() {
        let text = "![Foto](a.png \"T\") {#fig:a width=40%}\n\n| A | B |\n| --- | ---: |\n| 1 | 2 |\n{#tbl:x \"Res\"}\n";
        let cursor = text.find("Foto").unwrap();
        let image = context(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor))
            .image
            .expect("imagen");
        assert_eq!(image.destination, "a.png");
        assert_eq!(image.alt, "Foto");
        assert_eq!(image.title.as_deref(), Some("T"));
        assert_eq!(image.id.as_deref(), Some("fig:a"));
        assert_eq!(image.width_percent, Some(40));

        let cursor = text.find('1').unwrap();
        let table = context(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor))
            .table
            .expect("tabla");
        assert_eq!(table.columns, 2);
        assert_eq!(table.id.as_deref(), Some("tbl:x"));
        assert_eq!(table.caption.as_deref(), Some("Res"));
    }

    #[test]
    fn imagen_edicion_es_una_transaccion_con_undo_redo() {
        use crate::document::Document;
        use crate::Editor;

        let text = "![x](a.png) {#fig:x}";
        let cursor = text.find('x').unwrap();
        let selection = Selection::collapsed(sel_of(text, cursor, cursor).anchor);
        let action = FormatAction::ImageEdit {
            destination: "b.png".into(),
            alt: "y".into(),
            id: Some("fig:x".into()),
            width_percent: None,
        };
        let plan = plan(text, selection, &action).unwrap();
        let mut editor = Editor::new(Document::from_text(None, text.to_string()));
        editor.set_selection(selection).unwrap();
        editor
            .replace_all_selecting(&plan.edits, plan.selection)
            .unwrap();
        assert_eq!(editor.document().content(), "![y](b.png) {#fig:x}");
        assert!(editor.undo());
        assert_eq!(editor.document().content(), text);
        assert!(editor.redo());
        assert_eq!(editor.document().content(), "![y](b.png) {#fig:x}");
    }

    // ---- Regresiones del review de #30 ----

    #[test]
    fn atributos_en_orden_inverso_no_solapan_los_borrados() {
        // Limpiar id y ancho cuyos tokens están en orden inverso al de los
        // campos no debe solapar los espacios ni provocar cortes inválidos.
        let text = "![x](a.png) {width=50% #fig:x}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](a.png)".to_string())
        );
    }

    #[test]
    fn bloque_de_atributos_con_tokens_nuevos_no_se_elimina() {
        // Sustituir el id por un ancho deja un bloque con contenido nuevo:
        // no se puede borrar el bloque entero.
        let text = "![x](a.png) {#fig:x}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: Some(50),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](a.png) {width=50% }".to_string())
        );
    }

    #[test]
    fn tabla_atributos_con_crlf_se_detectan() {
        let text = "| A |\r\n| --- |\r\n{#tbl:x \"Res\"}\r\n";
        let cursor = text.find('A').unwrap();
        let action = FormatAction::TableAttrs {
            id: Some("tbl:y".into()),
            caption: Some("Res".into()),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("| A |\r\n| --- |\r\n{#tbl:y \"Res\"}\r\n".to_string())
        );
    }

    #[test]
    fn atributos_sin_separacion_no_consumen_atributos_ajenos() {
        // `{width=50%"cap"}` son dos tokens: editar el ancho no toca el caption.
        let text = "![x](a.png) {width=50%\"cap\"}";
        let cursor = text.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: Some(30),
        };
        assert_eq!(
            applied(text, Selection::collapsed(sel_of(text, cursor, cursor).anchor), &action),
            Some("![x](a.png) {width=30%\"cap\"}".to_string())
        );
    }

    #[test]
    fn ruta_con_espacios_se_serializa_entre_angulares() {
        // El destino con espacios debe seguir siendo sintaxis detectable.
        let action = FormatAction::ImageInsert {
            destination: "assets/my photo.png".into(),
            alt: "x".into(),
            id: None,
            width_percent: None,
        };
        let result = applied("", Selection::collapsed(pos(0, 0)), &action)
            .unwrap();
        assert_eq!(result, "![x](<assets/my photo.png>)");
        // Ida y vuelta: la imagen sigue detectándose y se edita el alt.
        let cursor = result.find('x').unwrap();
        let action = FormatAction::ImageEdit {
            destination: "assets/my photo.png".into(),
            alt: "y".into(),
            id: None,
            width_percent: None,
        };
        assert_eq!(
            applied(&result, Selection::collapsed(sel_of(&result, cursor, cursor).anchor), &action),
            Some("![y](<assets/my photo.png>)".to_string())
        );
    }

    #[test]
    fn enlace_con_espacios_se_serializa_entre_angulares() {
        let text = "ab";
        let action = FormatAction::Link {
            url: "my page.md".into(),
        };
        assert_eq!(
            applied(text, Selection::collapsed(pos(0, 2)), &action),
            Some("ab[texto](<my page.md>)".to_string())
        );
    }

    #[test]
    fn seleccion_que_abarca_alt_y_atributos_detecta_la_imagen() {
        // La selección empieza en el alt y termina dentro de `{...}`: es la
        // misma imagen, no dos constructos sueltos.
        let text = "![x](a.png) {#fig:x}";
        let selection = sel_of(text, text.find('x').unwrap(), text.find("fig").unwrap() + 2);
        let action = FormatAction::ImageEdit {
            destination: "a.png".into(),
            alt: "y".into(),
            id: Some("fig:x".into()),
            width_percent: None,
        };
        assert_eq!(
            applied(text, selection, &action),
            Some("![y](a.png) {#fig:x}".to_string())
        );
    }
}

