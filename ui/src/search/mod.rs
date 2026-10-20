//! Panel de búsqueda en el proyecto (ISSUE-028, §40.2, §39).
//!
//! `Ctrl+Shift+F` lo abre sobre el proyecto activo (el cableado del comando y
//! su acelerador vive en el integrador, §39). Ofrece el modelo funcional de
//! VS Code de §40.2: campo de búsqueda y de reemplazo, toggles «Aa» (Match
//! Case), «palabra» (Whole Word) y «.*» (Regex), alcance `.md` por defecto
//! ampliable (extensiones o «todos»), resultados **agrupados por fichero**
//! con contexto por coincidencia, expandir/contraer, **exclusión individual**
//! (checkbox por resultado), contadores de ficheros y coincidencias, y
//! «Reemplazar» (individual), «Reemplazar en fichero», «Reemplazar todo» y
//! «Deshacer reemplazo» — todo reemplazo masivo se deshace de una sola acción
//! (§40.2, [`minmark_fs::search::MassReplace`]).
//!
//! - §47.1/§47.5: la búsqueda y los reemplazos corren en hilos aparte (nunca
//!   en el hilo de UI) y se integran en el bucle principal con un canal y un
//!   timer GLib local: resultados parciales por lotes sin bloquear, y
//!   relanzar una búsqueda **cancela** la anterior (generación + bandera
//!   cooperativa).
//! - §12.5: los archivos no UTF-8 se saltan con nota en la línea de estado.
//! - §12.7/§3.7: si un archivo cambió en disco, el reemplazo se aborta
//!   completo con error visible; nunca sobrescritura silenciosa.
//! - §40.1 criterio 4: un patrón regex inválido es un error NO bloqueante en
//!   la línea de estado que conserva la última búsqueda válida.
//!
//! La lógica de resultados vive en [`model`] (pura, sin GTK). La API pública
//! para el integrador es [`SearchPanel`]: [`SearchPanel::new`],
//! [`SearchPanel::widget`], [`SearchPanel::set_root`],
//! [`SearchPanel::focus_search`], [`SearchPanel::run_search`] y los
//! callbacks [`SearchPanel::on_open_result`] y
//! [`SearchPanel::on_replaced`].

mod model;

pub use minmark_fs::search::ReplaceSummary;
pub use model::{ReplaceScope, SearchModel};

use model::scope_from;

use gtk4::prelude::*;
use gtk4::{
    glib, Box as GtkBox, Button, CheckButton, DropDown, Entry, Image, Label, ListBox, ListBoxRow,
    Orientation, PolicyType, ScrolledWindow, ToggleButton, Widget,
};
use minmark_document::{Matcher, SearchOptions};
use minmark_fs::search::{
    apply_replace, plan_replace, search_streaming, MassReplace, SearchEvent, SearchScope,
    SearchSource, SkippedFile,
};
use minmark_fs::ProjectIndex;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

/// Periodo del timer GLib que drena el canal del worker (patrón §47.5).
const POLL_MS: u64 = 60;

/// Botones de reemplazo §40.2, en orden de aparición (la UI se construye y
/// se testea contra esta especificación).
const BUTTON_LABELS: &[&str] = &[
    "Reemplazar",
    "Reemplazar en fichero",
    "Reemplazar todo",
    "Deshacer reemplazo",
];

/// Opciones del selector de alcance §40.2 (la primera es la por defecto).
const SCOPE_OPTIONS: &[&str] = &[
    "Markdown (.md, .markdown)",
    "Todos los archivos",
    "Personalizado:",
];

/// Resultado de un worker de reemplazo/deshacer (viaja por el canal).
enum OpDone {
    /// Reemplazo aplicado (el registro queda para «Deshacer»).
    Replaced(Result<MassReplace, String>),
    /// Deshacer terminado; el registro viaja de vuelta para reintentar si
    /// falló (all-or-nothing: no cambió nada).
    Undone(Result<ReplaceSummary, String>, MassReplace),
}

/// Estado compartido del panel: widgets y modelo, en un `Rc` para que los
/// closures de señales y los timers del bucle principal lo alcancen.
struct Core {
    search_entry: Entry,
    replace_entry: Entry,
    case_button: ToggleButton,
    word_button: ToggleButton,
    regex_button: ToggleButton,
    scope_dropdown: DropDown,
    custom_entry: Entry,
    list: ListBox,
    counters: Label,
    status: Label,
    replace_current: Button,
    replace_file: Button,
    replace_all: Button,
    undo_button: Button,
    model: RefCell<SearchModel>,
    root: RefCell<Option<PathBuf>>,
    /// Historial de reemplazos masivos: «Deshacer» revierte el último de una
    /// sola acción (§40.2).
    history: RefCell<Vec<MassReplace>>,
    /// Generación de búsqueda: al relanzar, los timers viejos se retiran.
    generation: Cell<u64>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    /// Un reemplazo/deshacer en curso (los botones quedan inactivos).
    busy: Cell<bool>,
    /// Cambio programático de widgets (sin relanzar búsqueda).
    programmatic: Cell<bool>,
    skipped: RefCell<Vec<SkippedFile>>,
    on_open: RefCell<Option<Box<dyn Fn(String, usize, usize)>>>,
    on_replaced: RefCell<Option<Box<dyn Fn(ReplaceSummary)>>>,
}

/// Panel de búsqueda de proyecto (§40.2): widgets GTK sobre [`SearchModel`].
pub struct SearchPanel {
    widget: GtkBox,
    core: Rc<Core>,
}

impl SearchPanel {
    /// Construye el panel (sin raíz; [`SearchPanel::set_root`] la fija).
    pub fn new() -> Self {
        let widget = GtkBox::new(Orientation::Vertical, 4);
        widget.add_css_class("mm-project-search");
        widget.set_margin_top(4);
        widget.set_margin_bottom(4);
        widget.set_margin_start(6);
        widget.set_margin_end(6);

        let search_entry = Entry::new();
        search_entry.set_placeholder_text(Some("Buscar en el proyecto"));
        search_entry.set_hexpand(true);
        let case_button = ToggleButton::with_label("Aa");
        case_button.set_tooltip_text(Some("Distinguir mayúsculas (Match Case)"));
        let word_button = ToggleButton::with_label("palabra");
        word_button.set_tooltip_text(Some("Solo palabras enteras (Whole Word)"));
        let regex_button = ToggleButton::with_label(".*");
        regex_button.set_tooltip_text(Some("Expresión regular (Regex)"));
        let search_row = GtkBox::new(Orientation::Horizontal, 4);
        search_row.append(&search_entry);
        search_row.append(&case_button);
        search_row.append(&word_button);
        search_row.append(&regex_button);

        let replace_entry = Entry::new();
        replace_entry.set_placeholder_text(Some("Reemplazar"));
        replace_entry.set_hexpand(true);
        let replace_current = Button::with_label(BUTTON_LABELS[0]);
        let replace_file = Button::with_label(BUTTON_LABELS[1]);
        let replace_all = Button::with_label(BUTTON_LABELS[2]);
        let undo_button = Button::with_label(BUTTON_LABELS[3]);
        let replace_row = GtkBox::new(Orientation::Horizontal, 4);
        replace_row.append(&replace_entry);
        replace_row.append(&replace_current);
        replace_row.append(&replace_file);
        replace_row.append(&replace_all);
        replace_row.append(&undo_button);

        let scope_dropdown = DropDown::from_strings(SCOPE_OPTIONS);
        let custom_entry = Entry::new();
        custom_entry.set_placeholder_text(Some("extensiones (p. ej. txt, log)"));
        custom_entry.set_sensitive(false);
        let counters = Label::new(Some("0 coincidencias en 0 ficheros"));
        counters.set_xalign(1.0);
        counters.set_hexpand(true);
        let scope_row = GtkBox::new(Orientation::Horizontal, 8);
        scope_row.append(&scope_dropdown);
        scope_row.append(&custom_entry);
        scope_row.append(&counters);

        let status = Label::new(None);
        status.add_css_class("error");
        status.set_xalign(0.0);
        status.set_visible(false);

        let list = ListBox::new();
        list.add_css_class("mm-project-search-results");
        let scroller = ScrolledWindow::new();
        scroller.set_child(Some(&list));
        scroller.set_policy(PolicyType::Automatic, PolicyType::Automatic);
        scroller.set_vexpand(true);

        widget.append(&search_row);
        widget.append(&replace_row);
        widget.append(&scope_row);
        widget.append(&status);
        widget.append(&scroller);

        let core = Rc::new(Core {
            search_entry,
            replace_entry,
            case_button,
            word_button,
            regex_button,
            scope_dropdown,
            custom_entry,
            list,
            counters,
            status,
            replace_current,
            replace_file,
            replace_all,
            undo_button,
            model: RefCell::new(SearchModel::default()),
            root: RefCell::new(None),
            history: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            cancel: RefCell::new(None),
            busy: Cell::new(false),
            programmatic: Cell::new(false),
            skipped: RefCell::new(Vec::new()),
            on_open: RefCell::new(None),
            on_replaced: RefCell::new(None),
        });

        // Búsqueda: Enter o cambiar consulta/opciones/alcance relanza (y el
        // relanzar cancela la anterior, §47.5).
        {
            let handler = core.clone();
            core.search_entry
                .connect_activate(move |_| handler.launch_search());
        }
        {
            let handler = core.clone();
            core.search_entry
                .connect_changed(move |_| handler.launch_search());
        }
        for button in [&core.case_button, &core.word_button, &core.regex_button] {
            let handler = core.clone();
            button.connect_toggled(move |_| handler.launch_search());
        }
        {
            let handler = core.clone();
            core.scope_dropdown
                .connect_selected_notify(move |dropdown| {
                    let core = &handler;
                    core.custom_entry.set_sensitive(dropdown.selected() == 2);
                    core.launch_search();
                });
        }
        {
            let handler = core.clone();
            core.custom_entry
                .connect_activate(move |_| handler.launch_search());
        }

        // Reemplazo §40.2 (individual / en fichero / todo) y deshacer único.
        {
            let handler = core.clone();
            core.replace_current
                .connect_clicked(move |_| handler.launch_replace(ReplaceScope::Current));
        }
        {
            let handler = core.clone();
            core.replace_file
                .connect_clicked(move |_| handler.launch_replace(ReplaceScope::Group));
        }
        {
            let handler = core.clone();
            core.replace_all
                .connect_clicked(move |_| handler.launch_replace(ReplaceScope::All));
        }
        {
            let handler = core.clone();
            core.undo_button
                .connect_clicked(move |_| handler.launch_undo());
        }

        core.rebuild();
        Self { widget, core }
    }

    /// Widget raíz del panel para montarlo en el layout del integrador.
    pub fn widget(&self) -> &Widget {
        self.widget.upcast_ref()
    }

    /// Fija la raíz del proyecto (§14.1). Al cambiar de raíz se limpian
    /// resultados e historial (las rutas relativas son de esa raíz, §17).
    pub fn set_root(&self, root: &Path) {
        let root = minmark_fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let cambio = self.core.root.borrow().as_ref() != Some(&root);
        *self.core.root.borrow_mut() = Some(root);
        if cambio {
            self.core.model.borrow_mut().clear();
            self.core.history.borrow_mut().clear();
            self.core.set_status("");
            self.core.rebuild();
        }
    }

    /// Da el foco al campo de búsqueda (§39: al abrir con `Ctrl+Shift+F`).
    pub fn focus_search(&self) {
        self.core.search_entry.grab_focus();
    }

    /// Búsqueda programática (p. ej. al abrir el panel con una selección):
    /// fija consulta, opciones y —si `replace` es `Some`— el campo de
    /// reemplazo, y relanza.
    pub fn run_search(&self, query: &str, options: SearchOptions, replace: Option<String>) {
        {
            let core = &self.core;
            core.programmatic.set(true);
            core.search_entry.set_text(query);
            core.case_button.set_active(options.case_sensitive);
            core.word_button.set_active(options.whole_word);
            core.regex_button.set_active(options.regex);
            if let Some(template) = replace {
                core.replace_entry.set_text(&template);
            }
            core.programmatic.set(false);
        }
        self.core.launch_search();
    }

    /// Callback de apertura de resultado: `(rel_path, línea 1-based, columna
    /// 1-based en caracteres)` — `Position::new(line - 1, col - 1)` en el
    /// editor (§8.6).
    pub fn on_open_result<F: Fn(String, usize, usize) + 'static>(&self, callback: F) -> &Self {
        *self.core.on_open.borrow_mut() = Some(Box::new(callback));
        self
    }

    /// Callback tras aplicar o deshacer un reemplazo masivo (§40.2): recibe
    /// el resumen (§3.7) con los `rel_path` afectados, para sincronizar
    /// pestañas abiertas y refrescos (§12.7).
    pub fn on_replaced<F: Fn(ReplaceSummary) + 'static>(&self, callback: F) -> &Self {
        *self.core.on_replaced.borrow_mut() = Some(Box::new(callback));
        self
    }
}

impl Default for SearchPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    /// Opciones §40.2/§40.1 de los toggles.
    fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.case_button.is_active(),
            whole_word: self.word_button.is_active(),
            regex: self.regex_button.is_active(),
        }
    }

    /// Alcance §40.2 del selector (Markdown por defecto, ampliable).
    fn scope(&self) -> SearchScope {
        scope_from(self.scope_dropdown.selected(), &self.custom_entry.text())
    }

    /// Lanza la búsqueda en un hilo aparte (§47.1/§47.5) y drena sus lotes
    /// con un timer GLib local. Relanzar cancela la anterior (generación +
    /// bandera cooperativa): los resultados parciales nunca bloquean.
    fn launch_search(self: &Rc<Self>) {
        if self.programmatic.get() {
            return;
        }
        if let Some(previous) = self.cancel.borrow().as_ref() {
            previous.store(true, Ordering::Relaxed);
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);

        let query = self.search_entry.text().to_string();
        if query.is_empty() {
            self.model.borrow_mut().clear();
            self.skipped.borrow_mut().clear();
            self.set_status("");
            self.rebuild();
            return;
        }
        let options = self.options();
        // Validación en el hilo de UI (solo compilación de patrón): una regex
        // inválida es un error no bloqueante que conserva los resultados
        // anteriores (§40.1 criterio 4).
        if let Err(error) = Matcher::new(&query, options) {
            self.set_status(&error.to_string());
            return;
        }
        let Some(root) = self.root.borrow().clone() else {
            self.set_status("Sin proyecto abierto");
            return;
        };

        self.model.borrow_mut().clear();
        self.skipped.borrow_mut().clear();
        self.set_status("Buscando…");
        self.rebuild();

        let scope = self.scope();
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = Some(cancel.clone());
        let (tx, rx) = channel::<SearchEvent>();
        let use_index = scope == SearchScope::markdown();
        std::thread::spawn(move || {
            let matcher = Matcher::new(&query, options).expect("patrón ya validado en la UI");
            let mut index;
            let source = if use_index {
                // Candidatos del índice (ISSUE-025, §45): refresco
                // incremental en background, nunca en el hilo de UI.
                index = ProjectIndex::load_or_new(&root);
                let _ = index.refresh();
                SearchSource::Index(&index)
            } else {
                SearchSource::Project
            };
            let _ = search_streaming(&root, source, &matcher, &scope, &cancel, &tx);
        });
        self.install_search_poller(rx, generation);
    }

    /// Timer local que integra los lotes del worker en el bucle principal
    /// (patrón §47.5 del explorador: canal + timer GLib).
    fn install_search_poller(self: &Rc<Self>, rx: Receiver<SearchEvent>, generation: u64) {
        let core = self.clone();
        glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
            if core.generation.get() != generation {
                // Búsqueda obsoleta (se relanzó otra): el hilo ya está
                // cancelado y sus eventos se descartan.
                return glib::ControlFlow::Break;
            }
            let mut batch = Vec::new();
            let mut finished = false;
            loop {
                match rx.try_recv() {
                    Ok(SearchEvent::Batch(files)) => batch.extend(files),
                    Ok(SearchEvent::Skipped(skipped)) => core.skipped.borrow_mut().push(skipped),
                    Ok(SearchEvent::Done(_)) => {
                        finished = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
            if !batch.is_empty() {
                core.model.borrow_mut().append_batch(batch);
            }
            if finished {
                core.set_status(&summary_text(
                    core.model.borrow().included_matches(),
                    core.model.borrow().total_matches(),
                    core.model.borrow().total_files(),
                    core.skipped.borrow().len(),
                ));
            }
            core.rebuild();
            if finished {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Reemplazo masivo (§40.2) de la selección `scope`, en un hilo aparte
    /// (§47.1: la escritura nunca bloquea la UI) y all-or-nothing con undo
    /// único posterior.
    fn launch_replace(self: &Rc<Self>, scope: ReplaceScope) {
        if self.busy.get() {
            return;
        }
        let Some(root) = self.root.borrow().clone() else {
            self.set_status("Sin proyecto abierto");
            return;
        };
        let files = self.model.borrow().replace_files(scope);
        if files.is_empty() {
            self.set_status("Nada seleccionado para reemplazar");
            return;
        }
        let template = self.replace_entry.text().to_string();
        let query = self.search_entry.text().to_string();
        let options = self.options();
        if let Err(error) = Matcher::new(&query, options) {
            self.set_status(&error.to_string());
            return;
        }
        self.set_busy(true);
        self.set_status("Reemplazando…");

        let (tx, rx) = channel::<OpDone>();
        std::thread::spawn(move || {
            let resultado = (|| {
                let matcher = Matcher::new(&query, options).map_err(|error| error.to_string())?;
                let plan = plan_replace(&root, &matcher, &template, &files)
                    .map_err(|error| error.to_string())?;
                apply_replace(&plan).map_err(|error| error.to_string())
            })();
            let _ = tx.send(OpDone::Replaced(resultado));
        });
        self.install_op_poller(rx);
    }

    /// «Deshacer reemplazo» (§40.2): revierte el último reemplazo masivo —
    /// **todos** sus ficheros en una sola acción — en un hilo aparte.
    fn launch_undo(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(mass) = self.history.borrow_mut().pop() else {
            self.set_status("No hay reemplazos que deshacer");
            return;
        };
        self.set_busy(true);
        self.set_status("Deshaciendo reemplazo…");

        let (tx, rx) = channel::<OpDone>();
        std::thread::spawn(move || {
            let resultado = mass.undo().map_err(|error| error.to_string());
            let _ = tx.send(OpDone::Undone(resultado, mass));
        });
        self.install_op_poller(rx);
    }

    /// Timer local para el resultado único de un reemplazo/deshacer.
    fn install_op_poller(self: &Rc<Self>, rx: Receiver<OpDone>) {
        let core = self.clone();
        glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    core.finish_op(OpDone::Replaced(Err(
                        "el reemplazo se interrumpió antes de terminar".to_owned(),
                    )));
                    glib::ControlFlow::Break
                }
                Ok(done) => {
                    core.finish_op(done);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    /// Integra el resultado de un reemplazo/deshacer en el hilo principal:
    /// historial, callback `on_replaced` (§12.7: sincronizar pestañas) y
    /// refresco de resultados.
    fn finish_op(self: &Rc<Self>, done: OpDone) {
        self.set_busy(false);
        match done {
            OpDone::Replaced(Ok(mass)) => {
                let resumen = mass.summary().clone();
                self.history.borrow_mut().push(mass);
                self.set_status(&format!(
                    "Reemplazado: {}",
                    touched_text(resumen.matches, resumen.files.len())
                ));
                if let Some(callback) = self.on_replaced.borrow().as_ref() {
                    callback(resumen);
                }
                self.launch_search();
            }
            OpDone::Replaced(Err(error)) => self.set_status(&error),
            OpDone::Undone(Ok(resumen), _) => {
                self.set_status(&format!(
                    "Deshecho: {}",
                    touched_text(resumen.matches, resumen.files.len())
                ));
                if let Some(callback) = self.on_replaced.borrow().as_ref() {
                    callback(resumen);
                }
                self.launch_search();
            }
            // All-or-nothing: el fallo no cambió nada y el registro vuelve al
            // historial para reintentar (§3.7).
            OpDone::Undone(Err(error), mass) => {
                self.history.borrow_mut().push(mass);
                self.set_status(&error);
            }
        }
    }

    /// Emite `on_open_result` para la coincidencia activa (§8.6: línea y
    /// columna 1-based en caracteres).
    fn open_current(&self) {
        let model = self.model.borrow();
        let Some((group, hit)) = model.current_match() else {
            return;
        };
        let (rel_path, line, col) = (group.rel_path.clone(), hit.line, hit.col);
        drop(model);
        if let Some(callback) = self.on_open.borrow().as_ref() {
            callback(rel_path, line, col);
        }
    }

    /// Vuelca el modelo en la lista (§40.2: agrupado por fichero, contexto
    /// por coincidencia, checkbox de exclusión por resultado). Cada fila
    /// captura su destino y un [`std::rc::Weak`] del núcleo (sin ciclos de
    /// `Rc`): un click basta para expandir, abrir o excluir.
    fn rebuild(self: &Rc<Self>) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let model = self.model.borrow();
        for (group_index, group) in model.groups().iter().enumerate() {
            // Fila de fichero: expandir/contraer (§40.2).
            let file_row = GtkBox::new(Orientation::Horizontal, 6);
            let chevron = if group.expanded {
                Image::from_icon_name("pan-down-symbolic")
            } else {
                Image::from_icon_name("pan-end-symbolic")
            };
            file_row.append(&chevron);
            let name = Label::new(None);
            name.set_markup(&format!(
                "<b>{}</b>",
                glib::markup_escape_text(&group.rel_path)
            ));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            file_row.append(&name);
            let count = Label::new(Some(&included_count_text(group)));
            count.add_css_class("dim-label");
            file_row.append(&count);

            let file_button = Button::new();
            file_button.add_css_class("flat");
            file_button.set_child(Some(&file_row));
            file_button.set_hexpand(true);
            file_button.set_tooltip_text(Some("Expandir/contraer los resultados de este fichero"));
            {
                let weak = Rc::downgrade(self);
                file_button.connect_clicked(move |_| {
                    if let Some(core) = weak.upgrade() {
                        core.model.borrow_mut().toggle_expanded(group_index);
                        core.rebuild();
                    }
                });
            }
            let file_list_row = ListBoxRow::new();
            file_list_row.set_child(Some(&file_button));
            self.list.append(&file_list_row);

            if !group.expanded {
                continue;
            }
            for (match_index, hit) in group.matches.iter().enumerate() {
                // Fila de coincidencia: contexto + exclusión individual.
                let match_row = GtkBox::new(Orientation::Horizontal, 6);
                let checkbox = CheckButton::new();
                checkbox.set_active(!group.is_excluded(match_index));
                checkbox.set_tooltip_text(Some("Incluir esta coincidencia en el reemplazo"));
                {
                    let weak = Rc::downgrade(self);
                    checkbox.connect_toggled(move |_| {
                        if let Some(core) = weak.upgrade() {
                            core.model
                                .borrow_mut()
                                .toggle_excluded(group_index, match_index);
                            core.refresh_controls();
                        }
                    });
                }
                match_row.append(&checkbox);

                let text_button = Button::new();
                text_button.add_css_class("flat");
                text_button.set_hexpand(true);
                text_button.set_tooltip_text(Some("Abrir este resultado"));
                {
                    let weak = Rc::downgrade(self);
                    text_button.connect_clicked(move |_| {
                        if let Some(core) = weak.upgrade() {
                            if core
                                .model
                                .borrow_mut()
                                .set_current(group_index, match_index)
                            {
                                core.rebuild();
                                core.open_current();
                            }
                        }
                    });
                }
                let inner = GtkBox::new(Orientation::Horizontal, 6);
                let line = Label::new(Some(&format!("L{}", hit.line)));
                line.add_css_class("dim-label");
                inner.append(&line);
                let text = Label::new(None);
                text.set_markup(&highlight(&hit.line_text, hit.col, hit.end - hit.start));
                text.set_xalign(0.0);
                text.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                inner.append(&text);
                text_button.set_child(Some(&inner));
                match_row.append(&text_button);

                let match_list_row = ListBoxRow::new();
                match_list_row.set_child(Some(&match_row));
                self.list.append(&match_list_row);
            }
        }
        drop(model);
        self.refresh_controls();
    }

    /// Contadores (§40.2) y sensibilidad de los botones de reemplazo.
    fn refresh_controls(&self) {
        let model = self.model.borrow();
        self.counters.set_text(&counters_text(&model));
        self.replace_current
            .set_sensitive(model.current().is_some() && !self.busy.get());
        self.replace_file.set_sensitive(
            model
                .current()
                .is_some_and(|(group, _)| model.included_in(group) > 0)
                && !self.busy.get(),
        );
        self.replace_all
            .set_sensitive(model.included_matches() > 0 && !self.busy.get());
        self.undo_button
            .set_sensitive(!self.history.borrow().is_empty() && !self.busy.get());
    }

    /// Activa/desactiva los botones de reemplazo durante una operación.
    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        if busy {
            self.replace_current.set_sensitive(false);
            self.replace_file.set_sensitive(false);
            self.replace_all.set_sensitive(false);
            self.undo_button.set_sensitive(false);
        } else {
            self.refresh_controls();
        }
    }

    /// Línea de estado (errores y notas §12.5); vacía la oculta.
    fn set_status(&self, text: &str) {
        self.status.set_text(text);
        self.status.set_visible(!text.is_empty());
    }
}

/// Contadores §40.2: coincidencias y ficheros, con exclusiones si las hay.
fn counters_text(model: &SearchModel) -> String {
    let total = model.total_matches();
    let incluidas = model.included_matches();
    let mut text = format!(
        "{} en {}",
        plural(total, "coincidencia", "coincidencias"),
        plural(model.total_files(), "fichero", "ficheros"),
    );
    if incluidas != total {
        text.push_str(&format!(
            " ({})",
            plural(total - incluidas, "excluida", "excluidas")
        ));
    }
    text
}

/// Texto de cierre de búsqueda con las notas §12.5 de archivos saltados.
fn summary_text(incluidas: usize, total: usize, files: usize, skipped: usize) -> String {
    let mut text = format!(
        "{} en {}",
        plural(total, "coincidencia", "coincidencias"),
        plural(files, "fichero", "ficheros"),
    );
    if incluidas != total {
        text.push_str(&format!(
            " ({})",
            plural(total - incluidas, "excluida", "excluidas")
        ));
    }
    if skipped > 0 {
        text.push_str(&format!(
            " · {} (no UTF-8 o ilegibles, §12.5)",
            plural(skipped, "archivo omitido", "archivos omitidos")
        ));
    }
    text
}

/// Resumen de reemplazo/deshacer para la línea de estado.
fn touched_text(matches: usize, files: usize) -> String {
    format!(
        "{} en {}",
        plural(matches, "coincidencia", "coincidencias"),
        plural(files, "fichero", "ficheros"),
    )
}

/// Contador de un grupo: «3» o «2 de 5» si hay excluidas.
fn included_count_text(group: &model::ResultGroup) -> String {
    let incluidas = group.included_count();
    if incluidas == group.matches.len() {
        format!("{incluidas}")
    } else {
        format!("{incluidas} de {}", group.matches.len())
    }
}

/// `n` con su sustantivo singular/plural.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Resalta la coincidencia dentro del contexto de línea (§40.2) en markup
/// Pango: `col` es 1-based en caracteres y `len` el tamaño en bytes del
/// tramo resaltado. Todo el texto se escapa.
fn highlight(line_text: &str, col: usize, len: usize) -> String {
    let at = line_text
        .char_indices()
        .nth(col.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(line_text.len());
    let to = (at + len).min(line_text.len());
    if to <= at {
        return glib::markup_escape_text(line_text).to_string();
    }
    let (before, rest) = line_text.split_at(at);
    let (hit, after) = rest.split_at(to - at);
    format!(
        "{}<b>{}</b>{}",
        glib::markup_escape_text(before),
        glib::markup_escape_text(hit),
        glib::markup_escape_text(after)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_fs::search::{FileMatches, LineMatch};

    /// §40.2: los botones exigidos por la issue están todos, con sus
    /// etiquetas, y el alcance por defecto del selector es Markdown.
    #[test]
    fn especificacion_de_botones_y_alcance_40_2() {
        assert_eq!(
            BUTTON_LABELS,
            [
                "Reemplazar",
                "Reemplazar en fichero",
                "Reemplazar todo",
                "Deshacer reemplazo"
            ]
        );
        assert_eq!(
            SCOPE_OPTIONS[0], "Markdown (.md, .markdown)",
            "el alcance por defecto §40.2 es .md"
        );
        assert_eq!(SCOPE_OPTIONS.len(), 3);
    }

    /// §40.2 contexto: el resaltado escapa el markup y respeta el tramo de la
    /// coincidencia (col 1-based en caracteres, longitud en bytes).
    #[test]
    fn resaltado_escapa_markup_y_marca_la_coincidencia() {
        assert_eq!(
            highlight("hola <objetivo> mundo", 7, 8),
            "hola &lt;<b>objetivo</b>&gt; mundo"
        );
        // Col fuera de rango o tramo que se sale de la línea: sin panics.
        assert_eq!(highlight("abc", 99, 5), "abc");
        assert_eq!(highlight("abc", 1, 99), "<b>abc</b>");
        assert_eq!(highlight("áé", 2, 2), "á<b>é</b>");
    }

    /// §40.2 contadores: incluidas vs. excluidas, con singular/plural.
    #[test]
    fn contadores_reflejan_exclusiones() {
        let mut model = SearchModel::default();
        model.set_results(vec![FileMatches {
            rel_path: "a.md".to_owned(),
            matches: vec![
                LineMatch {
                    line: 1,
                    col: 1,
                    line_text: "x".to_owned(),
                    start: 0,
                    end: 1,
                },
                LineMatch {
                    line: 2,
                    col: 1,
                    line_text: "x".to_owned(),
                    start: 3,
                    end: 4,
                },
            ],
        }]);
        assert_eq!(counters_text(&model), "2 coincidencias en 1 fichero");
        model.toggle_excluded(0, 1);
        assert_eq!(
            counters_text(&model),
            "2 coincidencias en 1 fichero (1 excluida)"
        );
        assert_eq!(
            summary_text(1, 2, 1, 1),
            "2 coincidencias en 1 fichero (1 excluida) · 1 archivo omitido (no UTF-8 o ilegibles, §12.5)"
        );
        assert_eq!(touched_text(1, 1), "1 coincidencia en 1 fichero");
    }
}
