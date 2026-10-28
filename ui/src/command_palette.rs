//! Command Palette §38 (fase 7 #29, §39: `Ctrl+Shift+P`).
//!
//! Ventana fuzzy sobre los labels de [`ALL_COMMANDS`] (la lista completa de
//! §38) con el patrón de [`crate::quickopen`]: campo de texto + lista de
//! resultados; ↑/↓ navega, Enter o doble click aceptan, Esc cierra. Al
//! aceptar, `window.activate_action(cmd.action_id(), None)` — el MISMO
//! camino `win.*` que menú, atajos y popover, con destino único
//! `TabManager::apply_format_active` (crate::tabs::TabManager).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use minmark_fs::fuzzy;

use crate::menus::actions::{Command, ALL_COMMANDS};

/// Filtro fuzzy sobre los labels de los comandos (§38): consulta vacía ⇒
/// todos en orden de menú; con consulta ⇒ coincidencias por score (estable:
/// los empates conservan el orden §36).
fn fuzzy_filter(query: &str) -> Vec<Command> {
    let query = query.trim();
    if query.is_empty() {
        return ALL_COMMANDS.to_vec();
    }
    let mut hits: Vec<(i64, Command)> = ALL_COMMANDS
        .iter()
        .filter_map(|cmd| Some((fuzzy::fuzzy_match(cmd.label(), query)?.score, *cmd)))
        .collect();
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.into_iter().map(|(_, cmd)| cmd).collect()
}

/// Paleta de comandos §38 con su ventana modal propia (reutilizable).
pub struct CommandPalette {
    window: gtk::Window,
    input: gtk::Entry,
    list: gtk::ListBox,
    /// Resultados compartidos entre la paleta y sus callbacks GTK (`Rc`, no
    /// `RefCell` suelto: un `clone` de `RefCell` copiaría el estado y los
    /// handlers mutarían otra celda).
    results: Rc<RefCell<Vec<Command>>>,
    /// Ventana de la aplicación que activa los `win.*` al aceptar (§38).
    parent: Rc<RefCell<Option<gtk::Window>>>,
}

impl CommandPalette {
    /// Crea la paleta oculta (presenta [`Self::present`]).
    pub fn new() -> Self {
        let window = gtk::Window::new();
        window.set_title(Some("Command Palette"));
        window.set_modal(true);
        window.set_default_size(560, 420);
        // Nombre accesible (§38: la paleta es una superficie de interfaz
        // identificable para AT).
        window.update_property(&[gtk::accessible::Property::Label("Command Palette")]);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(8);
        root.set_margin_end(8);

        let input = gtk::Entry::new();
        input.set_placeholder_text(Some("Filtrar comandos"));
        root.append(&input);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_vexpand(true);
        scroll.set_min_content_height(320);
        crate::list_scroll::follow_selection(&list);
        root.append(&scroll);

        window.set_child(Some(&root));

        // Cerrar la ventana (✕ de la barra de título) solo la OCULTA
        // (patrón Quick Open §41): la paleta es reutilizable y el siguiente
        // `Ctrl+Shift+P` debe volver a presentarla — destruirla dejaría el
        // próximo `present()` sobre un objeto muerto.
        window.connect_close_request(move |window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });

        let palette = CommandPalette {
            window: window.clone(),
            input: input.clone(),
            list: list.clone(),
            results: Rc::new(RefCell::new(Vec::new())),
            parent: Rc::new(RefCell::new(None)),
        };

        // Recalcula y repinta ante cada pulsación (mismo ritmo que Quick
        // Open §41: sin E/S por pulsación §47.5).
        {
            let list = palette.list.clone();
            let results = palette.results.clone();
            input.connect_changed(move |entry| {
                let found = fuzzy_filter(&entry.text());
                *results.borrow_mut() = found;
                repaint(&list, &results.borrow());
            });
        }

        // ↑/↓ navega; Esc cierra (§38: cancelar no toca el documento).
        {
            let controller = gtk::EventControllerKey::new();
            let key_window = window.clone();
            let list = list.clone();
            controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    key_window.set_visible(false);
                    return glib::Propagation::Stop;
                }
                if key == gtk::gdk::Key::Up || key == gtk::gdk::Key::Down {
                    let delta = if key == gtk::gdk::Key::Up { -1 } else { 1 };
                    let actual = list.selected_row().map(|row| row.index()).unwrap_or(0);
                    if let Some(row) = list.row_at_index((actual + delta).max(0)) {
                        list.select_row(Some(&row));
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(controller);
        }

        // Enter acepta la fila seleccionada.
        {
            let window = palette.window.clone();
            let list = palette.list.clone();
            let results = palette.results.clone();
            let parent = palette.parent.clone();
            input.connect_activate(move |_| {
                activate(&window, &list, &results, &parent);
            });
        }

        // Doble click sobre una fila: acepta ESA fila.
        {
            let window = palette.window.clone();
            let results = palette.results.clone();
            let parent = palette.parent.clone();
            palette.list.connect_row_activated(move |list, row| {
                if let Some(row) = list.row_at_index(row.index()) {
                    list.select_row(Some(&row));
                }
                activate(&window, list, &results, &parent);
            });
        }

        palette
    }

    /// Ancla la paleta a la ventana de la aplicación (modal §36) y guarda
    /// dónde activar los `win.*` al aceptar (§38).
    pub fn set_transient_for(&self, parent: &gtk::Window) {
        self.window.set_transient_for(Some(parent));
        *self.parent.borrow_mut() = Some(parent.clone());
    }

    /// Muestra la paleta con el campo limpio (fuzzy desde cero §38).
    pub fn present(&self) {
        self.input.set_text("");
        self.refresh();
        self.window.present();
        self.input.grab_focus();
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }

    /// Acepta la fila seleccionada: cierra y activa la acción `win.*` del
    /// comando sobre la ventana de la aplicación (§38).
    #[cfg(test)]
    fn activate_selected(&self) {
        activate(&self.window, &self.list, &self.results, &self.parent);
    }

    /// Repinta la lista con `results` y marca la primera fila.
    fn refresh(&self) {
        *self.results.borrow_mut() = fuzzy_filter(&self.input.text());
        repaint(&self.list, &self.results.borrow());
    }

    #[cfg(test)]
    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    /// Pide el cierre de la ventana como la ✕ de la barra de título
    /// (close-request → la paleta se oculta, no se destruye).
    #[cfg(test)]
    pub fn test_close(&self) {
        self.window.close();
    }

    #[cfg(test)]
    pub fn test_key(&self, key: gtk::gdk::Key) {
        let controllers = self.window.observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(controller) = controllers
                .item(i)
                .and_then(|c| c.downcast::<gtk::EventControllerKey>().ok())
            {
                if controller.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
                ) {
                    break;
                }
            }
        }
    }

    #[cfg(test)]
    pub fn test_filter(&self, query: &str) {
        self.input.set_text(query);
    }

    #[cfg(test)]
    pub fn test_activate(&self) {
        self.activate_selected();
    }

    #[cfg(test)]
    pub fn test_result_count(&self) -> usize {
        self.results.borrow().len()
    }

    #[cfg(test)]
    pub fn test_selected_label(&self) -> Option<String> {
        let row = self.list.selected_row()?;
        let label = row.child()?.downcast::<gtk::Label>().ok()?;
        Some(label.text().to_string())
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

/// Acepta la fila seleccionada: cierra la paleta y activa la acción `win.*`
/// de su comando sobre la ventana de la aplicación (§38: la paleta ejecuta
/// las MISMAS acciones que menú y atajos, con `activate_action(id, None)`).
fn activate(
    window: &gtk::Window,
    list: &gtk::ListBox,
    results: &RefCell<Vec<Command>>,
    parent: &RefCell<Option<gtk::Window>>,
) {
    let selected = list
        .selected_row()
        .and_then(|row| results.borrow().get(row.index() as usize).copied());
    window.set_visible(false);
    let Some(cmd) = selected else { return };
    let parent = parent.borrow().clone();
    if let Some(window) = parent {
        let _ = window.activate_action(cmd.action_id(), None);
    }
}

/// Repinta la lista de resultados y selecciona la primera fila.
fn repaint(list: &gtk::ListBox, results: &[Command]) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    for cmd in results {
        let label = gtk::Label::new(Some(cmd.label()));
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        row.update_property(&[gtk::accessible::Property::Label(cmd.label())]);
        list.append(&row);
    }
    if results.is_empty() {
        let label = gtk::Label::new(Some("Sin resultados"));
        label.set_xalign(0.0);
        list.append(&label);
    } else if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtro_fuzzy_sobre_los_labels_38() {
        // Consulta vacía: la paleta lista TODOS los comandos §38.
        assert_eq!(fuzzy_filter("").len(), ALL_COMMANDS.len());
        assert_eq!(fuzzy_filter("   ").len(), ALL_COMMANDS.len());

        // Fuzzy sobre labels: «bol» encuentra Bold y «link» Find/Link.
        let bold = fuzzy_filter("bold");
        assert_eq!(bold[0], Command::FormatStrong, "el match exacto primero");
        assert!(fuzzy_filter("bld").contains(&Command::FormatStrong));
        let link = fuzzy_filter("link");
        assert!(link.contains(&Command::InsertLink));

        // Sin coincidencias: lista vacía (la UI muestra «Sin resultados»).
        assert!(fuzzy_filter("zzzzzzzz").is_empty());
    }

    #[test]
    fn la_paleta_cubre_todos_los_comandos_con_label_unico() {
        // §38: la paleta puede ejecutar todas las acciones de la
        // aplicación; cada label de ALL_COMMANDS filtra su propio comando.
        for cmd in ALL_COMMANDS {
            let encontrados = fuzzy_filter(cmd.label());
            assert!(
                encontrados.contains(cmd),
                "la paleta no encuentra {} por su label",
                cmd.label()
            );
        }
    }
}
