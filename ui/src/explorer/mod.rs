//! Explorador de archivos: panel lateral con árbol perezoso (ISSUE-022/023).
//!
//! - §14.3: panel lateral ocultable (`Ctrl+B`), ancho redimensionable y
//!   estado restaurable (la persistencia vive en [`state`]).
//! - §14.4/§14.5: `Show All Files` y `Show Hidden Files` por ventana; los
//!   toggles viven en el menú **View** como acciones stateful `win.*` (#70)
//!   y entran al panel con [`ExplorerPane::set_show_all`] /
//!   [`ExplorerPane::set_show_hidden`].
//! - §14.7: los symlinks se listan; las carpetas enlazadas no se expanden.
//! - §14.8: menú contextual con New Markdown, New Folder, `Rename`, Move,
//!   Duplicate, Delete, Copy Relative Path y `Open With…` (#72, solo
//!   archivos: ni la raíz ni las carpetas ofrecen selector de apps); el
//!   nodo del click es el destinatario (con click en vacío → raíz del
//!   proyecto).
//! - §47.1/§47.5: los directorios se leen en una tarea aparte (un hilo por
//!   lectura, [`minmark_fs::read_dir_entries`]) y el resultado se integra en
//!   el bucle principal con `glib::idle_add_once`: la UI jamás hace E/S y el
//!   árbol crece a demanda, directorio a directorio.
//! - Activar un archivo lo abre como pestaña vía `OpenTarget` (ISSUE-021);
//!   activar una carpeta la expande o colapsa.

mod model;
mod state;

pub use state::{load as load_state, save as save_state, ExplorerState, MIN_WIDTH};

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use minmark_fs::{read_dir_entries, TreeEntry};
use model::{ExplorerFilters, Row, TreeModel};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

/// Indento por nivel de profundidad en el árbol.
const INDENT_PX: i32 = 16;

/// Notificador de cambio de un filtro por ventana (§14.4/§14.5, #70):
/// consumidor enlazado por el integrador —Quick Open y el estado de las
/// acciones `win.*` se enteran de cada cambio, también de los programáticos—;
/// sin consumidor, sin-op.
type FilterNotifier = Rc<RefCell<Option<Rc<dyn Fn(bool)>>>>;

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
    /// `Open With…` (#72, §14.8): selector de aplicaciones del sistema
    /// sobre un **archivo**; la acción queda insensible en la raíz y en las
    /// carpetas (la sensibilidad vive en [`set_context_target`]).
    OpenWith,
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
    (ExplorerAction::OpenWith, "Open With…"),
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
        ExplorerAction::OpenWith => "open-with",
    }
}

/// Panel lateral del explorador (§14.3): una lista de filas planas
/// indentadas ([`Row`]) sobre el modelo puro, sin cabecera propia — los
/// toggles §14.4/§14.5 (#70) residen en el menú View y el ancho mínimo del
/// panel queda libre para que la barra lateral se estreche.
pub struct ExplorerPane {
    widget: gtk::Box,
    model: Arc<Mutex<TreeModel>>,
    store: gtk::gio::ListStore,
    root: PathBuf,
    /// `Show All Files` §14.4 por ventana (estado espejo del filtro).
    show_all: Cell<bool>,
    /// `Show Hidden Files` §14.5 por ventana (estado espejo del filtro).
    show_hidden: Cell<bool>,
    /// Notificador del filtro `Show All Files` §14.4 (ISSUE-029: la paleta
    /// Quick Open consulta con `show_all_files`; #70: el integrador también
    /// refleja el valor en la acción `win.show-all-files`).
    on_show_all_changed: FilterNotifier,
    /// Notificador del filtro `Show Hidden Files` §14.5 (#70: el integrador
    /// refleja el valor en la acción `win.show-hidden-files`).
    on_show_hidden_changed: FilterNotifier,
    /// Solo tests (#72): destino y acción vigentes del menú contextual, los
    /// mismos que usa el click derecho.
    #[cfg(test)]
    context_target: Rc<RefCell<Option<PathBuf>>>,
    #[cfg(test)]
    open_with: gtk::gio::SimpleAction,
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
        let root = minmark_fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let model: Arc<Mutex<TreeModel>> = Arc::new(Mutex::new(TreeModel::new(root.clone())));
        let store = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("mm-explorer");

        // Sin cabecera propia (#70): los toggles §14.4/§14.5 residen en el
        // menú View y el ancho mínimo del panel no secuestra la barra.

        // Menú contextual §14.8 sobre el nodo del click (None = raíz).
        let context_target: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let (popover, open_with) = build_context_menu(&widget, &context_target, &on_action);

        // Lista de filas: un botón plano por fila, indentado por profundidad.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup({
            let model = model.clone();
            let store = store.clone();
            let on_open = Rc::clone(&on_open);
            let host = widget.clone();
            let popover = popover.clone();
            let open_with = open_with.clone();
            let context_target = Rc::clone(&context_target);
            move |_, object| {
                let list_item = object
                    .downcast_ref::<gtk::ListItem>()
                    .expect("el factory de lista recibe gtk::ListItem");
                let label = row_label();
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
                    let Some(list_item) = weak_item.upgrade() else {
                        return;
                    };
                    let Some(row) = current_row(&list_item) else {
                        return;
                    };
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
                    let open_with = open_with.clone();
                    let context_target = Rc::clone(&context_target);
                    gesture.connect_pressed(move |gesture, _, x, y| {
                        let target = weak_item
                            .upgrade()
                            .and_then(|list_item| current_row(&list_item))
                            .map(|row| row.path.clone());
                        set_context_target(&open_with, &context_target, target);
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
            let Some(row) = current_row(list_item) else {
                return;
            };
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
            let open_with = open_with.clone();
            let context_target = Rc::clone(&context_target);
            gesture.connect_pressed(move |gesture, _, x, y| {
                set_context_target(&open_with, &context_target, None);
                popup_at(&popover, &host, &gesture, x, y);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
        }
        scroll.add_controller(gesture);
        widget.append(&scroll);

        // §14.4/§14.5 (#70): los toggles viven en el menú View y entran por
        // `set_show_all`/`set_show_hidden`; aquí solo se crean los
        // notificadores (Quick Open y el estado de las acciones `win.*`).
        let on_show_all_changed: FilterNotifier = Rc::new(RefCell::new(None));
        let on_show_hidden_changed: FilterNotifier = Rc::new(RefCell::new(None));

        // Raíz del proyecto: primera lectura a demanda (§47.5).
        request_load(&model, &store, root.clone());

        Self {
            widget,
            model,
            store,
            root,
            show_all: Cell::new(false),
            show_hidden: Cell::new(false),
            on_show_all_changed,
            on_show_hidden_changed,
            #[cfg(test)]
            context_target,
            #[cfg(test)]
            open_with,
        }
    }

    /// Widget raíz del panel, para insertarlo en el `Paned` de la ventana.
    pub fn widget(&self) -> gtk::Widget {
        self.widget.clone().upcast()
    }

    /// Enlaza el filtro `Show All Files` §14.4 con sus consumidores
    /// (ISSUE-029: Quick Open consulta con `show_all_files`; #70: el estado
    /// de la acción `win.show-all-files`); se notifica cada cambio, también
    /// los programáticos.
    pub fn set_on_show_all_changed(&self, on_changed: Rc<dyn Fn(bool)>) {
        *self.on_show_all_changed.borrow_mut() = Some(on_changed);
    }

    /// Enlaza el filtro `Show Hidden Files` §14.5 con su consumidor (#70:
    /// el estado de la acción `win.show-hidden-files`); se notifica cada
    /// cambio, también los programáticos.
    pub fn set_on_show_hidden_changed(&self, on_changed: Rc<dyn Fn(bool)>) {
        *self.on_show_hidden_changed.borrow_mut() = Some(on_changed);
    }

    /// `Show All Files` §14.4 (#70): estado vigente del filtro por ventana.
    pub fn show_all(&self) -> bool {
        self.show_all.get()
    }

    /// Aplica `Show All Files` §14.4 (#70): vuelca el filtro al modelo,
    /// re-renderiza sin releer (§47.5) y notifica a sus consumidores (Quick
    /// Open ISSUE-029 y la acción `win.*`, sea la activación del menú o un
    /// set programático — los dos caminos convergen en la misma notificación).
    pub fn set_show_all(&self, show_all: bool) {
        self.show_all.set(show_all);
        self.apply_filters();
        if let Some(notify) = self.on_show_all_changed.borrow().as_ref() {
            notify(show_all);
        }
    }

    /// `Show Hidden Files` §14.5 (#70): estado vigente del filtro por ventana.
    pub fn show_hidden(&self) -> bool {
        self.show_hidden.get()
    }

    /// Aplica `Show Hidden Files` §14.5 (#70): filtro + re-render sin releer
    /// (§47.5) y notificación a sus consumidores (la acción `win.*`, tanto
    /// vía menú como programáticamente).
    pub fn set_show_hidden(&self, show_hidden: bool) {
        self.show_hidden.set(show_hidden);
        self.apply_filters();
        if let Some(notify) = self.on_show_hidden_changed.borrow().as_ref() {
            notify(show_hidden);
        }
    }

    /// Vuelca los toggles por ventana a los [`ExplorerFilters`] del modelo y
    /// re-renderiza la lista (§47.5: sin releer directorios).
    fn apply_filters(&self) {
        let mut model = lock(&self.model);
        let mut filters: ExplorerFilters = model.filters();
        filters.show_all = self.show_all.get();
        filters.show_hidden = self.show_hidden.get();
        model.set_filters(filters);
        rebuild(&self.store, &model);
    }

    /// Nombres de las filas renderizadas (solo tests #70): lo observable del
    /// filtrado §14.4/§14.5 sin alcanzar el modelo por dentro.
    #[cfg(test)]
    pub fn row_names(&self) -> Vec<String> {
        (0..self.store.n_items())
            .filter_map(|index| self.store.item(index))
            .filter_map(|item| item.downcast::<glib::BoxedAnyObject>().ok())
            .map(|item| item.borrow::<Row>().name.clone())
            .collect()
    }

    /// Recarga el árbol tras operaciones §14.8 (ISSUE-026 sustituirá esto por
    /// refresco por watcher §46). Conserva los filtros.
    pub fn refresh(&self) {
        self.model.lock().unwrap_or_else(|p| p.into_inner()).reset();
        rebuild(&self.store, &lock(&self.model));
        request_load(&self.model, &self.store, self.root.clone());
    }

    /// Solo tests (#72): fija el destino del menú contextual como haría el
    /// click derecho y devuelve si `Open With…` queda activo sobre él.
    #[cfg(test)]
    pub fn test_context_open_with(&self, target: Option<PathBuf>) -> bool {
        set_context_target(&self.open_with, &self.context_target, target);
        self.open_with.is_enabled()
    }

    /// Solo tests (#72): activa `explorer.open-with` sobre el destino
    /// vigente, el mismo camino que pulsar el ítem en el menú.
    #[cfg(test)]
    pub fn test_activate_open_with(&self) {
        self.open_with.activate(None);
    }
}

/// Construye el `PopoverMenu` §14.8 con las acciones `explorer.*` y
/// devuelve también la acción `Open With…` (#72), que el click derecho
/// habilita o deshabilita con [`set_context_target`] antes de cada apertura.
fn build_context_menu(
    host: &gtk::Box,
    target: &Rc<RefCell<Option<PathBuf>>>,
    on_action: &Rc<dyn Fn(ExplorerAction, Option<PathBuf>)>,
) -> (gtk::PopoverMenu, gtk::gio::SimpleAction) {
    let (menu, group, open_with) = context_menu_model(target, on_action);
    host.insert_action_group("explorer", Some(&group));
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(host);
    (popover, open_with)
}

/// Menú §14.8 en datos GIO puros: el modelo, el grupo `explorer.*` (una
/// acción por ítem, activada contra el nodo `target`) y la acción
/// `Open With…` (#72) en la mano para su sensibilidad por destino. GMenu y
/// GSimpleAction no necesitan display, así que este cableado se prueba en
/// modo headless.
fn context_menu_model(
    target: &Rc<RefCell<Option<PathBuf>>>,
    on_action: &Rc<dyn Fn(ExplorerAction, Option<PathBuf>)>,
) -> (
    gtk::gio::Menu,
    gtk::gio::SimpleActionGroup,
    gtk::gio::SimpleAction,
) {
    let menu = gtk::gio::Menu::new();
    let group = gtk::gio::SimpleActionGroup::new();
    // Secciones §14.8: crear/importar / gestionar / eliminar / usar el
    // archivo fuera (#72: copiar su ruta y abrirlo con otra app).
    const SECTIONS: &[&[usize]] = &[&[0, 1, 2], &[3, 4, 5], &[6], &[7, 8]];
    let mut open_with = None;
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
            if action == ExplorerAction::OpenWith {
                open_with = Some(simple.clone());
            }
            group.add_action(&simple);
        }
        menu.append_section(None::<&str>, &submenu);
    }
    // #72: hasta que el click derecho fije el destino no hay nada que abrir.
    let open_with = open_with.expect("CONTEXT_MENU_SPEC incluye Open With… (#72)");
    open_with.set_enabled(false);
    (menu, group, open_with)
}

/// Fija el destino del menú §14.8 y aplica su sensibilidad por destino
/// (#72): `Open With…` solo tiene sentido sobre un **archivo** — ni la raíz
/// del proyecto ni las carpetas ofrecen selector de aplicaciones.
fn set_context_target(
    open_with: &gtk::gio::SimpleAction,
    target: &Rc<RefCell<Option<PathBuf>>>,
    next: Option<PathBuf>,
) {
    open_with.set_enabled(next.as_deref().is_some_and(|path| !path.is_dir()));
    *target.borrow_mut() = next;
}

/// Muestra el menú en las coordenadas del gesto (relativas a `host`).
fn popup_at(
    popover: &gtk::PopoverMenu,
    host: &gtk::Box,
    gesture: &gtk::GestureClick,
    x: f64,
    y: f64,
) {
    let widget = gesture
        .widget()
        .and_then(|w| w.translate_coordinates(host, x, y));
    let (x, y) = widget.unwrap_or((x, y));
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.popup();
}

/// Fila vigente ([`Row`]) del `ListItem` reciclado.
fn current_row(list_item: &gtk::ListItem) -> Option<Row> {
    let item = list_item.item().and_downcast::<glib::BoxedAnyObject>()?;
    let row = (*item.borrow::<Row>()).clone();
    Some(row)
}

/// Etiqueta de fila (#70): nombre alineado a la izquierda con elipsis al
/// final, para que una fila larga se acomode a cualquier ancho de la barra
/// lateral en lugar de impedir estrecharla.
fn row_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
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
            let mut model = lock(&model);
            model.finish_load(&dir, entries);
            if let Some(store) = store_weak.upgrade() {
                rebuild(&store, &model);
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
    model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §14.8/§15: el menú contextual cubre todas las operaciones exigidas,
    /// `Rename` conserva el nombre de la SRS e `Import Markdown` (§15) usa su
    /// etiqueta literal; `Open With…` (#72) añade su etiqueta literal al
    /// final, junto a Copy Relative Path.
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
                "Open With…",
            ]
        );
        // Una acción registrada por ítem, sin duplicados.
        let mut nombres = std::collections::HashSet::new();
        for (action, _) in CONTEXT_MENU_SPEC {
            assert!(nombres.insert(action_name(*action)), "acción duplicada");
        }
    }

    /// #72: `Open With…` cableado de punta a punta sin display — ítem con su
    /// etiqueta literal, acción `explorer.open-with` que entrega el nodo
    /// vigente al despachante y sensibilidad por destino (solo archivos:
    /// ni la raíz del proyecto ni las carpetas ofrecen selector de apps).
    #[test]
    fn open_with_72_cableado_y_solo_sobre_archivos() {
        let raiz = std::env::temp_dir().join(format!("minmark-openwith72-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        let carpeta = raiz.join("carpeta");
        std::fs::create_dir_all(&carpeta).unwrap();
        let archivo = raiz.join("documento.md");
        std::fs::write(&archivo, "# doc\n").unwrap();

        /// Etiquetas del modelo §14.8, sección a sección (GMenu no necesita
        /// display; la introspección de `gio::MenuModel` tampoco).
        fn etiquetas(menu: &gtk::gio::Menu) -> Vec<String> {
            let mut out = Vec::new();
            for i in 0..menu.n_items() {
                let Some(section) = menu.item_link(i, "section") else {
                    continue;
                };
                for j in 0..section.n_items() {
                    let label = section.item_attribute_value(j, "label", None);
                    out.push(label.unwrap().str().unwrap_or_default().into());
                }
            }
            out
        }

        assert_eq!(action_name(ExplorerAction::OpenWith), "open-with");
        let target: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        /// Registro de activaciones entregadas al despachante.
        type Activaciones = Rc<RefCell<Vec<(ExplorerAction, Option<PathBuf>)>>>;
        let activaciones: Activaciones = Rc::new(RefCell::new(Vec::new()));
        let despachante: Rc<dyn Fn(ExplorerAction, Option<PathBuf>)> = {
            let activaciones = Rc::clone(&activaciones);
            Rc::new(move |action, nodo| activaciones.borrow_mut().push((action, nodo)))
        };
        let (menu, group, open_with) = context_menu_model(&target, &despachante);

        assert!(etiquetas(&menu).contains(&"Open With…".to_owned()));
        assert!(group.lookup_action("open-with").is_some());

        // Sin destino aún no hay nada que abrir; luego decide el nodo.
        assert!(!open_with.is_enabled(), "arranca insensible (#72)");
        set_context_target(&open_with, &target, None);
        assert!(!open_with.is_enabled(), "la raíz no ofrece Open With…");
        set_context_target(&open_with, &target, Some(carpeta));
        assert!(!open_with.is_enabled(), "una carpeta no lo ofrece");
        set_context_target(&open_with, &target, Some(archivo.clone()));
        assert!(open_with.is_enabled(), "un archivo sí");

        // Activar entrega (OpenWith, nodo) al despachante, como el menú.
        open_with.activate(None);
        assert_eq!(
            *activaciones.borrow(),
            [(ExplorerAction::OpenWith, Some(archivo))]
        );
    }

    /// #70: sin la cabecera de toggles el ancho mínimo del panel deja de
    /// secuestrar la barra lateral (regresión del hostage de 268px) y las
    /// filas elipsan su nombre para acomodarse a anchos estrechos.
    #[test]
    #[ignore = "requiere sesión gráfica GTK"]
    fn explorador_70_ancho_minimo_estrecho_y_filas_ellipsizadas() {
        gtk4::init().expect("GTK display");
        let root = std::env::temp_dir().join(format!("minmark-geo70-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("documento.md"), "# x\n").unwrap();
        let pane = ExplorerPane::new(&root, Rc::new(|_| {}), Rc::new(|_, _| {}));

        let (min, natural, _, _) = pane.widget().measure(gtk::Orientation::Horizontal, -1);
        println!("explorador #70: ancho mínimo {min}px, natural {natural}px");
        assert!(
            min <= 60,
            "ancho mínimo del explorador: {min}px (la cabecera de toggles lo fijaba en 268px, #70)"
        );

        // La fila creada por el factory elipsa al final: un nombre largo se
        // recorta en vez de ensanchar la barra.
        assert_eq!(
            row_label().ellipsize(),
            gtk::pango::EllipsizeMode::End,
            "las filas del explorador elipsan (#70)"
        );
    }
}
