//! Pestañas de documentos (§37).
//!
//! [`manager::TabManager`] es el modelo: mantiene los tabs (cada uno con su
//! [`minmark_document::Editor`] sobre el buffer único, §6.1, y su historial
//! Undo/Redo propio, §8.5), el tab activo y el ciclo de vida de ficheros
//! (§12) sobre un [`minmark_fs::LocalFs`] interno. [`bar::TabBar`] es la
//! barra GTK de pestañas: oculta con un solo documento abierto, con el
//! indicador `●` de cambios sin guardar junto al nombre (§37) y una «X» por
//! pestaña que cierra esa pestaña concreta (ISSUE-52).

mod bar;
mod manager;
pub mod scope;

pub use bar::TabBar;
pub use manager::TabManager;
pub use scope::{OpenTarget, WindowScope};
