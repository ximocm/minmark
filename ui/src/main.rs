//! minmark — aplicación de escritorio GTK4 + WebKitGTK 6.0 (ISSUE-020, ADR-001).
//!
//! Integra el core (`core/`) con la UI decidida en ADR-001:
//! - menús §36 (File, Edit, View, Project, Help; Insert/Format llegan con
//!   sus fases);
//! - atajos §39 (Ctrl+N/O/S, Ctrl+Shift+S, Ctrl+W, Ctrl+Alt+M);
//! - composiciones (§20, ISSUE-022/023): Preview combinado y PDF único desde
//!   el menú Project/File, con la cadena de ajustes §58 y la bibliografía
//!   unida de los archivos incluidos (ISSUE-024);
//! - editor Markdown Mode (ISSUE-015) y modo Visual §8.2 (ISSUE-030:
//!   representación semántica editable sobre el mismo buffer);
//! - Preview WebView continuo (ISSUE-016) con temas (ISSUE-017);
//! - pestañas §37 con indicador ● de modificado;
//! - ventanas múltiples (ISSUE-021): standalone tipo Notepad++ (archivos
//!   sueltos, nuevos o existentes) y ventanas de proyecto por carpeta (§48);
//! - explorador de archivos con árbol perezoso (ISSUE-022, §14.3–14.7) en las
//!   ventanas de proyecto;
//! - barra lateral estilo VS Code (§40.2): búsqueda en el proyecto con
//!   `Ctrl+Shift+F` (ISSUE-028) junto al explorador, una vista visible cada
//!   vez;
//! - Quick Open `Ctrl+P` (ISSUE-029, §41): paleta fuzzy sobre el índice §45
//!   con `:N`, `@heading`, `#heading` y MRU de abiertos;
//! - diálogos GTK4 y confirmaciones explícitas (§3.7, §12.7).
//!
//! Buffer único §6.1: el modelo vive en `minmark_document::Editor` dentro de
//! `TabManager`; el `TextBuffer` del TextView es el espejo sincronizado del
//! mismo documento en los dos modos §8 (Markdown y Visual, ISSUE-030).

mod academic;
mod composition;
mod editor;
mod explorer;
mod export;
mod find;
mod intelligence;
mod list_scroll;
mod menus;
mod preview;
mod quickopen;
mod search;
mod settings;
mod tabs;

use editor::{EditSink, EditorPane};
use explorer::{load_state, save_state, ExplorerAction, ExplorerPane, ExplorerState};
use find::{FindAction, FindBar};
use menus::{actions, dialogs, install_actions, Commands};
use preview::{PreviewPane, ViewMode};
use quickopen::QuickOpen;
use search::{ReplaceSummary, SearchPanel};
use tabs::{OpenTarget, TabBar, TabManager, WindowScope};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation, Paned, PopoverMenuBar,
    Widget,
};
use minmark_config::{
    detect_system_theme, resolve_theme, Composition, ResolvedTheme, Settings, VisualSyntax,
};
use minmark_document::{Position, Range, SearchOptions, Selection};
use minmark_fs::Watcher as _;
use minmark_renderer::Palette;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// Retardo de refresco de Preview/highlight tras teclear (§47.1: las tareas
/// secundarias nunca bloquean una pulsación de teclado).
const REFRESH_DEBOUNCE_MS: u64 = 120;

/// Periodo del bucle de eventos del watcher de proyecto (ISSUE-026, §46):
/// son eventos de inotify coalescidos, nunca un rescan del filesystem.
const WATCHER_POLL_MS: u64 = 250;

/// Periodo del timer local que integra los resultados de los hilos worker en
/// el bucle principal (patrón §47.5 de la búsqueda §40.2: canal `mpsc` +
/// timer GLib); solo la entrega a Quick Open, nunca la E/S.
const DELIVERY_POLL_MS: u64 = 60;

/// Tamaño del MRU de Quick Open (§41): el ranking solo bonifica los 8 últimos
/// abiertos (misma cota que `minmark_fs::fuzzy`).
const MRU_MAX: usize = 8;

/// Cambios de disco generados por nuestras propias operaciones §14.8
/// (ISSUE-026 criterio 3): se registran antes de escribir y los eventos del
/// watcher que coinciden no se marcan como cambio externo §12.7.
#[derive(Default)]
struct SelfOps {
    paths: RefCell<Vec<PathBuf>>,
}

impl SelfOps {
    fn register(&self, path: &Path) {
        self.paths.borrow_mut().push(path.to_path_buf());
    }

    /// `true` si `path` corresponde a una operación propia (una entrada por
    /// evento: un watcher duplicado no resucita el aviso).
    fn consume(&self, path: &Path) -> bool {
        let mut paths = self.paths.borrow_mut();
        let hit = paths
            .iter()
            .position(|own| own == path || path.starts_with(own) || own.starts_with(path));
        match hit {
            Some(index) => {
                paths.remove(index);
                true
            }
            None => false,
        }
    }
}

/// Refresco del índice §45 en una tarea aparte (§47.1/§47.5) con entrega al
/// hilo de UI al terminar. El refresco es incremental e idempotente
/// (ISSUE-025) y como mucho se pide una vez por tick del watcher; el índice
/// ya refrescado se reenvía a Quick Open (ISSUE-029, §41) para consultas
/// frescas.
fn spawn_index_refresh(root: PathBuf, slot: Rc<CmdSlot>) {
    let generation = slot
        .borrow()
        .as_ref()
        .and_then(Weak::upgrade)
        .map(|me| {
            let generation = me.index_generation.get().wrapping_add(1);
            me.index_generation.set(generation);
            generation
        })
        .unwrap_or(0);
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut index = minmark_fs::ProjectIndex::load_or_new(&root);
        if let Err(err) = index.refresh() {
            eprintln!("minmark: índice de proyecto no refrescado: {err}");
            return;
        }
        if let Err(err) = index.save() {
            eprintln!("minmark: no se pudo guardar la caché del índice: {err}");
        }
        let _ = tx.send((generation, index));
    });
    deliver_to_quickopen(rx, slot, |me, (generation, index)| {
        if generation != me.index_generation.get()
            || me.quickopen_base().as_deref() != Some(index.root())
        {
            return;
        }
        if me.scope.project_root().is_some() {
            me.quickopen.set_index(&index);
        }
        *me.index.borrow_mut() = Some(index);
        me.refresh_intelligence(false);
    });
}

/// Integra el resultado de un hilo worker en el bucle principal y lo entrega
/// a la paleta Quick Open (patrón §47.5 de la búsqueda §40.2: canal `mpsc` +
/// timer GLib). El timer se detiene al recibir o al cerrarse el canal.
fn deliver_to_quickopen<T: 'static>(
    rx: Receiver<T>,
    slot: Rc<CmdSlot>,
    apply: impl Fn(&AppCmds, T) + 'static,
) {
    glib::timeout_add_local(Duration::from_millis(DELIVERY_POLL_MS), move || {
        match rx.try_recv() {
            Ok(value) => {
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    apply(&me, value);
                }
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Lista de archivos de Quick Open en ventanas standalone (ISSUE-029, §41):
/// el `walk_tree` + `IgnorePatterns` del explorador, con la visibilidad por
/// defecto §14.4/§14.5 (tipos documentales, sin ocultos). Se construye en un
/// hilo aparte (§47.1/§47.5) y se entrega como fuente fija de la paleta
/// (el índice §45 solo existe en ventanas de proyecto).
fn spawn_quickopen_entries(base: PathBuf, slot: Rc<CmdSlot>) {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(quickopen_entries(&base));
    });
    deliver_to_quickopen(rx, slot, |me, entries| me.quickopen.set_entries(entries));
}

/// Contenido de [`spawn_quickopen_entries`]: los archivos del subárbol
/// `base` visibles por defecto en el explorador — tipos documentales §14.4,
/// sin segmentos ocultos §14.5 y sin los ignorados §14.6 (`.gitignore` de
/// `base`) — en orden estable por `rel_path` §17 (empate del ranking §41).
fn quickopen_entries(base: &Path) -> Vec<minmark_fs::fuzzy::Entry> {
    use minmark_fs::fuzzy::Entry;
    // Misma raíz canónica que `walk_tree` (§12.7: dos grafías, un fichero).
    let base = ruta_comparable(base);
    let ignore = minmark_fs::IgnorePatterns::from_root(&base);
    let mut entries: Vec<Entry> = Vec::new();
    let Ok(tree) = minmark_fs::walk_tree(&base) else {
        return entries;
    };
    for node in tree {
        if node.is_dir {
            continue;
        }
        let rel_path = minmark_fs::relative_path(&base, &node.path);
        if rel_path.split('/').any(|segment| segment.starts_with('.')) {
            continue; // §14.5: los ocultos no se listan
        }
        if ignore.is_ignored(Path::new(&rel_path), false) {
            continue; // §14.6
        }
        if !minmark_fs::fuzzy::is_document_type(&rel_path) {
            continue; // §14.4
        }
        entries.push(Entry {
            label: rel_path.clone(),
            rel_path,
            line: None,
            score: 0,
            heading: None,
        });
    }
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    entries
}

/// MRU de Quick Open (§41): `rel_path` al frente (lo más reciente primero),
/// sin duplicados y acotado a [`MRU_MAX`] (el ranking no bonifica más atrás).
fn mru_touch(recent: &mut Vec<String>, rel_path: &str) {
    recent.retain(|opened| opened != rel_path);
    recent.insert(0, rel_path.to_owned());
    recent.truncate(MRU_MAX);
}

/// Enlace tardío «widget → [`AppCmds`]»: los widgets se construyen antes que
/// el estado de la ventana, así que sus callbacks capturan esta celda.
///
/// Guarda un [`Weak`]`<AppCmds>` (no un `Rc`): los callbacks poseen la celda y
/// el `AppCmds` la referencia fuerte, sin ciclos de `Rc`.
type CmdSlot = RefCell<Option<Weak<AppCmds>>>;

/// Puente `EditSink` (TextView) → modelo (`TabManager` → core `Editor`).
struct Sink {
    tabs: Rc<RefCell<TabManager>>,
}

impl EditSink for Sink {
    fn insert_at(&self, at: Position, text: &str) {
        self.tabs.borrow_mut().insert_active(at, text);
    }

    fn delete_range(&self, from: Position, to: Position) {
        self.tabs.borrow_mut().delete_active(from, to);
    }

    fn set_selection(&self, selection: Selection) {
        self.tabs.borrow_mut().set_selection_active(selection);
    }
}

/// Estado de la ventana: widgets + modelo compartido.
struct AppCmds {
    app: Application,
    window: ApplicationWindow,
    /// Ámbito de esta ventana (ISSUE-021): standalone o de proyecto.
    scope: WindowScope,
    tabs: Rc<RefCell<TabManager>>,
    editor_pane: Rc<EditorPane>,
    preview: Rc<PreviewPane>,
    tab_bar: Rc<TabBar>,
    /// Barra de búsqueda/reemplazo del archivo actual (§40.1, ISSUE-027).
    find_bar: Rc<FindBar>,
    /// Paleta Quick Open (§41, ISSUE-029, §39: `Ctrl+P`).
    quickopen: Rc<QuickOpen>,
    index: RefCell<Option<minmark_fs::ProjectIndex>>,
    index_generation: Cell<u64>,
    intelligence_generation: Cell<u64>,
    intelligence_busy: Cell<bool>,
    intelligence_pending: Cell<bool>,
    intelligence_suggest: Cell<bool>,
    completion_source: RefCell<Option<(Option<PathBuf>, String, Position)>>,
    navigation_list: Rc<QuickOpen>,
    navigation_kind: Cell<bool>,
    navigation_positions: RefCell<std::collections::BTreeMap<String, (String, Position)>>,
    /// MRU de `rel_path` §17 abiertos (§41): el más reciente primero.
    recent: RefCell<Vec<String>>,
    /// Directorio base de la lista standalone de Quick Open (ISSUE-029) y de
    /// sus `rel_path`; `None` hasta abrir el primer fichero con ruta.
    quickopen_dir: RefCell<Option<PathBuf>>,
    /// Barra lateral §14.3/§40.2 (ISSUE-022/028): explorador + búsqueda de
    /// proyecto; solo en ventanas de proyecto.
    sidebar: Option<Rc<Sidebar>>,
    /// Watcher del subárbol del proyecto (ISSUE-026, §46); `None` en
    /// standalone.
    watcher: Option<Rc<RefCell<minmark_fs::InotifyWatcher>>>,
    /// Composiciones del `.mdproject.toml` de la raíz (§20, ISSUE-022);
    /// vacías en ventanas standalone, donde las acciones de composición
    /// quedan deshabilitadas. Se redescubren al cambiar `.mdproject.toml`
    /// (B1, revisión PR #44) vía [`AppCmds::refresh_compositions`].
    compositions: RefCell<Vec<Composition>>,
    /// Grupo «win» insertado por `install_actions` (B1), en referencia
    /// DÉBIL: sus acciones capturan `Rc<dyn Commands>` de esta ventana, así
    /// que una referencia fuerte cerraría el ciclo `AppCmds → grupo →
    /// acciones → AppCmds` y la ventana no se liberaría al cerrarse. La
    /// referencia fuerte la retiene la propia ventana (`insert_action_group`);
    /// `None` hasta `install_actions`.
    action_group: RefCell<Option<gtk4::glib::WeakRef<gtk4::gio::SimpleActionGroup>>>,
    /// Ventana temporal del Preview combinado (§20.4, ISSUE-023),
    /// reutilizable con el patrón Outline/Backlinks.
    composition_preview: RefCell<Option<Rc<CompositionPreview>>>,
    /// Operaciones propias §14.8 (ISSUE-026 criterio 3).
    self_ops: Rc<SelfOps>,
    settings: RefCell<Settings>,
    academic_monitors: RefCell<Vec<gtk4::gio::FileMonitor>>,
    academic_paths: RefCell<Vec<String>>,
    view: Rc<Cell<ViewMode>>,
    refresh_scheduled: Rc<Cell<bool>>,
    slot: Rc<CmdSlot>,
}

/// Vista activa de la barra lateral (§40.2, estilo VS Code: el explorador y
/// la búsqueda de proyecto comparten sidebar, con una sola vista visible).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    /// Explorador de archivos §14.3 (ISSUE-022).
    Explorer,
    /// Búsqueda en el proyecto §40.2 (ISSUE-028).
    Search,
}

/// Barra lateral de una ventana de proyecto y su estado (ISSUE-022 §14.3,
/// ISSUE-028 §40.2): contenedor vertical `explorador | búsqueda` como
/// start-child del `Paned` exterior, con una sola vista visible cada vez y
/// ancho compartido.
struct Sidebar {
    /// Contenedor vertical `explorador | búsqueda` (§40.2).
    widget: GtkBox,
    /// Explorador §14.3 (ISSUE-022).
    explorer: Rc<ExplorerPane>,
    /// Búsqueda en el proyecto §40.2 (ISSUE-028).
    search: Rc<SearchPanel>,
    /// Vista visible (§40.2: una cada vez).
    view: Cell<SidebarView>,
    /// `Paned` exterior (barra lateral | editor+preview); su posición es el
    /// ancho de la barra lateral.
    paned: Paned,
    visible: Cell<bool>,
    state: RefCell<ExplorerState>,
}

/// Ventana temporal del Preview de una composición (§20.4, ISSUE-023), con
/// el patrón Outline/Backlinks: instancia reutilizable que Escape (o la ✕)
/// oculta sin destruir, y cuya WebView conserva la página entre aperturas.
struct CompositionPreview {
    window: gtk4::Window,
    pane: Rc<PreviewPane>,
    /// Generación del último render pedido: descarta los entregados tarde
    /// cuando se abre otra composición mientras este render sigue en vuelo.
    generation: Cell<u64>,
}

impl AppCmds {
    fn refresh_intelligence(&self, suggest: bool) {
        self.intelligence_suggest
            .set(self.intelligence_suggest.get() || suggest);
        if self.intelligence_busy.get() {
            self.intelligence_pending.set(true);
            return;
        }
        let Some(snapshot) = self.intelligence_snapshot() else {
            return;
        };
        self.intelligence_busy.set(true);
        self.intelligence_pending.set(false);
        let suggest = self.intelligence_suggest.replace(false)
            && self.tabs.borrow().active().editor.selection().is_empty();
        let generation = self.intelligence_generation.get().wrapping_add(1);
        self.intelligence_generation.set(generation);
        let cursor = self.tabs.borrow().active().editor.selection().head;
        let offset = self.tabs.borrow().active().editor.offset_of(cursor).ok();
        let path = self.tabs.borrow().active().path.clone();
        let source = snapshot.source.clone();
        let backlinks = self.navigation_list.is_visible() && self.navigation_kind.get();
        let settings = self.settings.borrow().clone();
        let root = self.scope.project_root();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let html =
                preview::render_document_at(&source, &settings, path.as_deref(), root.as_deref());
            let links = snapshot.links();
            let completions = if suggest {
                offset
                    .map(|o| {
                        let citations = intelligence::citation_completions(
                            &source,
                            o,
                            &academic::load(&settings),
                        );
                        if citations.is_empty() {
                            snapshot.completions(o)
                        } else {
                            citations
                        }
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let back = backlinks.then(|| snapshot.backlinks());
            let _ = tx.send((
                generation,
                path,
                source,
                cursor,
                html,
                links,
                completions,
                back,
            ));
        });
        deliver_to_quickopen(
            rx,
            self.slot.clone(),
            |me, (generation, path, source, cursor, html, links, completions, back)| {
                me.intelligence_busy.set(false);
                if me.intelligence_pending.replace(false) {
                    me.refresh_intelligence(false);
                    return;
                }
                if generation != me.intelligence_generation.get() {
                    return;
                }
                let tabs = me.tabs.borrow();
                if tabs.active().path != path || tabs.active().editor.document().content() != source
                {
                    return;
                }
                let same_cursor = tabs.active().editor.selection() == Selection::collapsed(cursor);
                let document_key = (tabs.active().path.clone(), tabs.active().untitled_number);
                drop(tabs);
                let base_uri = path.as_deref().and_then(Path::parent).map(|directory| {
                    format!(
                        "{}/",
                        gtk4::gio::File::for_path(ruta_comparable(directory))
                            .uri()
                            .trim_end_matches('/')
                    )
                });
                me.preview
                    .update_html(&html, base_uri.as_deref(), document_key);
                me.editor_pane.set_link_diagnostics(&source, &links);
                me.preview.set_link_diagnostics(&links);
                if same_cursor && !completions.is_empty() {
                    *me.completion_source.borrow_mut() = Some((path, source.clone(), cursor));
                    me.editor_pane.show_completions(completions);
                }
                if let Some(entries) = back {
                    let mut positions = me.navigation_positions.borrow_mut();
                    positions.clear();
                    let entries = entries
                        .into_iter()
                        .enumerate()
                        .map(|(i, mut item)| {
                            let key = format!("backlink-{i}");
                            positions.insert(key.clone(), (item.entry.rel_path, item.position));
                            item.entry.rel_path = key;
                            item.entry
                        })
                        .collect();
                    me.navigation_list.set_entries(entries);
                }
            },
        );
    }

    fn accept_completion(&self, completion: intelligence::Completion) {
        let Some((path, source, cursor)) = self.completion_source.borrow_mut().take() else {
            return;
        };
        let mut tabs = self.tabs.borrow_mut();
        let active = tabs.active();
        if active.path != path
            || active.editor.document().content() != source
            || active.editor.selection() != Selection::collapsed(cursor)
        {
            return;
        }
        let range = Range::new(
            active.editor.position_of(completion.span.start as usize),
            active.editor.position_of(completion.span.end as usize),
        );
        tabs.replace_active(range, &completion.text);
        drop(tabs);
        self.reload_active();
    }

    fn show_navigation(&self, backlinks: bool) {
        self.navigation_kind.set(backlinks);
        self.navigation_positions.borrow_mut().clear();
        self.navigation_list
            .set_title(if backlinks { "Backlinks" } else { "Outline" });
        self.navigation_list.set_entries(Vec::new());
        self.navigation_list.present();
        self.refresh_navigation();
    }

    fn refresh_navigation(&self) {
        if !self.navigation_list.is_visible() {
            return;
        }
        if self.navigation_kind.get() {
            if self.tabs.borrow().active().path.is_none() {
                self.navigation_list.set_entries(Vec::new());
            } else {
                self.refresh_intelligence(false);
            }
            return;
        }
        let tabs = self.tabs.borrow();
        let headings = minmark_document::document_anchors(&minmark_document::find_heading_nodes(
            tabs.active().editor.document().content(),
        ));
        self.navigation_list.set_entries(
            headings
                .into_iter()
                .map(|h| minmark_fs::fuzzy::Entry {
                    label: format!(
                        "{}H{} {}",
                        "  ".repeat(h.level.saturating_sub(1) as usize),
                        h.level,
                        h.text
                    ),
                    rel_path: String::new(),
                    line: Some(h.line),
                    heading: None,
                    score: 0,
                })
                .collect(),
        );
    }
    fn intelligence_snapshot(&self) -> Option<intelligence::Snapshot> {
        let root = self
            .quickopen_base()
            .or_else(|| std::env::current_dir().ok())?;
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        let from = tab
            .path
            .as_ref()
            .map(|p| minmark_fs::relative_path(&root, &ruta_comparable(p)))
            .unwrap_or_else(|| "__untitled__.md".into());
        let buffers = tabs
            .tabs()
            .iter()
            .filter_map(|tab| {
                tab.path.as_ref().map(|p| {
                    (
                        minmark_fs::relative_path(&root, &ruta_comparable(p)),
                        tab.editor.document().content().to_owned(),
                    )
                })
            })
            .collect();
        Some(intelligence::Snapshot {
            index: self
                .index
                .borrow()
                .as_ref()
                .filter(|i| i.root() == root)
                .cloned()
                .unwrap_or_else(|| minmark_fs::ProjectIndex::new(root)),
            buffers,
            from,
            source: tab.editor.document().content().to_owned(),
            wiki: self.settings.borrow().wiki_links,
        })
    }

    fn activate_link(&self, position: Option<Position>, destination: Option<String>) {
        let Some(snapshot) = self.intelligence_snapshot() else {
            return;
        };
        let offset = position.and_then(|p| self.tabs.borrow().active().editor.offset_of(p).ok());
        let path = self.tabs.borrow().active().path.clone();
        let source = snapshot.source.clone();
        let root = snapshot.index.root().to_path_buf();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let target = if let Some(dest) = destination {
                snapshot.query(|r| {
                    if let Some(wiki) = dest.strip_prefix("minmark-wiki:") {
                        snapshot.wiki.then(|| r.resolve_wiki(wiki))
                    } else {
                        Some(r.resolve(&dest, &snapshot.from))
                    }
                })
            } else {
                snapshot
                    .links()
                    .into_iter()
                    .find(|l| {
                        offset
                            .is_some_and(|o| l.span.start as usize <= o && o < l.span.end as usize)
                    })
                    .map(|l| l.target)
            };
            let _ = tx.send((path, source, root, target));
        });
        deliver_to_quickopen(rx, self.slot.clone(), |me, (path, source, root, target)| {
            let current = me.tabs.borrow();
            if current.active().path != path
                || current.active().editor.document().content() != source
            {
                return;
            }
            drop(current);
            let Some(target) = target else { return };
            if let Some(message) = intelligence::problem(&target) {
                show_error(&me.window, &message);
                return;
            }
            if target.kind != minmark_fs::TargetKind::Local {
                return;
            }
            let Some(rel) = target.rel_path else { return };
            let target_path = root.join(&rel);
            if path.is_none() && rel == "__untitled__.md" {
                if let Some(anchor) = target.anchor {
                    me.focus_line(anchor.line);
                }
                return;
            }
            me.activate_or_open(&target_path);
            if me
                .tabs
                .borrow()
                .active()
                .path
                .as_ref()
                .is_some_and(|p| ruta_comparable(p) == ruta_comparable(&target_path))
            {
                if let Some(anchor) = target.anchor {
                    me.focus_line(anchor.line);
                }
            }
        });
    }

    fn focus_line(&self, line: u32) {
        let at = Position::new(line.saturating_sub(1), 0);
        self.editor_pane.focus_find_match(Range::new(at, at));
        if let Ok(byte) = self.tabs.borrow().active().editor.offset_of(at) {
            self.preview.scroll_to_byte(byte as u32);
        }
        let pane = self.editor_pane.clone();
        glib::idle_add_local_once(move || pane.scroll_to_cursor());
    }

    fn slot(&self) -> Option<Rc<AppCmds>> {
        self.slot.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Sincroniza vistas y título con el tab activo (cambio de tab, open,
    /// undo/redo, save) y refresca Preview + highlighting.
    fn reload_active(&self) {
        {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            self.editor_pane.load(&tab.editor, tab.mode);
        }
        self.refresh_now();
        self.sync_quickopen_context();
        self.refresh_navigation();
    }

    /// Contexto de Quick Open (§41, ISSUE-029) con el tab activo: `rel_path`
    /// §17 (`None` en tabs nuevos sin guardar; alimenta `:N` y `@heading`),
    /// MRU de abiertos (el activo el primero) y, en standalone, la lista de
    /// archivos del directorio activo (fuente fija, [`quickopen_entries`]).
    fn sync_quickopen_context(&self) {
        let path = self.tabs.borrow().active().path.clone();
        // Base de los `rel_path` (§17): la raíz del proyecto o, en
        // standalone, el directorio del primer fichero activo — estable al
        // navegar dentro de su subárbol (la lista no se estrecha) y con un
        // tab sin ruta (la lista cargada sigue valiendo).
        let mut recargar = false;
        if self.scope.project_root().is_none() {
            let activo = path.as_deref().and_then(Path::parent).map(ruta_comparable);
            let mut loaded = self.quickopen_dir.borrow_mut();
            let cambiar = match (&*loaded, &activo) {
                (_, None) => false,
                (None, Some(_)) => true,
                (Some(loaded), Some(activo)) => !activo.starts_with(loaded),
            };
            if cambiar {
                recargar = true;
                *loaded = activo;
            }
        }
        let base = self.quickopen_base();
        let rel_path = match (&base, &path) {
            (Some(base), Some(path)) => {
                Some(minmark_fs::relative_path(base, &ruta_comparable(path)))
            }
            _ => None,
        };
        self.quickopen.set_current_path(rel_path.clone());
        if let Some(rel_path) = &rel_path {
            let mut recent = self.recent.borrow_mut();
            mru_touch(&mut recent, rel_path);
            self.quickopen.set_recent(recent.clone());
        }
        // §45: en ventanas de proyecto la fuente es el índice y llega con
        // cada refresco; en standalone se reconstruye al cambiar de directorio.
        if recargar {
            match base {
                Some(base) => {
                    spawn_quickopen_entries(base.clone(), self.slot.clone());
                    spawn_index_refresh(base, self.slot.clone());
                }
                None => self.quickopen.set_entries(Vec::new()),
            }
        }
    }

    /// Base de los `rel_path` de Quick Open (§17, forma canónica como la del
    /// índice §45): la raíz del proyecto o, en standalone, el directorio
    /// cuya lista está cargada (ISSUE-029).
    fn quickopen_base(&self) -> Option<PathBuf> {
        match &self.scope {
            WindowScope::Project(root) => Some(ruta_comparable(root)),
            WindowScope::Standalone => self.quickopen_dir.borrow().clone(),
        }
    }

    /// Apertura desde Quick Open (§41): `rel_path` va a `OpenTarget`
    /// (ISSUE-021) y, si la entrada trae línea (`:N` o heading ya resuelto
    /// por la paleta), coloca cursor + scroll §8.6 (1-based → 0-based).
    fn open_quickopen_entry(&self, rel_path: &str, line: Option<u32>) {
        let Some(base) = self.quickopen_base() else {
            return;
        };
        let path = base.join(rel_path);
        // §41: `:N`/`@heading` saltan en el documento actual; se activa la
        // pestaña ya abierta en vez de duplicarla (mismo criterio que un
        // resultado §40.2: «abrir (o activar) la pestaña del fichero»).
        self.activate_or_open(&path);
        let Some(line) = line else { return };
        // El cursor solo se coloca si la pestaña activa es ese fichero
        // (`activate_or_open` pudo abrirlo en una ventana nueva o fallar).
        let activo = self
            .tabs
            .borrow()
            .active()
            .path
            .as_deref()
            .is_some_and(|opened| ruta_comparable(opened) == ruta_comparable(&path));
        if !activo {
            return;
        }
        self.focus_line(line);
    }

    /// Refresco completo del tab activo: highlighting, Preview y vista
    /// Visual comparten el mismo render del buffer (§6.1).
    fn refresh_now(&self) {
        let text = self
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        let root = self.scope.project_root();
        let document = self.tabs.borrow().active().path.clone();
        let effective = settings::load_at(root.as_deref(), &text, document.as_deref());
        self.watch_academic_resources(&effective);
        self.editor_pane.set_wiki_links(effective.wiki_links);
        {
            let mut settings = self.settings.borrow_mut();
            settings.wiki_links = effective.wiki_links;
            settings.language = effective.language;
            settings.heading_numbering = effective.heading_numbering;
            settings.counter_numbering = effective.counter_numbering;
            settings.toc_max_level = effective.toc_max_level;
            settings.bibliography = effective.bibliography;
            settings.bibliography_style = effective.bibliography_style;
            settings.pagination_enabled = effective.pagination_enabled;
        }
        self.editor_pane.refresh(&text);
        {
            let tabs = self.tabs.borrow();
            self.tab_bar.sync(&tabs);
        }
        self.update_title();
        // §40.1: con la barra abierta, el marcado sigue al contenido; el
        // ancla pasa a ser el cursor (edición, undo/redo o cambio de tab).
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.find_refresh(Some(cursor), false);
        self.refresh_intelligence(false);
    }

    fn watch_academic_resources(&self, settings: &Settings) {
        let mut paths = settings.bibliography.clone();
        if !settings.bibliography_style.eq_ignore_ascii_case("IEEE") {
            paths.push(settings.bibliography_style.clone());
        }
        if *self.academic_paths.borrow() == paths {
            return;
        }
        for monitor in self.academic_monitors.borrow_mut().drain(..) {
            monitor.cancel();
        }
        let mut monitors = self.academic_monitors.borrow_mut();
        for path in &paths {
            if let Ok(monitor) = gtk4::gio::File::for_path(path).monitor_file(
                gtk4::gio::FileMonitorFlags::NONE,
                None::<&gtk4::gio::Cancellable>,
            ) {
                let slot = self.slot.clone();
                monitor.connect_changed(move |_, _, _, _| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.schedule_refresh();
                    }
                });
                monitors.push(monitor);
            }
        }
        *self.academic_paths.borrow_mut() = paths;
    }

    /// Pinta el marcado de búsqueda y la barra desde [`find::FindState`]
    /// (sin recalcular). `focus` lleva la coincidencia activa a la vista
    /// (scroll + cursor, §8.6).
    fn find_repaint(&self, focus: bool) {
        if !self.find_bar.is_open() {
            return;
        }
        let (matches, active_index, active, counter, error) = {
            let state = self.find_bar.state().borrow();
            (
                state.matches().to_vec(),
                state.active_index(),
                state.active_match(),
                state.counter_text(),
                state.error().map(str::to_owned),
            )
        };
        self.editor_pane.set_find_marks(&matches, active_index);
        self.find_bar.set_counter(&counter);
        self.find_bar.set_error(error.as_deref());
        if focus {
            if let Some(range) = active {
                self.editor_pane.focus_find_match(range);
            }
        }
    }

    /// Recalcula la búsqueda sobre el tab activo (§40.1) y pinta.
    ///
    /// `anchor` fija el ancla (cursor al abrir o editar, o el final de un
    /// reemplazo); con `None` el ancla se conserva, de modo que teclear la
    /// consulta no la mueve.
    fn find_refresh(&self, anchor: Option<Position>, focus: bool) {
        if !self.find_bar.is_open() {
            return;
        }
        {
            let mut state = self.find_bar.state().borrow_mut();
            if let Some(position) = anchor {
                state.set_anchor(position);
            }
            let tabs = self.tabs.borrow();
            state.refresh(&tabs.active().editor);
        }
        self.find_repaint(focus);
    }

    /// Abre la barra de búsqueda (§39: `Ctrl+F`) o de reemplazo (`Ctrl+H`)
    /// sobre el tab activo y busca desde el cursor.
    fn find_open(&self, show_replace: bool) {
        self.find_bar.open(show_replace);
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.find_refresh(Some(cursor), true);
        self.find_bar.focus_search();
    }

    /// Acciones de la barra de búsqueda (§40.1).
    fn find_action(&self, action: FindAction) {
        match action {
            FindAction::Search => {
                // El ancla no se mueve al teclear la consulta.
                self.find_refresh(None, true);
            }
            FindAction::Next | FindAction::Previous => {
                {
                    let mut state = self.find_bar.state().borrow_mut();
                    if action == FindAction::Next {
                        state.next();
                    } else {
                        state.previous();
                    }
                }
                // La activa ya avanzó (con wrap §40.1): solo re-marcar y
                // llevarla a la vista.
                self.find_repaint(true);
            }
            FindAction::Replace => {
                let edit = self.find_bar.state().borrow().replace_edit();
                let Some((range, text)) = edit else { return };
                self.tabs.borrow_mut().replace_active(range, &text);
                // «Reemplazar» avanza a la siguiente: `reload_active` fija el
                // ancla en el cursor, que quedó al final del texto insertado
                // (§8.6), y recalcula.
                self.reload_active();
                self.find_repaint(true);
            }
            FindAction::ReplaceAll => {
                let edits = self.find_bar.state().borrow().replace_all_edits();
                if edits.is_empty() {
                    return;
                }
                // §8.5: una sola transacción; un solo undo la revierte entera.
                self.tabs.borrow_mut().replace_all_active(&edits);
                self.reload_active();
                self.find_repaint(true);
            }
            FindAction::Close => {
                self.find_bar.close();
                self.find_bar.state().borrow_mut().clear_results();
                self.editor_pane.clear_find_marks();
            }
        }
    }

    /// Título observable de la ventana (§3.7): tab activo + ámbito (ISSUE-021).
    fn update_title(&self) {
        let title = {
            let tabs = self.tabs.borrow();
            tabs.title(tabs.active_index())
        };
        let text = match &self.scope {
            WindowScope::Standalone => format!("{title} — minmark"),
            WindowScope::Project(root) => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                format!("{title} — minmark · {name}")
            }
        };
        self.window.set_title(Some(&text));
    }

    /// Abre una ventana nueva con su propio ámbito y sus pestañas (ISSUE-021).
    fn spawn_window(&self, scope: WindowScope, paths: &[PathBuf]) {
        build_window(&self.app, scope, paths);
    }

    /// Abre `path` según el ámbito de la ventana (ISSUE-021, `OpenTarget`):
    /// dentro del ámbito, pestaña aquí; fuera del ámbito, ventana standalone
    /// nueva — nunca se rechaza un archivo.
    fn open_path(&self, path: &Path) {
        if self.scope.target_for(path) == OpenTarget::NewStandaloneWindow {
            self.spawn_window(WindowScope::Standalone, &[path.to_path_buf()]);
            return;
        }
        let opened = self.tabs.borrow_mut().open(path);
        match opened {
            Ok(index) => self.select_tab(index),
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Abre `path` en una pestaña o activa la que ya lo tiene abierto
    /// (ISSUE-028 §5: «abrir (o activar) la pestaña del fichero»). La
    /// comparación de rutas admite grafías distintas del mismo fichero.
    fn activate_or_open(&self, path: &Path) {
        let objetivo = ruta_comparable(path);
        let abierta = {
            let tabs = self.tabs.borrow();
            tabs.tabs().iter().position(|tab| {
                tab.path
                    .as_deref()
                    .is_some_and(|p| ruta_comparable(p) == objetivo)
            })
        };
        match abierta {
            Some(index) => self.select_tab(index),
            None => self.open_path(path),
        }
    }

    /// Persiste visibilidad y ancho de la barra lateral (§14.3: estado
    /// restaurable).
    fn persist_explorer_state(&self) {
        let Some(side) = &self.sidebar else { return };
        if side.visible.get() {
            let width = side.paned.position();
            if width > 0 {
                side.state.borrow_mut().width = ExplorerState::clamp_width(width);
            }
        }
        let state = *side.state.borrow();
        if let Err(err) = save_state(&state) {
            eprintln!("minmark: no se pudo guardar el estado del explorador: {err}");
        }
    }

    /// Operaciones §14.8 del explorador (ISSUE-023) sobre el nodo `target`
    /// (`None` = raíz del proyecto).
    fn explorer_op(&self, action: ExplorerAction, target: Option<PathBuf>) {
        let Some(me) = self.slot() else { return };
        let Some(root) = self.scope.project_root() else {
            return;
        };
        match action {
            ExplorerAction::CopyRelativePath => {
                let Some(target) = target else { return };
                // §14.8/§17: ruta relativa al proyecto.
                let rel = minmark_fs::relative_path(&root, &target);
                if let Some(display) = gtk4::gdk::Display::default() {
                    display.clipboard().set_text(&rel);
                }
            }
            ExplorerAction::Delete => {
                // §14.9: a la papelera del sistema, con confirmación §3.7.
                let Some(target) = target else { return };
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    let name = target
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| target.display().to_string());
                    if dialogs::confirm_trash(&window, &name).await {
                        me.trash_target(&target);
                    }
                });
            }
            ExplorerAction::Duplicate => {
                let Some(target) = target else { return };
                match minmark_fs::duplicate(&root, &target) {
                    Ok(copy) => {
                        self.self_ops.register(&copy);
                        self.refresh_explorer();
                    }
                    Err(err) => show_error(&self.window, &err.to_string()),
                }
            }
            ExplorerAction::NewMarkdown | ExplorerAction::NewFolder => {
                // §14.8: crear dentro del nodo seleccionado (archivo → su
                // carpeta; vacío → raíz).
                let parent = match &target {
                    Some(path) if path.is_dir() => path.clone(),
                    Some(path) => path.parent().unwrap_or(&root).to_path_buf(),
                    None => root.clone(),
                };
                let (title, initial) = match action {
                    ExplorerAction::NewMarkdown => ("New Markdown", "nuevo.md"),
                    _ => ("New Folder", "nueva-carpeta"),
                };
                let window = self.window.clone();
                let prompt_window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    title,
                    initial,
                    Rc::new(move |answer| {
                        let Some(name) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        let created = if action == ExplorerAction::NewMarkdown {
                            minmark_fs::create_markdown(&root, &parent, &name)
                        } else {
                            minmark_fs::create_folder(&root, &parent, &name)
                        };
                        match created {
                            Ok(path) => {
                                // Criterio 3 (ISSUE-026): alta propia, no externa.
                                me.self_ops.register(&path);
                                me.refresh_explorer();
                            }
                            Err(err) => show_error(&prompt_window, &err.to_string()),
                        }
                    }),
                );
            }
            ExplorerAction::Rename => {
                let Some(target) = target else { return };
                let initial = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let parent = target.parent().unwrap_or(&root).to_path_buf();
                let window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    "Rename",
                    &initial,
                    Rc::new(move |answer| {
                        let Some(name) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.rename_with_links(&target, &parent.join(name));
                    }),
                );
            }
            ExplorerAction::Move => {
                let Some(target) = target else { return };
                let initial = minmark_fs::relative_path(&root, &target);
                let window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    "Move",
                    &initial,
                    Rc::new(move |answer| {
                        let Some(dest) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.rename_with_links(&target, &root.join(dest));
                    }),
                );
            }
            ExplorerAction::ImportMarkdown => {
                // §15: el nodo del click derecho es la carpeta destino.
                let dest = match &target {
                    Some(path) if path.is_dir() => path.clone(),
                    Some(path) => path.parent().unwrap_or(&root).to_path_buf(),
                    None => root.clone(),
                };
                self.import_markdown_into(Some(dest));
            }
        }
    }

    /// Import Markdown (§15, ISSUE-024): copia un `.md` **externo** dentro del
    /// proyecto (el original queda intacto, colisión → renombrado §18.3).
    ///
    /// `dest_dir` = destino ya elegido (menú contextual del explorador) o
    /// `None` (menú File: se pide con diálogo). Origen (criterio 1): la
    /// pestaña activa si es un `.md` externo al ámbito; si no, el picker.
    fn import_markdown_into(&self, dest_dir: Option<PathBuf>) {
        let Some(me) = self.slot() else { return };
        let root = self.scope.project_root();
        let externo = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            tab.path.clone().filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext == "md" || ext == "markdown")
                    && root
                        .as_ref()
                        .is_none_or(|root| minmark_fs::resolve_in_project(root, path).is_err())
            })
        };
        match externo {
            Some(source) => me.import_source(source, dest_dir),
            None => {
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    let Some(source) = dialogs::open_file(&window).await else {
                        return;
                    };
                    me.import_source(source, dest_dir);
                });
            }
        }
    }

    /// Segundo paso del import (§15, criterio 1–2): elegir la carpeta destino.
    ///
    /// - destino ya elegido (menú contextual del explorador) → copiar ya;
    /// - ventana de proyecto → prompt relativo a la raíz;
    /// - ventana standalone → diálogo de carpeta (la carpeta elegida es la
    ///   raíz de validación §48 de la importación).
    fn import_source(&self, source: PathBuf, dest_dir: Option<PathBuf>) {
        if let Some(dest) = dest_dir {
            let root = self.scope.project_root().unwrap_or_else(|| dest.clone());
            self.import_copy(&source, &dest, &root);
            return;
        }
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        match self.scope.project_root() {
            Some(root) => {
                dialogs::prompt_text(
                    &window,
                    "Import Markdown — destino",
                    ".",
                    Rc::new(move |answer| {
                        let Some(dest) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.import_copy(&source, &root.join(dest), &root);
                    }),
                );
            }
            None => {
                glib::spawn_future_local(async move {
                    let Some(dest) = dialogs::open_folder(&window).await else {
                        return;
                    };
                    me.import_copy(&source, &dest, &dest);
                });
            }
        }
    }

    /// Copia la importación y re-objetiva la pestaña del externo a la copia
    /// (ISSUE-024 criterio 4).
    fn import_copy(&self, source: &Path, dest_dir: &Path, root: &Path) {
        match minmark_fs::import_markdown(root, source, dest_dir) {
            Ok(copy) => {
                self.self_ops.register(&copy);
                self.tabs.borrow_mut().retarget_path(source, &copy);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Rename/movimiento §16 (ISSUE-023): plan all-or-nothing, aplicación con
    /// rollback y retarget de las pestañas afectadas.
    fn rename_with_links(&self, from: &Path, to: &Path) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        // Criterio 3 (ISSUE-026): el movimiento propio no avisa como externo.
        self.self_ops.register(from);
        self.self_ops.register(to);
        let options = minmark_fs::ChangeOptions {
            wiki_links: self.settings.borrow().wiki_links,
        };
        let result = minmark_fs::plan_path_change(&root, from, to, options)
            .and_then(|plan| minmark_fs::apply_path_change(&plan));
        match result {
            Ok(()) => {
                self.tabs.borrow_mut().retarget_path(from, to);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Delete §14.9: a la papelera; las pestañas afectadas se re-objetivan de
    /// forma observable (ISSUE-023 criterio 5).
    fn trash_target(&self, target: &Path) {
        // Criterio 3 (ISSUE-026): la baja propia no avisa como externa.
        self.self_ops.register(target);
        match minmark_fs::trash(target) {
            Ok(()) => {
                self.tabs.borrow_mut().path_deleted(target);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    fn refresh_explorer(&self) {
        if let Some(side) = &self.sidebar {
            side.explorer.refresh();
        }
    }

    /// Texto seleccionado del tab activo para precargar la búsqueda (§4,
    /// estilo VS Code); solo selección de una línea.
    fn selection_seed(&self) -> Option<String> {
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        let range = tab.editor.selection().range();
        let start = tab.editor.offset_of(range.start).ok()?;
        let end = tab.editor.offset_of(range.end).ok()?;
        let text = &tab.editor.document().content()[start..end];
        if text.is_empty() || text.contains('\n') {
            None
        } else {
            Some(text.to_owned())
        }
    }

    /// Muestra `view` en la barra lateral (§40.2: una vista visible cada vez,
    /// estilo VS Code) montándola primero si estaba oculta (§14.3: el ancho
    /// compartido se restaura del estado guardado).
    fn show_sidebar_view(&self, view: SidebarView) {
        let Some(side) = &self.sidebar else { return };
        side.explorer
            .widget()
            .set_visible(view == SidebarView::Explorer);
        side.search
            .widget()
            .set_visible(view == SidebarView::Search);
        side.view.set(view);
        if !side.visible.get() {
            let width = side.state.borrow().width;
            side.paned.set_start_child(Some(&side.widget));
            side.paned.set_position(width);
            side.visible.set(true);
            side.state.borrow_mut().visible = true;
            self.persist_explorer_state();
        }
    }

    /// Bucle de eventos del watcher de proyecto (ISSUE-026): los eventos de
    /// inotify coalescidos refrescan el árbol y el índice (§45) y marcan las
    /// pestañas afectadas (§12.7, sin recarga del buffer; §3.7 observable).
    fn pump_watcher(&self) {
        let Some(watcher) = &self.watcher else { return };
        let events = match watcher.borrow_mut().poll() {
            Ok(events) => events,
            Err(err) => {
                eprintln!("minmark: watcher de proyecto: {err}");
                return;
            }
        };
        if events.is_empty() {
            return;
        }
        let mut touched_tabs = false;
        for event in &events {
            // Criterio 3: nuestras operaciones §14.8 no avisan como externas.
            if self.self_ops.consume(&event.path) {
                continue;
            }
            self.tabs.borrow_mut().mark_external(&event.path);
            touched_tabs = true;
        }
        self.refresh_explorer();
        if events
            .iter()
            .any(|e| e.path.file_name().is_some_and(|n| n == ".mdproject.toml"))
        {
            self.refresh_now();
            // B1 (revisión PR #44, #22): la lista de composiciones de la
            // ventana no se congela en la apertura: se redescubre y las
            // acciones quedan acordes (§20).
            self.refresh_compositions();
        }
        if let Some(root) = self.scope.project_root() {
            spawn_index_refresh(root, self.slot.clone());
        }
        if touched_tabs {
            self.tab_bar.sync(&self.tabs.borrow());
            self.update_title();
        }
    }

    /// Refresco diferido tras cada edición real (§47.1).
    fn schedule_refresh(self: &Rc<Self>) {
        if self.refresh_scheduled.get() {
            return;
        }
        self.refresh_scheduled.set(true);
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(REFRESH_DEBOUNCE_MS), move || {
            me.refresh_scheduled.set(false);
            me.refresh_now();
            let byte = {
                let tabs = me.tabs.borrow();
                let editor = &tabs.active().editor;
                editor.offset_of(editor.selection().head).ok()
            };
            if let Some(byte) = byte {
                me.preview.follow_edit(byte as u32);
            }
            me.refresh_intelligence(true);
            me.refresh_navigation();
        });
    }

    fn select_tab(&self, index: usize) {
        self.tabs.borrow_mut().set_active(index);
        self.reload_active();
    }

    fn apply_view(&self) {
        let (show_editor, show_preview) = match self.view.get() {
            ViewMode::Editor => (true, false),
            ViewMode::Split => (true, true),
            ViewMode::Preview => (false, true),
        };
        self.editor_pane.widget().set_visible(show_editor);
        self.preview.widget().set_visible(show_preview);
    }

    // --- Composiciones (§20, ISSUE-022/023) ---------------------------------

    /// Contenido actual de los buffers abiertos con ruta (ISSUE-023): la
    /// composición usa el buffer sin guardar en lugar del disco (§6.1).
    fn open_buffers(&self) -> Vec<(PathBuf, String)> {
        self.tabs
            .borrow()
            .tabs()
            .iter()
            .filter_map(|tab| {
                Some((
                    ruta_comparable(tab.path.as_ref()?),
                    tab.editor.document().content().to_owned(),
                ))
            })
            .collect()
    }

    /// Redescubre las composiciones del proyecto (B1, revisión PR #44, #22):
    /// relée `.mdproject.toml` y reevalúa el `enabled` de
    /// `win.preview-composition` / `win.export-composition-pdf` (§20). Lo
    /// pide el watcher al cambiar ese archivo; al abrir la ventana ya
    /// descubre `build_window`. `build` vuelve a leer `.mdproject.toml` para
    /// la cadena §58 (`load_composition_at`), de modo que `files` y
    /// `overrides` son de lecturas próximas —eventualmente consistentes con
    /// el watcher—, no de una misma lectura atómica.
    fn refresh_compositions(&self) {
        let root = match &self.scope {
            WindowScope::Project(root) => Some(root.as_path()),
            WindowScope::Standalone => None,
        };
        let disponible = composition::refresh(&mut self.compositions.borrow_mut(), root);
        let grupo = self
            .action_group
            .borrow()
            .as_ref()
            .and_then(|grupo| grupo.upgrade());
        if let Some(group) = grupo {
            actions::refresh_composition_actions(&group, disponible);
        }
    }

    /// Ejecuta `then` con la composición elegida (ISSUE-022): con una sola
    /// no hay diálogo; con varias se presenta la selección (§20.1). Sin
    /// composiciones no hace nada (la acción está deshabilitada).
    fn with_composition(&self, then: Rc<dyn Fn(&AppCmds, Composition)>) {
        let nombres: Vec<String> = self
            .compositions
            .borrow()
            .iter()
            .map(|comp| comp.name.clone())
            .collect();
        match nombres.len() {
            0 => {}
            1 => {
                let comp = self.compositions.borrow()[0].clone();
                then(self, comp);
            }
            _ => {
                let Some(me) = self.slot() else { return };
                dialogs::choose_composition(
                    &self.window,
                    &nombres,
                    Rc::new(move |elegida| {
                        let Some(elegida) = elegida else { return };
                        let comp = me
                            .compositions
                            .borrow()
                            .iter()
                            .find(|comp| comp.name == elegida)
                            .cloned();
                        if let Some(comp) = comp {
                            then(&me, comp);
                        }
                    }),
                );
            }
        }
    }

    /// Ventana temporal del Preview combinado (patrón Outline/Backlinks):
    /// se crea en el primer uso y después se reutiliza; Escape o la ✕ la
    /// ocultan sin destruir la WebView.
    fn composition_window(&self) -> Rc<CompositionPreview> {
        if let Some(preview) = self.composition_preview.borrow().as_ref() {
            return Rc::clone(preview);
        }
        let window = gtk4::Window::builder()
            .transient_for(&self.window)
            .default_width(1000)
            .default_height(800)
            .build();
        let pane = Rc::new(PreviewPane::new());
        // El clic en un enlace no navega fuera del documento combinado
        // (§3.4: la WebView conserva la página, igual que el Preview normal);
        // un destino interno `#fragmento` (S1, ISSUE-024, revisión PR #44) sí
        // desplaza in-page hasta su elemento (§26).
        // Captura DÉBIL: el gestor de contenido de la WebView retiene este
        // callback y `pane` retiene la WebView, así que una captura fuerte
        // cerraría el ciclo y la ventana del Preview no se liberaría al
        // cerrarse (misma razón que el grupo «win», B1).
        let pane_interna = Rc::downgrade(&pane);
        pane.connect_link_activation(Rc::new(move |destino| {
            if let Some(pane) = pane_interna.upgrade() {
                pane.scroll_to_fragment(&destino);
            }
        }));
        window.set_child(Some(&pane.widget()));
        {
            let on_escape = window.clone();
            let controller = gtk4::EventControllerKey::new();
            controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    on_escape.set_visible(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(controller);
        }
        window.connect_close_request(move |window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });
        let preview = Rc::new(CompositionPreview {
            window,
            pane,
            generation: Cell::new(0),
        });
        *self.composition_preview.borrow_mut() = Some(Rc::clone(&preview));
        preview
    }

    /// Abre la ventana temporal con el Preview COMBINADO de la composición
    /// (§20.4, ISSUE-023): continuo o paginado según el `pagination`
    /// efectivo; el render corre en un hilo y se entrega al hilo de UI
    /// (patrón §47.5). Consultar no escribe los fuentes (§20).
    fn show_composition_preview(&self, comp: &Composition) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        let combined = composition::build(&root, comp, &self.open_buffers());
        let preview = self.composition_window();
        preview
            .window
            .set_title(Some(&format!("Preview — {}", combined.name)));
        preview.window.present();
        let generation = preview.generation.get().wrapping_add(1);
        preview.generation.set(generation);
        let base = composition::base_uri(&root);
        // Misma composición ⇒ misma página (actualización in-place §10.2);
        // otra composición ⇒ recarga completa de la WebView.
        let clave = (
            Some(root.join(format!(".mdproject-composition:{}", combined.name))),
            0u64,
        );
        let pane = Rc::clone(&preview.pane);
        let slot = self.slot.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let html = composition::render(combined);
            let _ = tx.send(html);
        });
        let ventana = Rc::clone(&preview);
        deliver_to_quickopen(rx, slot, move |_me, html| {
            if ventana.generation.get() != generation {
                return; // otro render más reciente ya ocupó la ventana
            }
            pane.update_html(&html, Some(&base), clave.clone());
        });
    }

    /// Exporta la composición elegida como un único PDF (§20.4, ISSUE-023)
    /// por el MISMO flujo que la exportación individual: diálogo de
    /// destino, sobrescritura y errores idénticos (§32.1).
    fn export_composition(&self, comp: &Composition) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        let combined = composition::build(&root, comp, &self.open_buffers());
        let suggested = format!("{}.pdf", combined.name);
        let crate::composition::Combined {
            composed,
            settings,
            meta,
            root,
            ..
        } = combined;
        self.run_pdf_export(suggested, move || {
            export::export_composed(composed.ast, &settings, &root, &meta)
        });
    }

    /// Flujo de exportación PDF único (§32.1): diálogo de destino con su
    /// confirmación de sobrescritura (`dialogs::save_pdf`), escritura del
    /// bytes y errores visibles con el mismo patrón que siempre. `produce`
    /// genera los bytes por el camino que corresponda (individual o
    /// composición) — la lógica del diálogo no se duplica.
    fn run_pdf_export(
        &self,
        suggested: String,
        produce: impl FnOnce() -> Result<Vec<u8>, minmark_pdf::PdfError> + 'static,
    ) {
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::save_pdf(&window, &suggested).await else {
                return;
            };
            match produce() {
                Ok(bytes) => {
                    if let Err(err) = std::fs::write(&path, &bytes) {
                        show_error(&window, &err.to_string());
                    }
                }
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }
}

impl Commands for AppCmds {
    fn outline(&self) {
        self.show_navigation(false);
    }
    fn follow_link(&self) {
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.activate_link(Some(cursor), None);
    }
    fn backlinks(&self) {
        self.show_navigation(true);
    }
    fn new(&self) {
        self.tabs.borrow_mut().new_tab();
        self.reload_active();
    }

    fn new_window(&self) {
        // ISSUE-021: un archivo nuevo fuera de proyecto vive en una ventana
        // standalone nueva (como Notepad++), sin restricción de carpeta.
        self.spawn_window(WindowScope::Standalone, &[]);
    }

    fn open(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::open_file(&window).await else {
                return;
            };
            // ISSUE-021: dentro del ámbito de la ventana, pestaña aquí; fuera
            // del ámbito (archivo ajeno al proyecto), ventana standalone
            // nueva — nunca se rechaza el archivo.
            me.open_path(&path);
        });
    }

    fn quick_open(&self) {
        // §41/§39: `Ctrl+P` presenta la paleta modal sobre esta ventana
        // (§36) con el contexto del tab activo (`present` repinta la
        // consulta desde cero, fuzzy limpio §41).
        self.quickopen.present();
    }

    fn open_folder(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(folder) = dialogs::open_folder(&window).await else {
                return;
            };
            // ISSUE-021: cada carpeta se abre en su propia ventana de proyecto.
            me.spawn_window(WindowScope::Project(folder), &[]);
        });
    }

    fn import_markdown(&self) {
        // §15: sin proyecto no hay carpeta destino (decisión registrada en
        // ISSUE-024: Import vive en ventanas de proyecto).
        self.import_markdown_into(None);
    }

    fn save(&self) {
        let result = self.tabs.borrow_mut().save_active();
        match result {
            Ok(()) => self.reload_active(),
            // §12.4: sin archivo asignado, Save As.
            Err(minmark_fs::FsError::MissingPath) => self.save_as(),
            // §12.7: nunca sobrescritura silenciosa; solo con confirmación
            // explícita del usuario (§3.7) se sobrescribe el cambio externo.
            Err(minmark_fs::FsError::ExternalModification { path }) => {
                let Some(me) = self.slot() else { return };
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    if dialogs::confirm_overwrite_external(&window, &path).await {
                        match me.tabs.borrow_mut().save_active_confirmed() {
                            Ok(()) => me.reload_active(),
                            Err(err) => show_error(&window, &err.to_string()),
                        }
                    }
                });
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    fn save_as(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        let suggested = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            match &tab.path {
                Some(path) => path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sin-titulo.md".to_owned()),
                None => format!("sin-titulo-{}.md", tab.untitled_number),
            }
        };
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::save_file_as(&window, &suggested).await else {
                return;
            };
            let saved = me.tabs.borrow_mut().save_active_as(&path);
            match saved {
                Ok(()) => me.reload_active(),
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }

    fn export_pdf(&self) {
        // §32.1 (ISSUE-035): exporta el documento activo tal cual está en el
        // buffer (§6.1), con los ajustes `pdf_*` vigentes. El motor ya está
        // probado en `minmark-pdf` (ISSUE-018).
        let Some(_me) = self.slot() else { return };
        // El front matter del snapshot puede preceder al refresco diferido.
        self.refresh_now();
        let root = self.scope.project_root();
        let (text, settings, suggested, document_path) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            let suggested = match &tab.path {
                Some(path) => path
                    .file_stem()
                    .map(|stem| format!("{}.pdf", stem.to_string_lossy()))
                    .unwrap_or_else(|| "documento.pdf".to_owned()),
                None => format!("sin-titulo-{}.pdf", tab.untitled_number),
            };
            (
                tab.editor.document().content().to_owned(),
                self.settings.borrow().clone(),
                suggested,
                tab.path.clone(),
            )
        };
        let meta = minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(&text));
        self.run_pdf_export(suggested, move || {
            export::export_document_at(
                &text,
                &settings,
                document_path.as_deref(),
                root.as_deref(),
                &meta,
            )
        });
    }

    fn export_composition_pdf(&self) {
        // §20.4/ISSUE-023: selección si hay varias y después el MISMO
        // flujo de diálogo que `export_pdf` (`run_pdf_export`).
        self.with_composition(Rc::new(|me, comp| me.export_composition(&comp)));
    }

    fn preview_composition(&self) {
        // §20.4/ISSUE-023: selección si hay varias y ventana temporal con
        // el Preview combinado (patrón Outline/Backlinks).
        self.with_composition(Rc::new(|me, comp| me.show_composition_preview(&comp)));
    }

    fn compositions_available(&self) -> bool {
        // ISSUE-022: sin proyecto con `[[composition]]` las acciones de
        // composición quedan deshabilitadas en menú y paleta.
        !self.compositions.borrow().is_empty()
    }

    fn close_tab(&self) {
        let (modified, title) = {
            let tabs = self.tabs.borrow();
            let index = tabs.active_index();
            (tabs.is_modified(index), tabs.title(index))
        };
        if !modified {
            self.tabs.borrow_mut().close_active();
            self.reload_active();
            return;
        }
        // §3.7: descartar cambios solo con confirmación explícita.
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            if dialogs::confirm_discard(&window, &title).await {
                me.tabs.borrow_mut().close_active();
                me.reload_active();
            }
        });
    }

    fn undo(&self) {
        self.tabs.borrow_mut().undo_active();
        self.reload_active();
    }

    fn redo(&self) {
        self.tabs.borrow_mut().redo_active();
        self.reload_active();
    }

    fn find(&self) {
        // §40.1/§39: Ctrl+F, modo búsqueda.
        self.find_open(false);
    }

    fn replace(&self) {
        // §40.1/§39: Ctrl+H, con la fila de reemplazo visible.
        self.find_open(true);
    }

    fn project_search(&self) {
        // §40.2/§39: `Ctrl+Shift+F` muestra la vista de búsqueda de la barra
        // lateral y enfoca el campo. En ventanas standalone no hay panel: el
        // comando es sin-op (§40.2).
        let Some(side) = &self.sidebar else { return };
        // Una vista cada vez (modelo VS Code §40.2): manda la búsqueda y el
        // explorador queda oculto.
        self.show_sidebar_view(SidebarView::Search);
        // Precarga desde la selección del editor activo (§4, estilo VS Code):
        // con selección de una línea fija la consulta, precarga el campo de
        // reemplazo y relanza.
        if let Some(seed) = self.selection_seed() {
            side.search
                .run_search(&seed, SearchOptions::default(), Some(String::new()));
        }
        side.search.focus_search();
    }

    fn toggle_mode(&self) {
        self.tabs.borrow_mut().toggle_mode_active();
        self.reload_active();
    }

    /// §8.3 (ISSUE-030): alterna la visibilidad de la sintaxis en Visual
    /// entre «ocultar siempre» y «revelar la del elemento editado».
    fn toggle_visual_syntax(&self) {
        {
            let mut settings = self.settings.borrow_mut();
            settings.visual_syntax = match settings.visual_syntax {
                VisualSyntax::Hidden => VisualSyntax::RevealCurrent,
                VisualSyntax::RevealCurrent => VisualSyntax::Hidden,
            };
        }
        self.editor_pane
            .set_visual_syntax(self.settings.borrow().visual_syntax);
        self.reload_active();
    }

    fn toggle_explorer(&self) {
        // §14.3: la barra lateral solo existe en ventanas de proyecto
        // (ISSUE-022); en standalone Ctrl+B es un no-op documentado.
        let Some(side) = &self.sidebar else { return };
        // §40.2 (estilo VS Code): si manda la búsqueda, Ctrl+B es lo inverso
        // de `win.project-search` y vuelve al explorador; si ya manda el
        // explorador, muestra/oculta la barra lateral (§14.3).
        if side.view.get() == SidebarView::Search {
            self.show_sidebar_view(SidebarView::Explorer);
            return;
        }
        let visible = !side.visible.get();
        if visible {
            let width = side.state.borrow().width;
            side.paned.set_start_child(Some(&side.widget));
            side.paned.set_position(width);
        } else {
            // El ancho actual se guarda antes de ocultar (§14.3).
            self.persist_explorer_state();
            side.paned.set_start_child(Option::<&Widget>::None);
        }
        side.visible.set(visible);
        side.state.borrow_mut().visible = visible;
        self.persist_explorer_state();
    }

    fn set_view(&self, mode: ViewMode) {
        self.view.set(mode);
        self.apply_view();
    }

    fn about(&self) {
        let about = gtk4::AboutDialog::builder()
            .program_name("minmark")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Editor Markdown académico — Privacy & Control Promise (§3)")
            .license("Apache-2.0 — ver LICENSE")
            .website("https://minmark.local")
            .modal(true)
            .transient_for(&self.window)
            .build();
        about.present();
    }

    fn shortcuts(&self) {
        let mut detail = String::new();
        for (action, accel) in menus::ACCELERATORS {
            detail.push_str(&format!("{accel}\t{action}\n"));
        }
        let alert = gtk4::AlertDialog::builder()
            .message("Atajos (§39)")
            .detail(detail.trim_end())
            .buttons(["Cerrar"])
            .modal(true)
            .build();
        alert.choose(
            Some(&self.window),
            std::option::Option::<&gtk4::gio::Cancellable>::None,
            |_| {},
        );
    }
}

/// Ruta comparable: canónica si se resuelve y la propia si no (§12.7: dos
/// grafías de un mismo fichero son el mismo fichero).
fn ruta_comparable(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Diálogo de error modal (§3.7: acciones observables).
fn show_error(parent: &ApplicationWindow, message: &str) {
    let alert = gtk4::AlertDialog::builder()
        .message("Error")
        .detail(message)
        .buttons(["Aceptar"])
        .modal(true)
        .build();
    alert.choose(
        Some(parent),
        std::option::Option::<&gtk4::gio::Cancellable>::None,
        |_| {},
    );
}

/// Color del separador del split editor↔preview (§9.1): barra vertical
/// visible — gris sobre tema claro y negro sobre tema oscuro (el negro sobre
/// fondo claro se perdería y el gris claro sobre oscuro, también).
fn split_separator_color(theme: ResolvedTheme) -> &'static str {
    match theme {
        ResolvedTheme::Light => "#767676",
        ResolvedTheme::Dark => "#050505",
    }
}

/// CSS de la UI GTK desde la [`Palette`] del tema resuelto (ISSUE-017):
/// lógica pura y testeable. Incluye la barra vertical del split §9.1, que
/// solo se aplica al `Paned` con clase `mm-split` (el del explorador no).
fn theme_css(palette: &Palette, theme: ResolvedTheme) -> String {
    format!(
        "window {{ background-color: {bg}; color: {fg}; }}\n\
         textview, textview > text {{ font-family: monospace; background-color: {bg}; color: {fg}; }}\n\
         .mm-tab-bar {{ background-color: {code}; }}\n\
         .mm-tab-bar button {{ padding: 2px 10px; }}\n\
         .mm-tab-bar button.active {{ border-bottom: 2px solid {fg}; font-weight: bold; }}\n\
         .mm-explorer {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview button {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview label {{ color: {fg}; }}\n\
         paned.mm-split > separator {{ min-width: 4px; background: {split}; }}\n",
        bg = palette.bg,
        fg = palette.fg,
        code = palette.code_bg,
        split = split_separator_color(theme),
    )
}

/// Estiliza la UI GTK con la [`Palette`] del tema resuelto (ISSUE-017).
fn install_theme_css(palette: &Palette, theme: ResolvedTheme) {
    let css = theme_css(palette, theme);
    let provider = CssProvider::new();
    provider.load_from_data(&css);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn build_window(app: &Application, scope: WindowScope, open_paths: &[PathBuf]) -> Rc<AppCmds> {
    let settings = Settings::default();
    let theme = resolve_theme(settings.theme, detect_system_theme());
    let palette = match theme {
        ResolvedTheme::Light => Palette::LIGHT,
        ResolvedTheme::Dark => Palette::DARK,
    };
    install_theme_css(&palette, theme);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("minmark")
        .default_width(1280)
        .default_height(800)
        .build();

    // Modelo + vistas (§6.1: el modelo vive en el core).
    let slot: Rc<CmdSlot> = Rc::new(RefCell::new(None));
    let tabs = Rc::new(RefCell::new(TabManager::new()));
    tabs.borrow_mut().set_scope(scope.clone());
    for path in open_paths {
        let opened = tabs.borrow_mut().open(path);
        match opened {
            Ok(index) => tabs.borrow_mut().set_active(index),
            Err(err) => eprintln!("minmark: no se pudo abrir {}: {err}", path.display()),
        }
    }

    let sink = Rc::new(Sink { tabs: tabs.clone() });
    let on_edited = {
        let slot = slot.clone();
        Rc::new(move || {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.schedule_refresh();
            }
        })
    };
    let editor_pane = Rc::new(EditorPane::new(sink, on_edited, &palette));
    // §8.3 (ISSUE-030): visibilidad de la sintaxis en Visual según ajuste.
    editor_pane.set_visual_syntax(settings.visual_syntax);
    let preview = Rc::new(PreviewPane::new());
    editor_pane.connect_link_activation({
        let slot = slot.clone();
        Rc::new(move |at| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.activate_link(Some(at), None);
            }
        })
    });
    preview.connect_link_activation({
        let slot = slot.clone();
        Rc::new(move |dest| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.activate_link(None, Some(dest));
            }
        })
    });
    editor_pane.connect_completion({
        let slot = slot.clone();
        Rc::new(move |item| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.accept_completion(item);
            }
        })
    });
    let tab_bar = Rc::new(TabBar::new({
        let slot = slot.clone();
        Rc::new(move |index| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.select_tab(index);
            }
        })
    }));

    // Barra de búsqueda/reemplazo del archivo actual (§40.1, ISSUE-027):
    // acciones en tardío por [`CmdSlot`], como el resto de widgets.
    let find_bar = Rc::new(FindBar::new({
        let slot = slot.clone();
        Rc::new(move |action| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.find_action(action);
            }
        })
    }));

    // Paleta Quick Open (§41, ISSUE-029): modal sobre esta ventana (§36) y
    // con la apertura en tardío por [`CmdSlot`], como el resto de widgets.
    let quickopen = Rc::new(QuickOpen::new());
    let navigation_list = Rc::new(QuickOpen::new());
    navigation_list.set_transient_for(window.upcast_ref::<gtk4::Window>());
    navigation_list.set_on_open({
        let slot = slot.clone();
        Rc::new(move |path, line| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                if path.is_empty() {
                    if let Some(line) = line {
                        me.focus_line(line);
                    }
                } else {
                    let target = me.navigation_positions.borrow().get(&path).cloned();
                    if let Some((path, at)) = target {
                        me.open_quickopen_entry(&path, Some(at.line + 1));
                        if me
                            .tabs
                            .borrow()
                            .active()
                            .path
                            .as_ref()
                            .zip(me.quickopen_base())
                            .is_some_and(|(p, base)| {
                                minmark_fs::relative_path(&base, &ruta_comparable(p)) == path
                            })
                        {
                            me.editor_pane.focus_find_match(Range::new(at, at));
                        }
                    }
                }
            }
        })
    });
    quickopen.set_transient_for(window.upcast_ref::<gtk4::Window>());
    quickopen.set_on_open({
        let slot = slot.clone();
        Rc::new(move |rel_path: String, line: Option<u32>| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.open_quickopen_entry(&rel_path, line);
            }
        })
    });

    // Barra lateral §14.3/§40.2 (ISSUE-022/028): el explorador y la búsqueda
    // de proyecto solo existen en ventanas de proyecto y comparten sidebar
    // (§40.2); el estado (visibilidad/ancho) se restaura al abrir.
    let explorer_state = load_state();
    let lateral = match &scope {
        WindowScope::Project(root) => {
            let on_open: Rc<dyn Fn(PathBuf)> = {
                let slot = slot.clone();
                Rc::new(move |path: PathBuf| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.open_path(&path);
                    }
                })
            };
            let on_action: Rc<dyn Fn(ExplorerAction, Option<PathBuf>)> = {
                let slot = slot.clone();
                Rc::new(move |action, target| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.explorer_op(action, target);
                    }
                })
            };
            let explorer = Rc::new(ExplorerPane::new(root, on_open, on_action));

            // §14.4 (ISSUE-029): el toggle `Show All Files` del explorador
            // alimenta la consulta de Quick Open.
            explorer.set_on_show_all_changed({
                let quickopen = quickopen.clone();
                Rc::new(move |show_all: bool| quickopen.set_show_all_files(show_all))
            });

            // Búsqueda en el proyecto (§40.2, ISSUE-028) con su raíz (§14.1) y
            // callbacks en tardío por [`CmdSlot`], como el resto de widgets.
            let search = Rc::new(SearchPanel::new());
            search.set_root(root);

            // Activar un resultado (ISSUE-028 §5): abrir (o activar) la
            // pestaña del fichero y colocar el cursor (§8.6: las posiciones
            // del callback son 1-based y las del editor 0-based; la columna
            // cuenta caracteres Unicode).
            search.on_open_result({
                let slot = slot.clone();
                move |rel_path: String, line: usize, col: usize| {
                    let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                        return;
                    };
                    let Some(root) = me.scope.project_root() else {
                        return;
                    };
                    let path = root.join(&rel_path);
                    me.activate_or_open(&path);
                    // El cursor solo se coloca si la pestaña activa es ese
                    // fichero (`open_path` pudo abrir una ventana nueva).
                    let activo = me
                        .tabs
                        .borrow()
                        .active()
                        .path
                        .as_deref()
                        .is_some_and(|p| ruta_comparable(p) == ruta_comparable(&path));
                    if !activo {
                        return;
                    }
                    let position =
                        Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32);
                    me.tabs
                        .borrow_mut()
                        .set_selection_active(Selection::collapsed(position));
                    me.reload_active();
                }
            });

            // Reemplazo aplicado o deshecho (ISSUE-028 §5): orden exacto del
            // criterio — escrituras propias, pestañas, derivados §45 y
            // repintado del activo.
            search.on_replaced({
                let slot = slot.clone();
                move |summary: ReplaceSummary| {
                    let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                        return;
                    };
                    let Some(root) = me.scope.project_root() else {
                        return;
                    };
                    // 1. Escrituras PROPIAS §14.8 (ISSUE-026 criterio 3):
                    //    registradas antes de que el watcher (§46) las vea.
                    for rel in &summary.files {
                        me.self_ops.register(&root.join(rel));
                    }
                    // 2. Sincronizar las pestañas abiertas (§12.7): la limpia
                    //    se recarga en silencio (la escritura es nuestra); la
                    //    modificada NUNCA se recarga — solo se marca y el
                    //    usuario decide al guardar (§3.7). Las operaciones de
                    //    `TabManager` son por path (puede haber varias pestañas
                    //    del mismo fichero): primero se marca el path si alguna
                    //    de sus pestañas está modificada y después la recarga
                    //    limpia el banderín de las pestañas limpias.
                    let objetivos: Vec<PathBuf> = summary
                        .files
                        .iter()
                        .map(|rel| ruta_comparable(&root.join(rel)))
                        .collect();
                    let mut active_reloaded = false;
                    {
                        let mut tabs = me.tabs.borrow_mut();
                        let active = tabs.active_index();
                        let mut grupos: Vec<(PathBuf, Vec<usize>)> = Vec::new();
                        for index in 0..tabs.len() {
                            let Some(path) = tabs.tabs()[index].path.clone() else {
                                continue;
                            };
                            if !objetivos.contains(&ruta_comparable(&path)) {
                                continue;
                            }
                            match grupos.iter_mut().find(|(p, _)| *p == path) {
                                Some((_, indices)) => indices.push(index),
                                None => grupos.push((path, vec![index])),
                            }
                        }
                        for (path, indices) in grupos {
                            if indices.iter().any(|&i| tabs.is_modified(i)) {
                                tabs.mark_external(&path);
                            }
                            if indices.iter().any(|&i| !tabs.is_modified(i)) {
                                if let Err(err) = tabs.reload_from_disk(&path) {
                                    show_error(&me.window, &err.to_string());
                                    continue;
                                }
                                active_reloaded |= indices.contains(&active);
                            }
                        }
                    }
                    // 3. Derivados §45 (headings y referencias cambian).
                    spawn_index_refresh(root, slot.clone());
                    // 4. Preview/activo: repintar solo si la activa se recargó.
                    if active_reloaded {
                        me.reload_active();
                    }
                }
            });

            Some((explorer, search))
        }
        WindowScope::Standalone => None,
    };

    // §45/§46 (ISSUE-025/026): índice del proyecto y watcher del subárbol en
    // ventanas de proyecto. El índice se construye en una tarea aparte
    // (§47.1/§47.5); el watcher solo EMITE eventos (§46: sin rescan continuo,
    // §12.7: sin recarga automática).
    let watcher = match &scope {
        WindowScope::Project(root) => {
            spawn_index_refresh(root.clone(), slot.clone());
            match minmark_fs::InotifyWatcher::new() {
                Ok(mut watcher) => {
                    if let Err(err) = minmark_fs::Watcher::watch_tree(&mut watcher, root) {
                        eprintln!("minmark: sin watching de proyecto: {err}");
                    }
                    Some(Rc::new(RefCell::new(watcher)))
                }
                Err(err) => {
                    eprintln!("minmark: sin watching de proyecto: {err}");
                    None
                }
            }
        }
        WindowScope::Standalone => None,
    };

    // Layout §36: barra de menús superior sin toolbar permanente.
    let menubar = PopoverMenuBar::from_model(Some(&menus::build_menubar()));
    menubar.set_hexpand(true);
    let paned = Paned::new(Orientation::Horizontal);
    // §9.1: separador editor|preview como barra vertical visible (`mm-split`;
    // el `Paned` del explorador no lleva la clase y queda como estaba).
    paned.add_css_class("mm-split");
    paned.set_start_child(Some(&editor_pane.widget()));
    paned.set_end_child(Some(&preview.widget()));
    paned.set_position(640);
    paned.set_wide_handle(true);

    // §14.3/§40.2: `barra lateral | editor+preview`; la barra lateral
    // (`explorador | búsqueda`, una vista visible cada vez) es el start-child
    // del `Paned` exterior y su posición es el ancho compartido, restaurado
    // del estado guardado.
    let (content, sidebar) = match lateral {
        Some((explorer, search)) => {
            let sidebar_box = GtkBox::new(Orientation::Vertical, 0);
            sidebar_box.append(&explorer.widget());
            sidebar_box.append(search.widget());
            // Al abrir manda el explorador (§40.2).
            search.widget().set_visible(false);
            let outer = Paned::new(Orientation::Horizontal);
            outer.set_wide_handle(true);
            outer.set_end_child(Some(&paned));
            if explorer_state.visible {
                outer.set_start_child(Some(&sidebar_box));
                outer.set_position(explorer_state.width);
            }
            let panel = Rc::new(Sidebar {
                widget: sidebar_box,
                explorer,
                search,
                view: Cell::new(SidebarView::Explorer),
                paned: outer.clone(),
                visible: Cell::new(explorer_state.visible),
                state: RefCell::new(explorer_state),
            });
            (outer.upcast::<Widget>(), Some(panel))
        }
        None => (paned.clone().upcast(), None),
    };

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&menubar);
    vbox.append(&tab_bar.widget());
    vbox.append(&find_bar.widget());
    vbox.append(&content);
    content.set_vexpand(true);
    vbox.set_vexpand(true);
    window.set_child(Some(&vbox));

    // §20 (ISSUE-022): composiciones del `.mdproject.toml` de la raíz del
    // proyecto activo; en standalone la lista queda vacía y `install_actions`
    // deshabilita las acciones de composición (`compositions_available`). El
    // redescubrimiento en caliente (B1) usa el mismo `composition::refresh`.
    let mut compositions = Vec::new();
    composition::refresh(&mut compositions, scope.project_root().as_deref());

    let cmds = Rc::new(AppCmds {
        app: app.clone(),
        window: window.clone(),
        scope,
        tabs: tabs.clone(),
        editor_pane,
        preview,
        tab_bar,
        find_bar,
        quickopen,
        index: RefCell::new(None),
        index_generation: Cell::new(0),
        intelligence_generation: Cell::new(0),
        intelligence_busy: Cell::new(false),
        intelligence_pending: Cell::new(false),
        intelligence_suggest: Cell::new(false),
        completion_source: RefCell::new(None),
        navigation_list,
        navigation_kind: Cell::new(false),
        navigation_positions: RefCell::new(std::collections::BTreeMap::new()),
        recent: RefCell::new(Vec::new()),
        quickopen_dir: RefCell::new(None),
        sidebar,
        watcher,
        compositions: RefCell::new(compositions),
        action_group: RefCell::new(None),
        composition_preview: RefCell::new(None),
        self_ops: Rc::new(SelfOps::default()),
        settings: RefCell::new(settings),
        academic_monitors: RefCell::new(Vec::new()),
        academic_paths: RefCell::new(Vec::new()),
        view: Rc::new(Cell::new(ViewMode::Split)),
        refresh_scheduled: Rc::new(Cell::new(false)),
        slot: slot.clone(),
    });
    *slot.borrow_mut() = Some(Rc::downgrade(&cmds));
    // B1: la ventana conserva una referencia DÉBIL al grupo para reevaluar
    // el `enabled` de las acciones de composiciones cuando la lista cambie
    // (§20) sin cerrar el ciclo de referencias con `cmds`.
    *cmds.action_group.borrow_mut() =
        Some(install_actions(&window, app, cmds.clone()).downgrade());

    // ISSUE-026 (§46): bucle de eventos del watcher en el hilo de UI. Se detiene
    // con la ventana.
    if cmds.watcher.is_some() {
        glib::timeout_add_local(Duration::from_millis(WATCHER_POLL_MS), {
            let slot = slot.clone();
            let window = window.downgrade();
            move || {
                if window.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    me.pump_watcher();
                }
                glib::ControlFlow::Continue
            }
        });
    }

    // §3.7: cerrar la ventana con cambios sin guardar pide confirmación.
    window.connect_close_request({
        let cmds = cmds.clone();
        move |window| {
            // §14.3: al cerrar se persiste el estado del explorador.
            cmds.persist_explorer_state();
            let any_modified = cmds
                .tabs
                .borrow()
                .tabs()
                .iter()
                .any(|tab| tab.editor.document().is_modified());
            if !any_modified {
                return glib::Propagation::Proceed;
            }
            let window = window.clone();
            glib::spawn_future_local(async move {
                if dialogs::confirm_discard(&window, "los documentos abiertos").await {
                    window.destroy();
                }
            });
            glib::Propagation::Stop
        }
    });

    cmds.reload_active();
    cmds.apply_view();
    // Cambios de configuración global, también con pestañas sin editar.
    // Solo se consulta el stamp del archivo, nunca se recorre el proyecto.
    let mut config_stamp = settings::global_path()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    glib::timeout_add_local(Duration::from_secs(1), {
        let slot = slot.clone();
        let window = window.downgrade();
        move || {
            if window.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            let stamp = settings::global_path()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok());
            if stamp != config_stamp {
                config_stamp = stamp;
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    me.refresh_now();
                }
            }
            glib::ControlFlow::Continue
        }
    });
    window.present();
    cmds
}

/// Garantiza que el sandbox de WebKit puede funcionar antes de crear la
/// WebView (fix de arranque: crash por `dbus-proxy` en kernels sin user
/// namespaces).
///
/// WebKitGTK lanza sus procesos Web dentro de bubblewrap, que necesita user
/// namespaces sin privilegios. En kernels que los restringen (p. ej. Ubuntu
/// con `kernel.apparmor_restrict_unprivileged_userns=1`) el lanzamiento del
/// `dbus-proxy` dentro de bwrap aborta el proceso entero (`Failed to fully
/// launch dbus-proxy` → SIGTRAP). Si el sandbox no puede arrancar, se
/// desactiva explícitamente con la variable que WebKit define para ello:
/// minmark es una app local sin red (§3.4) y la alternativa es un crash al
/// abrir cualquier ventana con Preview (decisión registrada en CHANGELOG).
fn ensure_webkit_sandbox_usable() {
    let bwrap_ok = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "/bin/true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !bwrap_ok {
        std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
    }
}

fn main() {
    // Antes de tocar ningún widget de WebKit (§4.5: el sandbox es un medio,
    // no el fin; un crash en el arranque no es aceptable).
    ensure_webkit_sandbox_usable();
    // Argumentos (ISSUE-021): cada carpeta abre una ventana de proyecto y los
    // archivos una ventana standalone con sus pestañas (como Notepad++).
    let mut files: Vec<PathBuf> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        let path = PathBuf::from(arg);
        if path.is_dir() {
            folders.push(path);
        } else {
            files.push(path);
        }
    }
    let app = Application::builder().application_id("app.minmark").build();
    app.connect_activate(move |app| {
        if !files.is_empty() {
            build_window(app, WindowScope::Standalone, &files);
        }
        for folder in &folders {
            build_window(app, WindowScope::Project(folder.clone()), &[]);
        }
        if files.is_empty() && folders.is_empty() {
            build_window(app, WindowScope::Standalone, &[]);
        }
    });
    // Los ficheros de la línea de comandos se abren en `connect_activate`; no
    // se pasan como argumentos de GApplication (que exigiría HANDLES_OPEN).
    app.run_with_args::<&str>(&["minmark"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISSUE-026 criterio 3: los cambios generados por nuestras operaciones
    /// §14.8 no avisan como cambio externo; una entrada por evento.
    #[test]
    fn self_ops_consume_los_cambios_propios() {
        let ops = SelfOps::default();
        ops.register(Path::new("/p/cap"));
        ops.register(Path::new("/p/doc.md"));

        assert!(ops.consume(Path::new("/p/doc.md")), "el propio archivo");
        assert!(
            !ops.consume(Path::new("/p/doc.md")),
            "una sola entrada por registro"
        );
        assert!(
            ops.consume(Path::new("/p/cap/a.md")),
            "los eventos dentro de una carpeta propia también"
        );
        assert!(
            !ops.consume(Path::new("/p/ajeno.md")),
            "lo externo sigue avisando"
        );
    }

    /// §9.1 (petición de usuario): la barra vertical del split editor|preview
    /// está en el CSS de ambos temas con su color y grosor de 4px.
    #[test]
    fn css_del_split_editor_preview_en_ambos_temas() {
        let light = theme_css(&Palette::LIGHT, ResolvedTheme::Light);
        let dark = theme_css(&Palette::DARK, ResolvedTheme::Dark);
        for (css, color) in [(&light, "#767676"), (&dark, "#050505")] {
            assert!(
                css.contains("paned.mm-split > separator"),
                "regla del separador del split"
            );
            assert!(css.contains("min-width: 4px"), "barra de 4px");
            assert!(css.contains(color), "color {color} del tema");
        }
        // Solo el tema claro es gris y solo el oscuro, negro.
        assert!(light.contains("background: #767676"));
        assert!(dark.contains("background: #050505"));
    }

    /// §41 (ISSUE-029): el MRU de Quick Open deja lo más reciente primero,
    /// sin duplicados y acotado a los 8 que bonifica el ranking.
    #[test]
    fn mru_quickopen_reciente_primero_sin_duplicados_y_techo() {
        let mut recent = Vec::new();
        mru_touch(&mut recent, "a.md");
        mru_touch(&mut recent, "docs/b.md");
        mru_touch(&mut recent, "a.md");
        assert_eq!(
            recent,
            ["a.md", "docs/b.md"],
            "lo reciente al frente y sin duplicados"
        );
        for i in 0..MRU_MAX + 2 {
            mru_touch(&mut recent, &format!("f{i}.md"));
        }
        assert_eq!(recent.len(), MRU_MAX, "techo del MRU §41");
        assert_eq!(recent[0], format!("f{}.md", MRU_MAX + 1));
        assert!(
            !recent.iter().any(|opened| opened == "a.md"),
            "lo viejo cae con el techo"
        );
    }

    /// §14.4/§14.5/§14.6 (ISSUE-029): la lista standalone de Quick Open son
    /// los tipos documentales visibles del subárbol, sin ocultos ni
    /// ignorados, en orden estable por `rel_path` §17.
    #[test]
    fn lista_standalone_de_quickopen_respeta_14_4_14_6() {
        let base = std::env::temp_dir().join(format!("minmark-qo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("carpeta")).unwrap();
        std::fs::create_dir_all(base.join(".oculta")).unwrap();
        std::fs::write(base.join("a.md"), "# a\n").unwrap();
        std::fs::write(base.join("carpeta/b.md"), "# b\n").unwrap();
        std::fs::write(base.join("imagen.png"), "").unwrap();
        std::fs::write(base.join("guion.txt"), "").unwrap(); // §14.4: fuera
        std::fs::write(base.join("ignorado.md"), "").unwrap(); // §14.6
        std::fs::write(base.join(".oculto.md"), "").unwrap(); // §14.5
        std::fs::write(base.join(".oculta/c.md"), "").unwrap(); // §14.5
        std::fs::write(base.join(".gitignore"), "ignorado.md\n").unwrap();

        let entries = quickopen_entries(&base);
        let rel_paths: Vec<&str> = entries
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        assert_eq!(rel_paths, ["a.md", "carpeta/b.md", "imagen.png"]);
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod gtk_integration_tests {
    use super::*;
    fn pump_until(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "timeout GTK");
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn settle_preview(cmds: &AppCmds) {
        pump_until(|| {
            !cmds.intelligence_busy.get()
                && !cmds.intelligence_pending.get()
                && !cmds.refresh_scheduled.get()
                && !cmds.preview.test_is_loading()
        });
    }
    fn preview_number(cmds: &AppCmds, script: &str) -> f64 {
        settle_preview(cmds);
        let result = Rc::new(Cell::new(None));
        cmds.preview.test_eval_number(script, result.clone());
        pump_until(|| result.get().is_some());
        result.get().unwrap()
    }
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_navigation_outline_completion_and_dirty_buffers() {
        gtk4::init().expect("GTK display");
        ensure_webkit_sandbox_usable();
        let root = std::env::temp_dir().join(format!("minmark-gtk-phase4-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.md");
        let b = root.join("b.md");
        std::fs::write(&a, "# Origen\n\n[ir](b.md#nuevo)\n").unwrap();
        std::fs::write(&b, "# Anterior\n").unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.phase4")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            &[a.clone(), b.clone()],
        );
        pump_until(|| cmds.index.borrow().is_some());
        cmds.tabs.borrow_mut().replace_active(
            Range::new(Position::new(0, 0), Position::new(0, 10)),
            "# Nuevo",
        );
        cmds.select_tab(1);
        let before = cmds.tabs.borrow().len();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 2), Position::new(2, 2)));
        assert!(cmds.editor_pane.emit_test_key(
            gtk4::gdk::Key::Return,
            gtk4::gdk::ModifierType::CONTROL_MASK
        ));
        pump_until(|| cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        assert_eq!(cmds.tabs.borrow().len(), before);
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .starts_with("# Nuevo"));
        assert!(cmds.tabs.borrow().active().editor.document().is_modified());
        cmds.select_tab(1);
        cmds.toggle_mode();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 2), Position::new(2, 2)));
        assert!(cmds.editor_pane.emit_test_key(
            gtk4::gdk::Key::Return,
            gtk4::gdk::ModifierType::CONTROL_MASK
        ));
        pump_until(|| cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        cmds.select_tab(1);
        settle_preview(&cmds);
        let clicked = Rc::new(Cell::new(false));
        cmds.preview.test_click_first_link(clicked.clone());
        pump_until(|| clicked.get() && cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        cmds.show_navigation(false);
        assert!(cmds.navigation_list.is_visible());
        cmds.navigation_list.hide();
        cmds.select_tab(1);
        cmds.tabs.borrow_mut().replace_active(
            Range::new(Position::new(2, 0), Position::new(2, 16)),
            "[ir](b",
        );
        cmds.reload_active();
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        let unchanged = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            unchanged
        );
        // Una pulsación nueva vuelve a habilitar sugerencias tras Escape.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(2, 6), ".");
        cmds.reload_active();
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Tab, gtk4::gdk::ModifierType::empty()));
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("[ir](b.md"));
        cmds.tabs.borrow_mut().undo_active();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("[ir](b.\n"));
        // Mantener la posición de lectura durante ediciones sucesivas,
        // sin iniciar una nueva carga de WebKit ni perder el contenido.
        let long_source = (0..100)
            .map(|n| format!("Párrafo {n}.\n\n"))
            .collect::<String>();
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), &long_source);
        cmds.reload_active();
        settle_preview(&cmds);
        let scroll = Rc::new(Cell::new(None));
        cmds.preview
            .test_eval_number("window.scrollTo(0, 900); window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        let before_scroll = scroll.get().unwrap();
        assert!(
            before_scroll > 500.0,
            "Preview must be scrolled for this regression"
        );
        for _ in 0..3 {
            cmds.tabs
                .borrow_mut()
                .insert_active(Position::new(0, 0), "x");
            cmds.refresh_now();
            assert!(
                !cmds.preview.test_is_loading(),
                "editing must not reload Preview"
            );
        }
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview.test_eval_number(
            "document.body.textContent.includes('xxxPárrafo') ? window.scrollY : -1",
            scroll.clone(),
        );
        pump_until(|| scroll.get().is_some());
        assert!((scroll.get().unwrap() - before_scroll).abs() < 2.0);
        // Navegar explícitamente sigue prevaleciendo sobre la posición previa.
        cmds.focus_line(1);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!(scroll.get().unwrap() < 50.0);
        // Una edición real sigue el cursor hasta un bloque fuera de pantalla.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(160, 0), "editando ");
        cmds.tabs
            .borrow_mut()
            .set_selection_active(Selection::collapsed(Position::new(160, 9)));
        cmds.schedule_refresh();
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview.test_eval_number(
            "(() => { const e = [...document.querySelectorAll('p')].find(e => e.textContent.startsWith('editando Párrafo 80')); if (!e) return -1; const r = e.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight ? scrollY : -1; })()",
            scroll.clone(),
        );
        pump_until(|| scroll.get().is_some());
        assert!(
            scroll.get().unwrap() > 900.0,
            "Preview must follow the edited block"
        );
        let edit_scroll = scroll.get().unwrap();
        // Seguir escribiendo en un bloque visible no produce saltos.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(160, 9), "x");
        cmds.tabs
            .borrow_mut()
            .set_selection_active(Selection::collapsed(Position::new(160, 10)));
        cmds.schedule_refresh();
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!((scroll.get().unwrap() - edit_scroll).abs() < 2.0);
        // Otro documento no hereda el scroll de este.
        cmds.select_tab(2);
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!(scroll.get().unwrap() < 50.0);
        // Flechas más allá de las filas visibles: Quick Open y las listas
        // de documento deben desplazar la selección en ambos sentidos.
        for document_list in [false, true] {
            let palette = QuickOpen::new();
            palette.set_transient_for(&cmds.window.clone().upcast());
            if document_list {
                palette.set_title("Backlinks");
            }
            palette.set_entries(
                (0..1000)
                    .map(|n| minmark_fs::fuzzy::Entry {
                        label: format!("Nota {n:04}"),
                        rel_path: format!("nota-{n:04}.md"),
                        line: None,
                        heading: None,
                        score: 0,
                    })
                    .collect(),
            );
            palette.present();
            pump_until(|| palette.test_selected_visible(0));
            for _ in 0..30 {
                palette.test_key(gtk4::gdk::Key::Down);
            }
            pump_until(|| palette.test_selected_visible(30));
            for _ in 0..30 {
                palette.test_key(gtk4::gdk::Key::Up);
            }
            pump_until(|| palette.test_selected_visible(0));
            palette.hide();
        }
        // El popup de autocompletado también conserva visible su selección
        // mientras el teclado permanece en el editor.
        cmds.editor_pane.show_completions(
            (0..50)
                .map(|n| intelligence::Completion {
                    span: minmark_document::Span { start: 0, end: 0 },
                    text: format!("nota-{n:04}.md"),
                    label: format!("Nota {n:04}"),
                })
                .collect(),
        );
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(0));
        for _ in 0..30 {
            assert!(cmds
                .editor_pane
                .emit_test_key(gtk4::gdk::Key::Down, gtk4::gdk::ModifierType::empty()));
        }
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(30));
        for _ in 0..30 {
            assert!(cmds
                .editor_pane
                .emit_test_key(gtk4::gdk::Key::Up, gtk4::gdk::ModifierType::empty()));
        }
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(0));
        cmds.editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty());
        // Backlinks reales de 1000 documentos: ni la consulta ni la lista
        // deben heredar el límite de 50 sugerencias del autocompletado.
        for n in 0..1000 {
            std::fs::write(root.join(format!("nota-{n:04}.md")), "[Destino](b.md)\n").unwrap();
        }
        spawn_index_refresh(root.clone(), cmds.slot.clone());
        pump_until(|| {
            cmds.index
                .borrow()
                .as_ref()
                .is_some_and(|index| index.len() >= 1002)
        });
        cmds.show_navigation(true);
        pump_until(|| cmds.navigation_positions.borrow().len() == 1000);
        assert_eq!(cmds.navigation_list.test_result_count(), 1000);
        pump_until(|| cmds.navigation_list.test_selected_visible(0));
        for _ in 0..75 {
            cmds.navigation_list.test_key(gtk4::gdk::Key::Down);
        }
        pump_until(|| cmds.navigation_list.test_selected_visible(75));
        cmds.navigation_list.test_filter("nota-0999");
        assert_eq!(cmds.navigation_list.test_result_count(), 1);
        cmds.navigation_list.test_activate();
        assert_eq!(
            cmds.tabs.borrow().active().path.as_ref(),
            Some(&root.join("nota-0999.md"))
        );
        cmds.window.destroy();
        academic_workflow(&app, &root.join("academic"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Evalúa `script` en la WebView y devuelve el número observado.
    fn eval_js(pane: &PreviewPane, script: &str) -> f64 {
        let result = Rc::new(Cell::new(None));
        pane.test_eval_number(script, result.clone());
        pump_until(|| result.get().is_some());
        result.get().unwrap()
    }

    /// Repite la evaluación hasta observar `esperado`; con plazo, para que un
    /// fallo real acabe en timeout descriptivo y no en un bucle infinito.
    fn espera_js(pane: &PreviewPane, script: &str, esperado: f64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            let valor = eval_js(pane, script);
            if valor == esperado {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timeout GTK esperando {esperado} de: {script} (último {valor})"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn nombres_de(cmds: &AppCmds) -> Vec<String> {
        cmds.compositions
            .borrow()
            .iter()
            .map(|comp| comp.name.clone())
            .collect()
    }

    /// Estado `enabled` de las acciones compositivas del grupo «win»
    /// instalado en la ventana (B1): acción a acción — probar «al menos una»
    /// no acredita que las dos sigan a la lista. Pasa por la referencia
    /// débil de `AppCmds`, que es la vía real de [`AppCmds::refresh_compositions`].
    fn acciones_compositivas(cmds: &AppCmds) -> [bool; 2] {
        let grupo = cmds
            .action_group
            .borrow()
            .as_ref()
            .and_then(|grupo| grupo.upgrade())
            .expect("grupo «win» vivo mientras la ventana existe (retención débil)");
        ["preview-composition", "export-composition-pdf"].map(|id| {
            grupo
                .lookup_action(id)
                .unwrap_or_else(|| panic!("falta la acción {id}"))
                .is_enabled()
        })
    }

    /// B1 (revisión PR #44, #22): el watcher redescubre las composiciones
    /// cuando cambia `.mdproject.toml` en caliente — la ventana no se queda
    /// congelada con la lista de apertura — y `compositions_available`
    /// (origen del `enabled` de las acciones §20) queda acorde: la nueva
    /// composición aparece, quitarlas deshabilita y volver a declararlas
    /// habilita.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn composiciones_se_redescubren_al_cambiar_mdproject_en_caliente() {
        gtk4::init().expect("GTK display");
        ensure_webkit_sandbox_usable();
        let root =
            std::env::temp_dir().join(format!("minmark-gtk-redescubre-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("uno.md"), "# Uno\n").unwrap();
        std::fs::write(root.join("dos.md"), "# Dos\n").unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.redescubre")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let uno = root.join("uno.md");
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            std::slice::from_ref(&uno),
        );
        assert_eq!(nombres_de(&cmds), ["A"], "descubrimiento al abrir");
        assert!(cmds.compositions_available());
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "instalación: las dos habilitadas (§20)"
        );

        // Añade una composición en caliente: la ventana la redescubre.
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n[[composition]]\nname = \"B\"\nfiles = [\"dos.md\"]\n",
        )
        .unwrap();
        pump_until(|| nombres_de(&cmds).iter().any(|n| n == "B"));
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "siguen habilitadas con la lista ampliada"
        );

        // Quita todas: `compositions_available` habilita/deshabilita acorde.
        std::fs::write(root.join(".mdproject.toml"), "pagination = false\n").unwrap();
        pump_until(|| !cmds.compositions_available());
        assert_eq!(
            acciones_compositivas(&cmds),
            [false, false],
            "B1: al vaciar la lista, las DOS acciones deshabilitadas"
        );

        // Y al volver a declararlas se habilitan de nuevo.
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        pump_until(|| cmds.compositions_available());
        assert_eq!(nombres_de(&cmds), ["A"]);
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "B1: al volver la lista, las DOS acciones habilitadas"
        );

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(root);
    }

    /// S1/ISSUE-024 (revisión PR #44, aceptación de #24): en la ventana del
    /// Preview combinado, pulsar un enlace `#fragmento` (incluidas las
    /// referencias cruzadas resueltas por `compose`) hace scroll in-page
    /// hasta su elemento (§26) sin salir del documento.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn preview_combinado_navega_destinos_internos_con_scroll_in_page() {
        gtk4::init().expect("GTK display");
        ensure_webkit_sandbox_usable();
        let root = std::env::temp_dir().join(format!(
            "minmark-gtk-destino-interno-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"dos.md\"]\n",
        )
        .unwrap();
        // El destino queda muy por debajo del pliegue del viewport: sin el
        // scroll in-page no puede entrar en pantalla.
        let mut uno = String::from("[Ir al destino](#destino)\n\n");
        for n in 0..150 {
            uno.push_str(&format!("Párrafo {n}.\n\n"));
        }
        std::fs::write(root.join("uno.md"), uno).unwrap();
        std::fs::write(root.join("dos.md"), "## Destino {#destino}\n").unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.destino-interno")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let uno_path = root.join("uno.md");
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            std::slice::from_ref(&uno_path),
        );
        cmds.preview_composition();
        let pane = {
            let preview = cmds.composition_preview.borrow();
            Rc::clone(
                &preview
                    .as_ref()
                    .expect("ventana del Preview combinado")
                    .pane,
            )
        };
        const EN_VISIBLE: &str = "(() => { const e = document.getElementById('destino'); if (!e) return -1; const r = e.getBoundingClientRect(); return (r.top >= 0 && r.top < innerHeight) ? 1 : 0; })()";
        // El HTML combinado llega con el enlace y con su ancla.
        espera_js(
            &pane,
            "document.querySelector('a[href=\\\"#destino\\\"]') ? 1 : 0",
            1.0,
        );
        assert_eq!(
            eval_js(&pane, "document.getElementById('destino') ? 1 : 0"),
            1.0
        );
        // Antes de pulsar, el destino está fuera del viewport.
        assert_eq!(
            eval_js(&pane, EN_VISIBLE),
            0.0,
            "el destino ya era visible: el test no probaría nada"
        );
        // Pulsar el enlace interno desplaza hasta el elemento.
        assert_eq!(
            eval_js(
                &pane,
                "(() => { const a = document.querySelector('a[href=\\\"#destino\\\"]'); if (!a) return 0; a.click(); return 1; })()"
            ),
            1.0
        );
        espera_js(&pane, EN_VISIBLE, 1.0);

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(root);
    }

    fn academic_workflow(app: &Application, root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        let paper = root.join("academic.md");
        let source = include_str!("../../docs/qa/fixtures/academic.md");
        std::fs::write(&paper, source).unwrap();
        std::fs::write(
            root.join("references.bib"),
            include_str!("../../docs/qa/fixtures/references.bib"),
        )
        .unwrap();
        std::fs::write(
            root.join("extra.bib"),
            include_str!("../../docs/qa/fixtures/extra.bib"),
        )
        .unwrap();
        std::fs::write(
            root.join("author-year.csl"),
            include_str!("../../docs/qa/fixtures/author-year.csl"),
        )
        .unwrap();
        let cmds = build_window(
            app,
            WindowScope::Project(root.to_owned()),
            std::slice::from_ref(&paper),
        );
        assert_eq!(
            preview_number(&cmds, "document.querySelectorAll('[role=math] svg').length"),
            5.0
        );
        assert_eq!(preview_number(&cmds, "(() => { const ids = [...document.querySelectorAll('svg [id]')].map(e => e.id); return new Set(ids).size === ids.length ? 1 : 0; })()"), 1.0);
        assert_eq!(preview_number(&cmds, "[...document.querySelectorAll('[role=math] svg')].every(e => { const b = e.getBBox(), v = e.viewBox.baseVal; return b.y >= -2 && b.y + b.height <= v.height + 2; }) ? 1 : 0"), 1.0);
        assert_eq!(preview_number(&cmds, "(() => { document.documentElement.dataset.theme = 'dark'; const color = getComputedStyle(document.body).color; return [...document.querySelectorAll('[role=math] svg use')].every(e => getComputedStyle(e).fill === color) && [...document.querySelectorAll('[role=math] svg [stroke=\"#000000\"]')].every(e => getComputedStyle(e).stroke === color) ? 1 : 0; })()"), 1.0);
        preview_number(&cmds, "document.documentElement.dataset.theme = 'light'; 1");
        assert_eq!(
            preview_number(
                &cmds,
                "document.querySelectorAll('.mm-footnotes li').length"
            ),
            2.0
        );
        assert_eq!(preview_number(&cmds, "(() => { document.querySelector('[role=doc-noteref]').click(); const r = document.getElementById('mm-note-1').getBoundingClientRect(); return r.top >= 0 && r.top < innerHeight ? 1 : 0; })()"), 1.0);
        assert_eq!(preview_number(&cmds, "(() => { document.querySelector('.mm-footnotes a').click(); const r = document.getElementById('mm-note-ref-1-1').getBoundingClientRect(); return r.top >= 0 && r.top < innerHeight ? 1 : 0; })()"), 1.0);
        assert_eq!(
            preview_number(&cmds, "document.querySelectorAll('.mm-toc a').length"),
            4.0
        );
        assert_eq!(preview_number(&cmds, "document.body.textContent.includes('Ecuación 2.1') && document.body.textContent.includes('Energy Research') ? 1 : 0"), 1.0);
        assert!(!cmds.tabs.borrow().active().editor.document().is_modified());
        let evidence = std::env::var_os("MINMARK_QA_OUTPUT_DIR").map(PathBuf::from);
        if let Some(evidence) = &evidence {
            std::fs::create_dir_all(evidence).unwrap();
            let done = Rc::new(Cell::new(false));
            cmds.preview
                .test_snapshot(&evidence.join("academic-continuous.png"), done.clone());
            pump_until(|| done.get());
            let settings = cmds.settings.borrow().clone();
            let pdf = export::export_document_at(
                source,
                &settings,
                Some(&paper),
                Some(root),
                &minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(source)),
            )
            .unwrap();
            std::fs::write(evidence.join("academic.pdf"), pdf).unwrap();
            std::fs::write(
                evidence.join("academic.html"),
                preview::render_document(source, &settings),
            )
            .unwrap();
        }
        // Cambiar un .bib actualiza Preview sin editar ni marcar dirty el Markdown.
        let changed = include_str!("../../docs/qa/fixtures/references.bib")
            .replace("Energy Research", "Energy Research Updated");
        std::fs::write(root.join("references.bib"), changed).unwrap();
        pump_until(|| cmds.refresh_scheduled.get() || cmds.intelligence_busy.get());
        assert_eq!(
            preview_number(
                &cmds,
                "document.body.textContent.includes('Energy Research Updated') ? 1 : 0"
            ),
            1.0
        );
        assert!(!cmds.tabs.borrow().active().editor.document().is_modified());
        // Teclado real del popup: aceptar, cancelar y un único undo/redo.
        let completion_source = "---\nbibliography: references.bib\n---\nβ [@Sm, p. 42]\r\n";
        let original_len = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .len();
        let end = cmds.tabs.borrow().active().editor.position_of(original_len);
        cmds.tabs
            .borrow_mut()
            .replace_active(Range::new(Position::new(0, 0), end), completion_source);
        cmds.reload_active();
        let cursor = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .position_of(completion_source.find(", p.").unwrap());
        cmds.editor_pane
            .focus_find_match(Range::new(cursor, cursor));
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Tab, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source.replace("@Sm", "@smith2025")
        );
        cmds.undo();
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source
        );
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source
        );
        cmds.redo();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("@smith2025, p. 42"));
        cmds.toggle_mode();
        cmds.toggle_mode();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .ends_with("\r\n"));
        // CSL alternativo, notas y TOC comparten la composición paginada/PDF.
        let paginated = source.replace(
            "language: es",
            "language: es\npagination: true\nbibliography_style: author-year.csl",
        );
        let len = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .len();
        let end = cmds.tabs.borrow().active().editor.position_of(len);
        cmds.tabs
            .borrow_mut()
            .replace_active(Range::new(Position::new(0, 0), end), &paginated);
        cmds.reload_active();
        assert!(preview_number(&cmds, "document.querySelectorAll('.mm-paged-page').length") >= 2.0);
        if let Some(evidence) = &evidence {
            let done = Rc::new(Cell::new(false));
            cmds.preview
                .test_snapshot(&evidence.join("academic-paginated.png"), done.clone());
            pump_until(|| done.get());
            let pdf = export::export_document_at(
                &paginated,
                &cmds.settings.borrow(),
                Some(&paper),
                Some(root),
                &minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(&paginated)),
            )
            .unwrap();
            std::fs::write(evidence.join("academic-author-year.pdf"), pdf).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&paper).unwrap(), source);
        cmds.window.destroy();
    }
}
