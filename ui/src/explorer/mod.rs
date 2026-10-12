//! Explorador de archivos: panel lateral con árbol perezoso (ISSUE-022/023).
//!
//! - §14.3: panel lateral ocultable (`Ctrl+B`), ancho redimensionable y
//!   estado restaurable (la persistencia vive en [`state`]).
//! - §14.4/§14.5: `Show All Files` y `Show Hidden Files` por ventana.
//! - §14.7: los symlinks se listan; las carpetas enlazadas no se expanden.
//! - §14.8: menú contextual con New Markdown, New Folder, `Rename`, Move,
//!   Duplicate, Delete y Copy Relative Path; el nodo del click es el
//!   destinatario (con click en vacío → raíz del proyecto).
//! - §47.1/§47.5: los directorios se leen en una tarea aparte (un hilo por
//!   lectura, [`minmark_fs::read_dir_entries`]) y el resultado se integra en
//!   el bucle principal con `glib::idle_add_once`: la UI jamás hace E/S y el
//!   árbol crece a demanda, directorio a directorio.
//! - Activar un archivo lo abre como pestaña vía `OpenTarget` (ISSUE-021);
//!   activar una carpeta la expande o colapsa.

mod model;
mod state;

pub use state::{load as load_state, save as save_state, ExplorerState};

use model::{ExplorerFilters, Row, TreeModel};
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use minmark_fs::{read_dir_entries, TreeEntry};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

/// Indento por nivel de profundidad en el árbol.
const INDENT_PX: i32 = 16;

/// Operaciones del menú contextual §14.8 (ISSUE-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorerAction {
    NewMarkdown,
    NewFolder,
    Rename,
    Move,
    Duplicate,
    Delete,
    CopyRelativePath,
    /// Import Markdown (§15, ISSUE-024): el nodo del click es la carpeta
    /// destino dentro del proyecto.
    ImportMarkdown,
}

/// Menú contextual §14.8 en datos puros: `(acción, etiqueta)`; `Rename`
/// conserva su nombre inglés tal y como lo lista la SRS.
const CONTEXT_MENU_SPEC: &[(ExplorerAction, &str)] = &[
    (ExplorerAction::NewMarkdown, "New Markdown"),
    (ExplorerAction::NewFolder, "New Folder"),
    (ExplorerAction::ImportMarkdown, "Import Markdown"),
    (ExplorerAction::Rename, "Rename"),
    (ExplorerAction::Move, "Move"),
    (ExplorerAction::Duplicate, "Duplicate"),
    (ExplorerAction::Delete, "Delete"),
    (ExplorerAction::CopyRelativePath, "Copy Relative Path"),
];

/// Nombre de la acción en el grupo `explorer.*`.
fn action_name(action: ExplorerAction) -> &'static str {
    match action {
        ExplorerAction::NewMarkdown => "new-markdown",
        ExplorerAction::NewFolder => "new-folder",
        ExplorerAction::Rename => "rename",
        ExplorerAction::Move => "move",
        ExplorerAction::Duplicate => "duplicate",
        ExplorerAction::Delete => "delete",
        ExplorerAction::CopyRelativePath => "copy-relative-path",
        ExplorerAction::ImportMarkdown => "import-markdown",
    }
}

/// Panel lateral del explorador (§14.3): cabecera con los toggles §14.4/§14.5
/// y una lista de filas planas indentadas ([`Row`]) sobre el modelo puro.
pub struct ExplorerPane {
    widget: gtk::Box,
    model: Arc<Mutex<TreeModel>>,
    store: gtk::gio::ListStore,
    root: PathBuf,
    /// Notificador del toggle `Show All Files` §14.4 (ISSUE-029: la paleta
    /// Quick Open consulta con `show_all_files`).
    on_show_all_changed: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>>,
}

impl ExplorerPane {
    /// Crea el panel para la raíz `root` (ventana de proyecto) y comienza su
    /// carga incremental (§47.5). `on_open(path)` abre un archivo activado;
    /// `on_action(acción, nodo)` ejecuta una operación §14.8 sobre el nodo
    /// del menú contextual (`None` = raíz del proyecto).
    pub fn new(
        root: &Path,
        on_open: Rc<dyn Fn(PathBuf)>,
        on_action: Rc<dyn Fn(ExplorerAction, Option<PathBuf>)>,
    ) -> Self {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let model: Arc<Mutex<TreeModel>> = Arc::new(Mutex::new(TreeModel::new(root.clone())));
        let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("mm-explorer");

        // Cabecera §14.4/§14.5: toggles por ventana.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.set_margin_top(4);
        header.set_margin_bottom(4);
        header.set_margin_start(6);
        header.set_margin_end(6);
        let show_all = gtk::CheckButton::with_label("Show All Files");
        let show_hidden = gtk::CheckButton::with_label("Show Hidden Files");
        header.append(&show_all);
        header.append(&show_hidden);
        widget.append(&header);

        // Menú contextual §14.8 sobre el nodo del click (None = raíz).
        let context_target: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let popover = build_context_menu(&widget, &context_target, &on_action);

        // Lista de filas: un botón plano por fila, indentado por profundidad.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let model = model.clone();
            let store = store.clone();
            let on_open = Rc::clone(&on_open);
            let host = widget.clone();
            let popover = popover.clone();
            let context_target = Rc::clone(&context_target);
            move |_, object| {
                let list_item = object
                    .downcast_ref::<gtk::ListItem>()
                    .expect("el factory de lista recibe gtk::ListItem");
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let button = gtk::Button::new();
                button.set_child(Some(&label));
                button.set_has_frame(false);
                button.set_halign(gtk::Align::Fill);

                // Un solo handler por widget (ListView recicla widgets): lee
                // la fila vigente del ListItem en el momento del click.
                let weak_item = list_item.downgrade();
                let model = model.clone();
                let store = store.clone();
                let on_open = Rc::clone(&on_open);
                button.connect_clicked(move |_| {
                    let Some(list_item) = weak_item.upgrade() else { return };
                    let Some(row) = current_row(&list_item) else { return };
                    if row.expandable {
                        toggle_dir(&model, &store, &row.path);
                    } else if !row.is_dir {
                        // §14.7: las carpetas enlazadas (no expandibles) no
                        // abren nada; los archivos van a `OpenTarget`.
                        on_open(row.path.clone());
                    }
                });

                // Click derecho §14.8: el nodo de la fila es el destinatario.
                let gesture = gtk::GestureClick::new();
                gesture.set_button(3);
                {
                    let weak_item = list_item.downgrade();
                    let host = host.clone();
                    let popover = popover.clone();
                    let context_target = Rc::clone(&context_target);
                    gesture.connect_pressed(move |gesture, _, x, y| {
                        let target = weak_item
                            .upgrade()
                            .and_then(|list_item| current_row(&list_item))
                            .map(|row| row.path.clone());
                        *context_target.borrow_mut() = target;
                        popup_at(&popover, &host, &gesture, x, y);
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    });
                }
                button.add_controller(gesture);
                list_item.set_child(Some(&button));
            }
        });
        factory.connect_bind(|_, object| {
            let list_item = object
                .downcast_ref::<gtk::ListItem>()
                .expect("el factory de lista recibe gtk::ListItem");
            let Some(row) = current_row(list_item) else { return };
            let Some(button) = list_item.child().and_downcast::<gtk::Button>() else {
                return;
            };
            let Some(label) = button.child().and_downcast::<gtk::Label>() else {
                return;
            };
            let arrow = if !row.expandable {
                "  "
            } else if row.expanded {
                "▾ "
            } else {
                "▸ "
            };
            label.set_text(&format!("{arrow}{}", row.name));
            button.set_tooltip_text(Some(&row.path.display().to_string()));
            button.set_margin_start(row.depth as i32 * INDENT_PX);
        });

        let selection = gtk::SingleSelection::new(Some(store.clone()));
        // Sin selección automática: activar una fila es abrir/expandir, no
        // seleccionar (§3.7: toda acción es explícita).
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let list = gtk::ListView::new(Some(selection), Some(factory));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);

        // Click derecho en vacío: operaciones sobre la raíz del proyecto.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        {
            let host = widget.clone();
            let popover = popover.clone();
            let context_target = Rc::clone(&context_target);
            gesture.connect_pressed(move |gesture, _, x, y| {
                *context_target.borrow_mut() = None;
                popup_at(&popover, &host, &gesture, x, y);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
        }
        scroll.add_controller(gesture);
        widget.append(&scroll);

        // §14.4/§14.5: cambios de filtro re-renderizan sin releer (§47.5).
        wire_filter(&show_all, &model, &store, |filters, active| {
            filters.show_all = active
        });
        wire_filter(&show_hidden, &model, &store, |filters, active| {
            filters.show_hidden = active
        });

        // §14.4 (ISSUE-029): el toggle `Show All Files` se enlaza con su
        // consumidor (Quick Open); sin consumidor, sin-op.
        let on_show_all_changed: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>> =
            Rc::new(RefCell::new(None));
        show_all.connect_toggled({
            let on_show_all_changed = on_show_all_changed.clone();
            move |check| {
                if let Some(notify) = on_show_all_changed.borrow().as_ref() {
                    notify(check.is_active());
                }
            }
        });

        // Raíz del proyecto: primera lectura a demanda (§47.5).
        request_load(&model, &store, root.clone());

        Self { widget, model, store, root, on_show_all_changed }
    }

    /// Widget raíz del panel, para insertarlo en el `Paned` de la ventana.
    pub fn widget(&self) -> gtk::Widget {
        self.widget.clone().upcast()
    }

    /// Enlaza el toggle `Show All Files` §14.4 con su consumidor (ISSUE-029:
    /// Quick Open consulta con `show_all_files`); se notifica cada cambio.
    pub fn set_on_show_all_changed(&self, on_changed: Rc<dyn Fn(bool)>) {
        *self.on_show_all_changed.borrow_mut() = Some(on_changed);
    }

    /// Recarga el árbol tras operaciones §14.8 (ISSUE-026 sustituirá esto por
    /// refresco por watcher §46). Conserva los filtros.
    pub fn refresh(&self) {
        self.model.lock().unwrap_or_else(|p| p.into_inner()).reset();
        rebuild(&self.store, &lock(&self.model));
        request_load(&self.model, &self.store, self.root.clone());
    }
}

/// Construye el `PopoverMenu` §14.8 con las acciones `explorer.*`.
fn build_context_menu(
    host: &gtk::Box,
    target: &Rc<RefCell<Option<PathBuf>>>,
    on_action: &Rc<dyn Fn(ExplorerAction, Option<PathBuf>)>,
) -> gtk::PopoverMenu {
    let menu = gtk::gio::Menu::new();
    let group = gtk::gio::SimpleActionGroup::new();
    // Secciones §14.8: crear/importar / gestionar / eliminar / copiar ruta.
    const SECTIONS: &[&[usize]] = &[&[0, 1, 2], &[3, 4, 5], &[6], &[7]];
    for section in SECTIONS {
        let submenu = gtk::gio::Menu::new();
        for &entry in *section {
            let (action, label) = CONTEXT_MENU_SPEC[entry];
            submenu.append_item(&gtk::gio::MenuItem::new(
                Some(label),
                Some(&format!("explorer.{}", action_name(action))),
            ));
            let simple = gtk::gio::SimpleAction::new(action_name(action), None);
            let on_action = Rc::clone(on_action);
            let target = Rc::clone(target);
            simple.connect_activate(move |_, _| {
                on_action(action, target.borrow().clone());
            });
            group.add_action(&simple);
        }
        menu.append_section(None::<&str>, &submenu);
    }
    host.insert_action_group("explorer", Some(&group));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(host);
    popover
}

/// Muestra el menú en las coordenadas del gesto (relativas a `host`).
fn popup_at(popover: &gtk::PopoverMenu, host: &gtk::Box, gesture: &gtk::GestureClick, x: f64, y: f64) {
    let widget = gesture
        .widget()
        .and_then(|w| w.translate_coordinates(host, x, y));
    let (x, y) = widget.unwrap_or((x, y));
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        x as i32,
        y as i32,
        1,
        1,
    )));
    popover.popup();
}

/// Fila vigente ([`Row`]) del `ListItem` reciclado.
fn current_row(list_item: &gtk::ListItem) -> Option<Row> {
    let item = list_item.item().and_downcast::<glib::BoxedAnyObject>()?;
    let row = (*item.borrow::<Row>()).clone();
    Some(row)
}

/// Conecta un toggle §14.4/§14.5: actualiza el filtro y re-renderiza.
fn wire_filter(
    check: &gtk::CheckButton,
    model: &Arc<Mutex<TreeModel>>,
    store: &gtk::gio::ListStore,
    apply: fn(&mut ExplorerFilters, bool),
) {
    let model = model.clone();
    let store = store.clone();
    check.connect_toggled(move |button| {
        let mut model = lock(&model);
        let mut filters = model.filters();
        apply(&mut filters, button.is_active());
        model.set_filters(filters);
        rebuild(&store, &model);
    });
}

/// Expande/colapsa `dir` y pide su lectura si hace falta (§47.5).
fn toggle_dir(model: &Arc<Mutex<TreeModel>>, store: &gtk::gio::ListStore, dir: &Path) {
    let needs_load = {
        let mut model = lock(model);
        let expanded = model.toggle(dir);
        expanded && model.begin_load(dir)
    };
    rebuild(store, &lock(model));
    if needs_load {
        spawn_load(model.clone(), store, dir.to_path_buf());
    }
}

/// Pide la lectura de `dir` (§47.5): `begin_load` + hilo de lectura.
fn request_load(model: &Arc<Mutex<TreeModel>>, store: &gtk::gio::ListStore, dir: PathBuf) {
    if lock(model).begin_load(&dir) {
        spawn_load(model.clone(), store, dir);
    }
}

/// Lee `dir` en un hilo aparte (§47.1: nunca bloquea la UI ni la escritura) y
/// vuelca el resultado en el bucle principal, que es el único que toca GTK.
fn spawn_load(model: Arc<Mutex<TreeModel>>, store: &gtk::gio::ListStore, dir: PathBuf) {
    let store_weak = glib::SendWeakRef::from(store.downgrade());
    std::thread::spawn(move || {
        let entries: Result<Vec<TreeEntry>, String> =
            read_dir_entries(&dir).map_err(|err| err.to_string());
        glib::idle_add_once(move || {
            {
                let mut model = lock(&model);
                model.finish_load(&dir, entries);
                if let Some(store) = store_weak.upgrade() {
                    rebuild(&store, &model);
                }
            }
        });
    });
}

/// Re-renderiza la lista completa desde el modelo (§47.5: sin releer).
fn rebuild(store: &gtk::gio::ListStore, model: &TreeModel) {
    store.remove_all();
    for row in model.rows() {
        store.append(&glib::BoxedAnyObject::new(row));
    }
}

/// Lock sin panics en la UI: un hilo envenenado no tumba la ventana.
fn lock(model: &Mutex<TreeModel>) -> MutexGuard<'_, TreeModel> {
    model.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §14.8/§15: el menú contextual cubre todas las operaciones exigidas,
    /// `Rename` conserva el nombre de la SRS e `Import Markdown` (§15) usa su
    /// etiqueta literal.
    #[test]
    fn menu_contextual_14_8_completo() {
        let etiquetas: Vec<&str> = CONTEXT_MENU_SPEC.iter().map(|(_, l)| *l).collect();
        assert_eq!(
            etiquetas,
            [
                "New Markdown",
                "New Folder",
                "Import Markdown",
                "Rename",
                "Move",
                "Duplicate",
                "Delete",
                "Copy Relative Path",
            ]
        );
        // Una acción registrada por ítem, sin duplicados.
        let mut nombres = std::collections::HashSet::new();
        for (action, _) in CONTEXT_MENU_SPEC {
            assert!(nombres.insert(action_name(*action)), "acción duplicada");
        }
    }
}
