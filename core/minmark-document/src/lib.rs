//! Modelo documental de minmark (§5.1, §6).
//!
//! - [`Document`]: buffer Markdown único, fuente de verdad (§6.1).
//! - [`Ast`]: modelo sintáctico compartido entre parser, renderer y PDF.
//! - [`Editor`]: ediciones con undo/redo único (§8.5) y posiciones
//!   semánticas (§8.6); [`Matcher`]: búsqueda/reemplazo del archivo actual
//!   (§40.1, ISSUE-027) con motor regex propio std-only.
//! - [`LineEnding`]: preservación LF/CRLF (§12.6).
//!
//! Crate sin dependencias: solo `std` (§4.1).

mod ast;
mod document;
mod editor;
mod position;
mod regex;
mod references;
mod search;

pub use ast::{
    Align, Ast, Attributes, Block, BlockKind, CitationItem, Inline, ListItem, Span, Substitution,
};
pub use document::{Document, LineEnding};
pub use editor::{EditError, Editor};
pub use position::{Position, Range, Selection};
pub use references::{
    apply_replacements, find_headings, find_markdown_references, find_toml_references, DestForm,
    Heading, RefKind, Reference,
};
pub use search::{Matcher, SearchError, SearchOptions, TextMatch};
