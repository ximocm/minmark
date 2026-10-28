//! Menús, atajos y diálogos de minmark (ISSUE-020).
//!
//! - [`actions`]: comandos explícitos y observables (§3.7), barra de menú §36
//!   (File, Edit, View, Insert, Format, Project, Help) y atajos de teclado §39.
//! - [`dialogs`]: diálogos GTK4 asíncronos — abrir/guardar (§12.4), selector
//!   de imagen §18.1 y formulario multi-campo #30, y confirmaciones
//!   explícitas ante descartes (§3.7) y cambios externos (§12.7).
//!
//! El integrador (`main.rs`) declara `mod menus;` y consume esta API:
//! [`build_menubar`] + [`install_actions`] para la ventana, [`Commands`] como
//! receptor de comandos y [`dialogs`] para los diálogos.

pub mod actions;
pub mod dialogs;

pub use actions::{build_menubar, install_actions, Commands, ACCELERATORS};
