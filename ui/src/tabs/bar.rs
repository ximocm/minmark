//! Barra de pestañas (§37, ISSUE-52).
//!
//! Barra `gtk::Box` horizontal con una caja por pestaña: un botón con el
//! título ([`TabManager::title`], «● nombre» si hay cambios sin guardar) y
//! una «X» que cierra **esa** pestaña concreta (ISSUE-52). La barra se oculta
//! con un solo documento abierto (§37).

use crate::tabs::TabManager;
use gtk4 as gtk;
use gtk4::prelude::*;
use std::rc::Rc;

/// Barra de pestañas GTK (§37). `sync` la re-renderiza desde el modelo.
pub struct TabBar {
    root: gtk::Box,
    on_select: Rc<dyn Fn(usize)>,
    /// `on_close(index)` cierra la pestaña `index` (ISSUE-52): el modelo solo
    /// cambia el tab activo si se cierra él (§37).
    on_close: Rc<dyn Fn(usize)>,
}

impl TabBar {
    /// Crea la barra; `on_select(index)` se invoca al pulsar la pestaña
    /// `index` y `on_close(index)` al pulsar su «X». La visibilidad la fija
    /// [`TabBar::sync`] (§37).
    pub fn new(on_select: Rc<dyn Fn(usize)>, on_close: Rc<dyn Fn(usize)>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("mm-tab-bar");
        root.set_visible(false);
        Self {
            root,
            on_select,
            on_close,
        }
    }

    /// Widget raíz de la barra, para insertarla en la ventana.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Re-renderiza las pestañas desde `manager` (§37): una caja por pestaña
    /// con `title(index)` (el activo resaltado) y su «X» de cierre, y
    /// `set_visible(len > 1)` para ocultar la barra con un solo documento.
    pub fn sync(&self, manager: &TabManager) {
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        for index in 0..manager.len() {
            let title = manager.title(index);
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            cell.add_css_class("mm-tab");

            let button = gtk::Button::with_label(&title);
            let tab = &manager.tabs()[index];
            if let Some(path) = &tab.path {
                let mut tooltip = path.display().to_string();
                if tab.external_change {
                    tooltip.push_str("\nEl archivo cambió o se eliminó fuera de minmark. Activa esta pestaña para ver las opciones.");
                }
                button.set_tooltip_text(Some(&tooltip));
            }
            let on_select = Rc::clone(&self.on_select);
            button.connect_clicked(move |_| on_select(index));
            cell.append(&button);

            // ISSUE-52: la «X» cierra esta pestaña concreta. El icono no
            // aporta etiqueta accesible: se fija nombre accesible y tooltip,
            // y el área de clic la agranda el CSS (`mm-tab-close`).
            let close = gtk::Button::from_icon_name("window-close-symbolic");
            close.add_css_class("mm-tab-close");
            close.set_tooltip_text(Some(&format!("Cerrar «{title}»")));
            close.update_property(&[gtk::accessible::Property::Label(&format!(
                "Cerrar «{title}»"
            ))]);
            let on_close = Rc::clone(&self.on_close);
            close.connect_clicked(move |_| on_close(index));
            cell.append(&close);

            if index == manager.active_index() {
                // §37: el tab activo va resaltado.
                cell.add_css_class("active");
            }
            self.root.append(&cell);
        }
        self.root.set_visible(manager.len() > 1);
    }
}
