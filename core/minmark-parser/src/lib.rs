//! Parser Markdown de minmark (§7).
//!
//! Wrapper sobre pulldown-cmark (parser único, §4.2; versión benchmarkada en
//! ISSUE-010) que produce la AST compartida de [`minmark_document`].
//!
//! Capas:
//! 1. `blocks`: front matter (§21), callouts `:::` (§22), math `$$` (§24) y
//!    posiciones de atributos (§7.4.3);
//! 2. `events`: estructura CommonMark + GFM (§7.1) de pulldown-cmark;
//! 3. `inlines`: gramática de extensiones inline (§7.4) sobre el fuente crudo.
//!
//! Reglas normativas:
//! - §7.2: el HTML embebido nunca se interpreta; se preserva como texto.
//! - §6.2: cada bloque conserva su [`Span`] en el fuente original y parsear
//!   nunca modifica el buffer (la AST es derivada, §6.1).

mod attrs;
mod blocks;
mod events;
mod highlight;
mod inlines;
mod visual;

use minmark_document::Ast;

pub use highlight::{Highlight, HighlightKind};
pub use visual::{CalloutTone, VisualPlan, VisualSpan, VisualStyle};

/// Opciones de parseo del dialecto (§7.3, §58.2 niveles de proyecto).
#[derive(Debug, Clone, Default)]
pub struct ParseOptions<'a> {
    /// Nombres de colores personalizados del proyecto (§23, §58.2).
    pub custom_colors: &'a [String],
    /// Tipos `:::` personalizados del proyecto (§22, §58.2).
    pub custom_callout_types: &'a [String],
    /// Wiki links activados (§19.3). Por defecto: desactivados.
    pub wiki_links: bool,
}

/// Parsea un documento Markdown completo a la AST compartida (§7).
pub fn parse(source: &str) -> Ast {
    parse_with(source, &ParseOptions::default())
}

/// Parsea con las extensiones configurables (§7.3).
pub fn parse_with(source: &str, options: &ParseOptions) -> Ast {
    blocks::parse_document(source, 0, options, true).0
}

/// Tokens de syntax highlighting estilo CodeMirror (ISSUE-015).
///
/// Rangos de fuente con tipo semántico, derivados de la misma gramática que
/// la AST (parser único, §4.2). El texto plano no produce token.
pub fn highlight(source: &str) -> Vec<Highlight> {
    highlight_with(source, &ParseOptions::default())
}

/// Highlighting con las extensiones configurables (§7.3).
pub fn highlight_with(source: &str, options: &ParseOptions) -> Vec<Highlight> {
    blocks::parse_document(source, 0, options, true).1
}

/// Plan visual del modo Visual (ISSUE-030, §8.2–§8.4): tramos de sintaxis
/// ocultable (§8.3) y de contenido semántico sobre el fuente, más los
/// elementos (bloques) para revelar la sintaxis del elemento editado.
pub fn visual(source: &str) -> VisualPlan {
    visual_with(source, &ParseOptions::default())
}

/// Plan visual con las extensiones configurables (§7.3).
pub fn visual_with(source: &str, options: &ParseOptions) -> VisualPlan {
    visual::build(source, options)
}
