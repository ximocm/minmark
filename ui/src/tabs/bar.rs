//! Barra de pestañas (§37).
//!
//! Barra `gtk::Box` horizontal con un botón por pestaña rotulado con
//! [`TabManager::title`] («● nombre» si hay cambios sin guardar). La barra se
//! oculta con un solo documento abierto (§37).

use crate::tabs::TabManager;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::rc::Rc;

/// Barra de pestañas GTK (§37). `sync` la re-renderiza desde el modelo.
pub struct TabBar {
    root: gtk::Box,
    on_select: Rc<dyn Fn(usize)>,
}

impl TabBar {
    /// Crea la barra; `on_select(index)` se invoca al pulsar la pestaña
    /// `index`. La visibilidad la fija [`TabBar::sync`] (§37).
    pub fn new(on_select: Rc<dyn Fn(usize)>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("mm-tab-bar");
        root.set_visible(false);
        Self { root, on_select }
    }

    /// Widget raíz de la barra, para insertarla en la ventana.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Re-renderiza los botones desde `manager` (§37): un botón por pestaña
    /// con `title(index)`, el activo resaltado, y `set_visible(len > 1)` para
    /// ocultar la barra con un solo documento.
    pub fn sync(&self, manager: &TabManager) {
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        for index in 0..manager.len() {
            let button = gtk::Button::with_label(&manager.title(index));
            let tab = &manager.tabs()[index];
            if let Some(path) = &tab.path {
                let mut tooltip = path.display().to_string();
                if tab.external_change {
                    tooltip.push_str("\nEl archivo cambió o se eliminó fuera de minmark. Activa esta pestaña para ver las opciones.");
                }
                button.set_tooltip_text(Some(&tooltip));
            }
            if index == manager.active_index() {
                // §37: el tab activo va resaltado.
                button.add_css_class("active");
            }
            let on_select = Rc::clone(&self.on_select);
            button.connect_clicked(move |_| on_select(index));
            self.root.append(&button);
        }
        self.root.set_visible(manager.len() > 1);
    }
}
