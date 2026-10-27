//! Comandos, barra de menú §36 y atajos §39.
//!
//! El modelo de menú vive en datos puros ([`MENUBAR_SPEC`]) para poder
//! verificar la estructura §36 sin display ni main loop de GTK;
//! [`build_menubar`] lo materializa en `gio::Menu` (GMenu es GObject puro de
//! GIO y no necesita `gtk_init`). [`install_actions`] registra las acciones
//! `win.*` (deshabilitando las de composición sin proyecto, ISSUE-022) y
//! fija los aceleradores §39. [`ALL_COMMANDS`] es la lista completa §38,
//! pública para que la Command Palette se alimente de ella.

use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use crate::preview::ViewMode;

/// Comandos de la aplicación (§36: File, Edit, View, Insert, Format,
/// Project, Help; Insert/Format con la fase 7, #29 e Image/Table con #30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    New,
    /// Nueva ventana standalone (ISSUE-021): archivo nuevo fuera de proyecto.
    NewWindow,
    Open,
    /// Quick Open (§41, §39: `Ctrl+P`): paleta fuzzy de archivos del proyecto.
    QuickOpen,
    Outline,
    FollowLink,
    Backlinks,
    /// Abrir carpeta como ventana de proyecto nueva (ISSUE-021).
    OpenFolder,
    /// Import Markdown (§15, ISSUE-024): copia un `.md` externo al proyecto.
    ImportMarkdown,
    Save,
    SaveAs,
    /// Exporta el documento activo a PDF (§32.1, ISSUE-035, §39: `Ctrl+E`).
    ExportPdf,
    /// Exporta la composición activa como un único PDF (§20.4, ISSUE-023).
    ExportCompositionPdf,
    CloseTab,
    Undo,
    Redo,
    /// Búsqueda en el archivo actual (§40.1, §39: `Ctrl+F`).
    Find,
    /// Reemplazo en el archivo actual (§40.1, §39: `Ctrl+H`).
    Replace,
    /// Búsqueda en el proyecto (§40.2, §39: `Ctrl+Shift+F`).
    ProjectSearch,
    ToggleMode,
    /// Visibilidad de la sintaxis en Visual (§8.3, ISSUE-030): ocultar
    /// siempre / revelar la del elemento editado.
    VisualSyntax,
    /// Muestra/oculta el explorador de archivos (§14.3, §39: `Ctrl+B`).
    ToggleExplorer,
    /// Command Palette §38 (§39: `Ctrl+Shift+P`), menú View.
    CommandPalette,
    /// Cambio de modo de vista (§36, menú View).
    View(ViewMode), // ViewMode de crate::preview
    /// Enlace `[texto](url)` (fase 7 #29, menú Insert, `Ctrl+K`): diálogo de
    /// URL prellenado con la del enlace bajo el cursor si lo hay.
    InsertLink,
    /// Callout `::: tipo` (§22, fase 7 #29, menú Insert).
    InsertCallout,
    /// Imagen `![alt](ruta)` (fase 7 #30, §18, menú Insert): selector de
    /// fichero (§18.1) + formulario de campos; copia a `<raíz>/assets/` con
    /// renombrado §18.3 e inserción con ruta relativa §17.
    InsertImage,
    /// Tabla GFM (fase 7 #30, §35, menú Insert): formulario filas/columnas
    /// (1–50 × 1–8) → `FormatAction::TableInsert`.
    InsertTable,
    /// Editar la imagen bajo el cursor/selección (fase 7 #30, §18/§26.2;
    /// popover y Command Palette): diálogo con Ruta, Alt, ID y Ancho %
    /// prellenados desde `format::context` → `FormatAction::ImageEdit`.
    EditImage,
    /// Editar los atributos `{#id "caption"}` de la tabla (fase 7 #30,
    /// §26.3; popover y Command Palette): diálogo ID/Caption prellenado →
    /// `FormatAction::TableAttrs`.
    EditTableAttrs,
    /// Heading ATX 1–6 (fase 7 #29, menú Format, `Ctrl+1..6`).
    FormatHeading1,
    FormatHeading2,
    FormatHeading3,
    FormatHeading4,
    FormatHeading5,
    FormatHeading6,
    /// Negrita `**texto**` (fase 7 #29, menú Format, `Ctrl+Shift+B`).
    FormatStrong,
    /// Cursiva `*texto*` (fase 7 #29, menú Format, `Ctrl+I`).
    FormatEmphasis,
    /// Tachado `~~texto~~` (fase 7 #29, menú Format, `Ctrl+Shift+X`).
    FormatStrikethrough,
    /// Código inline `` `texto` `` (fase 7 #29, menú Format, `Ctrl+Shift+C`).
    FormatCode,
    /// Preview combinado de una composición (§20.4, ISSUE-023, menú Project).
    PreviewComposition,
    About,
    Shortcuts,
}

impl Command {
    /// Identificador de acción GTK («win.…») con el que se registra el
    /// comando y al que apuntan menús y aceleradores (§36).
    pub fn action_id(&self) -> &'static str {
        match self {
            Command::New => "win.new",
            Command::NewWindow => "win.new-window",
            Command::Open => "win.open",
            Command::QuickOpen => "win.quick-open",
            Command::Outline => "win.outline",
            Command::FollowLink => "win.follow-link",
            Command::Backlinks => "win.backlinks",
            Command::OpenFolder => "win.open-folder",
            Command::ImportMarkdown => "win.import-markdown",
            Command::Save => "win.save",
            Command::SaveAs => "win.save-as",
            Command::ExportPdf => "win.export-pdf",
            Command::ExportCompositionPdf => "win.export-composition-pdf",
            Command::CloseTab => "win.close-tab",
            Command::Undo => "win.undo",
            Command::Redo => "win.redo",
            Command::Find => "win.find",
            Command::Replace => "win.replace",
            Command::ProjectSearch => "win.project-search",
            Command::ToggleMode => "win.toggle-mode",
            Command::VisualSyntax => "win.visual-syntax",
            Command::ToggleExplorer => "win.toggle-explorer",
            Command::CommandPalette => "win.command-palette",
            Command::View(ViewMode::Editor) => "win.view-editor",
            Command::View(ViewMode::Split) => "win.view-split",
            Command::View(ViewMode::Preview) => "win.view-preview",
            Command::InsertLink => "win.insert-link",
            Command::InsertCallout => "win.insert-callout",
            Command::InsertImage => "win.insert-image",
            Command::InsertTable => "win.insert-table",
            Command::EditImage => "win.edit-image",
            Command::EditTableAttrs => "win.edit-table-attrs",
            Command::FormatHeading1 => "win.format-heading1",
            Command::FormatHeading2 => "win.format-heading2",
            Command::FormatHeading3 => "win.format-heading3",
            Command::FormatHeading4 => "win.format-heading4",
            Command::FormatHeading5 => "win.format-heading5",
            Command::FormatHeading6 => "win.format-heading6",
            Command::FormatStrong => "win.format-strong",
            Command::FormatEmphasis => "win.format-emphasis",
            Command::FormatStrikethrough => "win.format-strikethrough",
            Command::FormatCode => "win.format-code",
            Command::PreviewComposition => "win.preview-composition",
            Command::About => "win.about",
            Command::Shortcuts => "win.shortcuts",
        }
    }

    /// Etiqueta de menú en inglés (§36: File, Edit, …). El ítem de atajos
    /// conserva «Atajos», el nombre con el que §36 lo lista en Help. §38:
    /// es también la etiqueta que filtra la Command Palette.
    pub fn label(&self) -> &'static str {
        match self {
            Command::New => "New",
            Command::NewWindow => "New Window",
            Command::Open => "Open",
            Command::QuickOpen => "Quick Open",
            Command::Outline => "Outline",
            Command::FollowLink => "Open Link",
            Command::Backlinks => "Backlinks",
            Command::OpenFolder => "Open Folder",
            Command::ImportMarkdown => "Import Markdown",
            Command::Save => "Save",
            Command::SaveAs => "Save As",
            Command::ExportPdf => "Export PDF",
            Command::ExportCompositionPdf => "Export Composition PDF",
            Command::CloseTab => "Close Tab",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::Find => "Find",
            Command::Replace => "Replace",
            Command::ProjectSearch => "Project Search",
            Command::ToggleMode => "Markdown/Visual",
            Command::VisualSyntax => "Toggle Visual Syntax",
            Command::ToggleExplorer => "Toggle Explorer",
            Command::CommandPalette => "Command Palette",
            Command::View(mode) => mode.label(),
            Command::InsertLink => "Link",
            Command::InsertCallout => "Callout",
            Command::InsertImage => "Image",
            Command::InsertTable => "Table",
            Command::EditImage => "Edit Image",
            Command::EditTableAttrs => "Table Attributes",
            Command::FormatHeading1 => "Heading 1",
            Command::FormatHeading2 => "Heading 2",
            Command::FormatHeading3 => "Heading 3",
            Command::FormatHeading4 => "Heading 4",
            Command::FormatHeading5 => "Heading 5",
            Command::FormatHeading6 => "Heading 6",
            Command::FormatStrong => "Bold",
            Command::FormatEmphasis => "Italic",
            Command::FormatStrikethrough => "Strikethrough",
            Command::FormatCode => "Code",
            Command::PreviewComposition => "Preview Composition",
            Command::About => "About",
            Command::Shortcuts => "Atajos",
        }
    }

    /// Acción que exige un proyecto con composiciones (§20, ISSUE-022): sin
    /// ellas el ítem queda deshabilitado en el menú y en la paleta (§38)
    /// vía [`Commands::compositions_available`].
    pub fn requires_composition(&self) -> bool {
        matches!(
            self,
            Command::PreviewComposition | Command::ExportCompositionPdf
        )
    }
}

/// Receptor de comandos: lo implementa el integrador (main.rs).
pub trait Commands {
    fn new(&self);
    /// Nueva ventana standalone con un archivo nuevo (ISSUE-021).
    fn new_window(&self);
    fn open(&self);
    /// Abre la paleta Quick Open (§41, §39: `Ctrl+P`).
    fn quick_open(&self);
    fn outline(&self) {}
    fn follow_link(&self) {}
    fn backlinks(&self) {}
    /// Abre una carpeta en una ventana de proyecto nueva (ISSUE-021).
    fn open_folder(&self);
    /// Import Markdown (§15): copia un `.md` externo dentro del proyecto.
    fn import_markdown(&self);
    fn save(&self);
    fn save_as(&self);
    /// Exporta el documento activo a PDF (§32.1, ISSUE-035): diálogo de
    /// destino y motor [`minmark_pdf::TypstEngine`].
    fn export_pdf(&self);
    /// Exporta la composición elegida como un único PDF (§20.4, ISSUE-023):
    /// el mismo flujo de diálogo que [`Commands::export_pdf`].
    fn export_composition_pdf(&self) {}
    /// Preview combinado de la composición elegida (§20.4, ISSUE-023) en
    /// una ventana temporal con el patrón Outline/Backlinks.
    fn preview_composition(&self) {}
    /// ¿Hay composiciones que gestionar (§20, ISSUE-022)? Sin proyecto
    /// abierto las acciones de composición quedan deshabilitadas.
    fn compositions_available(&self) -> bool {
        false
    }
    fn close_tab(&self);
    fn undo(&self);
    fn redo(&self);
    /// Abre la barra de búsqueda del archivo actual (§40.1).
    fn find(&self);
    /// Abre la barra de búsqueda con la fila de reemplazo (§40.1).
    fn replace(&self);
    /// Abre la vista de búsqueda en el proyecto de la barra lateral
    /// (§40.2); sin-op en ventanas standalone.
    fn project_search(&self);
    fn toggle_mode(&self);
    /// Alterna la visibilidad de la sintaxis en Visual (§8.3).
    fn toggle_visual_syntax(&self);
    /// Muestra/oculta el explorador (§14.3); sin-op en ventanas standalone.
    fn toggle_explorer(&self);
    fn set_view(&self, mode: ViewMode);
    /// Aplica `win.format-strong` (negrita) sobre la selección o el bloque
    /// del cursor vía [`TabManager::apply_format_active`](crate::tabs::TabManager::apply_format_active)
    /// (fase 7 #29); sin-op por defecto para no romper implementadores.
    fn format_strong(&self) {}
    /// Cursiva `win.format-emphasis` (fase 7 #29).
    fn format_emphasis(&self) {}
    /// Tachado `win.format-strikethrough` (fase 7 #29).
    fn format_strikethrough(&self) {}
    /// Código inline `win.format-code` (fase 7 #29).
    fn format_code(&self) {}
    /// Heading 1 `win.format-heading1` (fase 7 #29).
    fn format_heading1(&self) {}
    /// Heading 2 `win.format-heading2` (fase 7 #29).
    fn format_heading2(&self) {}
    /// Heading 3 `win.format-heading3` (fase 7 #29).
    fn format_heading3(&self) {}
    /// Heading 4 `win.format-heading4` (fase 7 #29).
    fn format_heading4(&self) {}
    /// Heading 5 `win.format-heading5` (fase 7 #29).
    fn format_heading5(&self) {}
    /// Heading 6 `win.format-heading6` (fase 7 #29).
    fn format_heading6(&self) {}
    /// `win.insert-link`: diálogo de URL prellenado con la del enlace bajo
    /// el cursor si lo hay; Cancelar no toca el documento (fase 7 #29).
    fn insert_link(&self) {}
    /// `win.insert-callout`: envuelve o retona el callout con `tone`
    /// (tono inicial `note`; la acción guarda el último tono elegido).
    fn insert_callout(&self, _tone: &str) {}
    /// `win.insert-image` (fase 7 #30): selector de fichero §18.1 +
    /// formulario de campos; copia a `<raíz>/assets/` §18.3 e
    /// `FormatAction::ImageInsert` con ruta relativa §17. Cancelar en
    /// cualquier punto ⇒ cero cambios y NO hay copia.
    fn insert_image(&self) {}
    /// `win.insert-table` (fase 7 #30): diálogo filas/columnas (1–50 × 1–8)
    /// → `FormatAction::TableInsert` sobre la pestaña activa.
    fn insert_table(&self) {}
    /// `win.edit-image` (fase 7 #30): diálogo de imagen prellenado con el
    /// constructo bajo el cursor/selección → `FormatAction::ImageEdit`.
    fn edit_image(&self) {}
    /// `win.edit-table-attrs` (fase 7 #30): diálogo ID/Caption prellenado
    /// con la tabla bajo el cursor/selección → `FormatAction::TableAttrs`.
    fn edit_table_attrs(&self) {}
    /// Command Palette §38 (§39: `Ctrl+Shift+P`).
    fn command_palette(&self) {}
    fn about(&self);
    fn shortcuts(&self);
}

/// Atajos §39 (identificador de acción → acelerador GTK).
pub const ACCELERATORS: &[(&str, &str)] = &[
    ("win.new", "<Control>n"),
    ("win.new-window", "<Control><Shift>n"),
    ("win.open", "<Control>o"),
    ("win.quick-open", "<Control>p"),
    ("win.outline", "<Control><Alt>o"),
    ("win.follow-link", "<Control>Return"),
    ("win.backlinks", "<Control><Alt>b"),
    ("win.open-folder", "<Control><Shift>o"),
    ("win.save", "<Control>s"),
    ("win.save-as", "<Control><Shift>s"),
    ("win.export-pdf", "<Control>e"),
    ("win.close-tab", "<Control>w"),
    ("win.undo", "<Control>z"),
    ("win.redo", "<Control><Shift>z"),
    ("win.find", "<Control>f"),
    ("win.replace", "<Control>h"),
    ("win.project-search", "<Control><Shift>f"),
    ("win.toggle-mode", "<Control><Alt>m"),
    ("win.toggle-explorer", "<Control>b"),
    ("win.command-palette", "<Control><Shift>p"),
    // Controles contextuales §44 (fase 7 #29): sin conflicto con §39
    // (`Ctrl+B` sigue siendo Toggle Explorer, `Ctrl+P` Quick Open).
    ("win.format-strong", "<Control><Shift>b"),
    ("win.format-emphasis", "<Control>i"),
    ("win.format-strikethrough", "<Control><Shift>x"),
    ("win.format-code", "<Control><Shift>c"),
    ("win.format-heading1", "<Control>1"),
    ("win.format-heading2", "<Control>2"),
    ("win.format-heading3", "<Control>3"),
    ("win.format-heading4", "<Control>4"),
    ("win.format-heading5", "<Control>5"),
    ("win.format-heading6", "<Control>6"),
    ("win.insert-link", "<Control>k"),
];

/// Todos los comandos, en orden de aparición en los menús §36. §38: es la
/// lista completa que la Command Palette ejecuta — pública para que la
/// paleta se alimente de ella — y todo comando es alcanzable desde el menú
/// (los dos comandos de constructo #30 llegan por el popover §44 y la
/// paleta; ver `menu_y_comandos_coherentes`).
pub const ALL_COMMANDS: &[Command] = &[
    Command::New,
    Command::NewWindow,
    Command::Open,
    Command::QuickOpen,
    Command::Outline,
    Command::FollowLink,
    Command::Backlinks,
    Command::OpenFolder,
    Command::ImportMarkdown,
    Command::Save,
    Command::SaveAs,
    Command::ExportPdf,
    Command::ExportCompositionPdf,
    Command::CloseTab,
    Command::Undo,
    Command::Redo,
    Command::Find,
    Command::Replace,
    Command::ProjectSearch,
    Command::ToggleMode,
    Command::VisualSyntax,
    Command::ToggleExplorer,
    Command::CommandPalette,
    Command::View(ViewMode::Editor),
    Command::View(ViewMode::Split),
    Command::View(ViewMode::Preview),
    Command::InsertLink,
    Command::InsertCallout,
    Command::InsertImage,
    Command::InsertTable,
    Command::EditImage,
    Command::EditTableAttrs,
    Command::FormatHeading1,
    Command::FormatHeading2,
    Command::FormatHeading3,
    Command::FormatHeading4,
    Command::FormatHeading5,
    Command::FormatHeading6,
    Command::FormatStrong,
    Command::FormatEmphasis,
    Command::FormatStrikethrough,
    Command::FormatCode,
    Command::PreviewComposition,
    Command::About,
    Command::Shortcuts,
];

/// Tono inicial de `win.insert-callout` (fase 7 #29): la acción es
/// stateful con estado de tipo string — sigue siendo activable con
/// `activate_action(id, None)` (menú y paleta §38) y el popover de
/// controles actualiza el estado antes de activarla para retonar.
pub const DEFAULT_CALLOUT_TONE: &str = "note";

/// Ítem puro del modelo de menú §36: `(etiqueta, action_id)`.
type EntrySpec = (&'static str, &'static str);
/// Sección pura del modelo de menú §36: ítems contiguos con separador.
type SectionSpec = &'static [EntrySpec];
/// Submenú puro §36: `(título, secciones)`.
type SubmenuSpec = (&'static str, &'static [SectionSpec]);

/// Modelo de menú §36 en datos puros: File, Edit, View, Insert, Format,
/// Project y Help. View agrupa «Markdown/Visual» (ToggleMode) y la sección
/// radio implícita Editor/Split/Preview (§36) más la Command Palette §38;
/// Insert (Link, Callout, Image, Table) y Format (Heading 1–6, Bold, Italic,
/// Strikethrough, Code) llegan con la fase 7 (#29/#30); Project nace con las
/// composiciones (§20, ISSUE-022).
const MENUBAR_SPEC: &[SubmenuSpec] = &[
    (
        "File",
        &[&[
            ("New", "win.new"),
            ("New Window", "win.new-window"),
            ("Open", "win.open"),
            ("Quick Open", "win.quick-open"),
            ("Open Folder", "win.open-folder"),
            ("Import Markdown", "win.import-markdown"),
            ("Save", "win.save"),
            ("Save As", "win.save-as"),
            ("Export PDF", "win.export-pdf"),
            ("Export Composition PDF", "win.export-composition-pdf"),
            ("Close Tab", "win.close-tab"),
        ]],
    ),
    (
        "Edit",
        &[&[
            ("Undo", "win.undo"),
            ("Redo", "win.redo"),
            ("Find", "win.find"),
            ("Replace", "win.replace"),
            ("Project Search", "win.project-search"),
            ("Open Link", "win.follow-link"),
        ]],
    ),
    (
        "View",
        &[
            &[
                ("Markdown/Visual", "win.toggle-mode"),
                ("Toggle Visual Syntax", "win.visual-syntax"),
                ("Toggle Explorer", "win.toggle-explorer"),
                ("Outline", "win.outline"),
                ("Backlinks", "win.backlinks"),
                ("Command Palette", "win.command-palette"),
            ],
            &[
                ("Editor", "win.view-editor"),
                ("Split", "win.view-split"),
                ("Preview", "win.view-preview"),
            ],
        ],
    ),
    (
        "Insert",
        &[&[
            ("Link", "win.insert-link"),
            ("Callout", "win.insert-callout"),
            // Fase 7 #30 (§UI): Image y Table junto a Link/Callout. Los
            // comandos de edición de constructo (Edit Image, Table
            // Attributes) viven en el popover §44 y la paleta §38, sin
            // ítem de menú.
            ("Image", "win.insert-image"),
            ("Table", "win.insert-table"),
        ]],
    ),
    (
        "Format",
        &[&[
            ("Heading 1", "win.format-heading1"),
            ("Heading 2", "win.format-heading2"),
            ("Heading 3", "win.format-heading3"),
            ("Heading 4", "win.format-heading4"),
            ("Heading 5", "win.format-heading5"),
            ("Heading 6", "win.format-heading6"),
            ("Bold", "win.format-strong"),
            ("Italic", "win.format-emphasis"),
            ("Strikethrough", "win.format-strikethrough"),
            ("Code", "win.format-code"),
        ]],
    ),
    (
        "Project",
        &[&[("Preview Composition", "win.preview-composition")]],
    ),
    (
        "Help",
        &[&[("Atajos", "win.shortcuts"), ("About", "win.about")]],
    ),
];

/// Modelo de menú §36: File, Edit, View, Insert, Format, Project, Help.
pub fn build_menubar() -> gio::Menu {
    let menubar = gio::Menu::new();
    for &(title, sections) in MENUBAR_SPEC {
        let submenu = gio::Menu::new();
        for entries in sections {
            let section = gio::Menu::new();
            for &(label, action_id) in *entries {
                section.append_item(&gio::MenuItem::new(Some(label), Some(action_id)));
            }
            submenu.append_section(None::<&str>, &section);
        }
        menubar.append_submenu(Some(title), &submenu);
    }
    menubar
}

/// Enruta un comando activado al receptor [`Commands`] (§3.7: toda acción de
/// la UI es explícita y observable en el host). `callout_tone` solo lo usa
/// [`Command::InsertCallout`]: el tono vive en el estado de la acción (tono
/// inicial [`DEFAULT_CALLOUT_TONE`], último elegido en el popover después).
fn dispatch(cmd: Command, host: &dyn Commands, callout_tone: &str) {
    match cmd {
        Command::New => host.new(),
        Command::NewWindow => host.new_window(),
        Command::Open => host.open(),
        Command::QuickOpen => host.quick_open(),
        Command::Outline => host.outline(),
        Command::FollowLink => host.follow_link(),
        Command::Backlinks => host.backlinks(),
        Command::OpenFolder => host.open_folder(),
        Command::ImportMarkdown => host.import_markdown(),
        Command::Save => host.save(),
        Command::SaveAs => host.save_as(),
        Command::ExportPdf => host.export_pdf(),
        Command::ExportCompositionPdf => host.export_composition_pdf(),
        Command::CloseTab => host.close_tab(),
        Command::Undo => host.undo(),
        Command::Redo => host.redo(),
        Command::Find => host.find(),
        Command::Replace => host.replace(),
        Command::ProjectSearch => host.project_search(),
        Command::ToggleMode => host.toggle_mode(),
        Command::VisualSyntax => host.toggle_visual_syntax(),
        Command::ToggleExplorer => host.toggle_explorer(),
        Command::CommandPalette => host.command_palette(),
        Command::View(mode) => host.set_view(mode),
        Command::InsertLink => host.insert_link(),
        Command::InsertCallout => host.insert_callout(callout_tone),
        Command::InsertImage => host.insert_image(),
        Command::InsertTable => host.insert_table(),
        Command::EditImage => host.edit_image(),
        Command::EditTableAttrs => host.edit_table_attrs(),
        Command::FormatHeading1 => host.format_heading1(),
        Command::FormatHeading2 => host.format_heading2(),
        Command::FormatHeading3 => host.format_heading3(),
        Command::FormatHeading4 => host.format_heading4(),
        Command::FormatHeading5 => host.format_heading5(),
        Command::FormatHeading6 => host.format_heading6(),
        Command::FormatStrong => host.format_strong(),
        Command::FormatEmphasis => host.format_emphasis(),
        Command::FormatStrikethrough => host.format_strikethrough(),
        Command::FormatCode => host.format_code(),
        Command::PreviewComposition => host.preview_composition(),
        Command::About => host.about(),
        Command::Shortcuts => host.shortcuts(),
    }
}

/// Construye el grupo de acciones `win.*` de la ventana: cada comando se
/// registra sin parámetro en un [`gio::SimpleActionGroup`] insertado con el
/// prefijo «win» (los `action_id` §36 son nombres de acción detallados
/// `win.*`).
///
/// §20/ISSUE-022: sin proyecto con composiciones el ítem queda deshabilitado
/// (también en la paleta §38: la misma acción `win.*`). Aislado de la
/// ventana para poder construirlo y verificarlo sin display (patrón
/// [`build_menubar`]); [`install_actions`] lo inserta en la ventana.
pub fn action_group(host: &Rc<dyn Commands>) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    for &cmd in ALL_COMMANDS {
        let name = cmd
            .action_id()
            .strip_prefix("win.")
            .expect("los action_id usan el prefijo «win.» (§36)");
        // `insert-callout` lleva el tono en su estado (parámetro `None`:
        // la paleta §38 la activa con `activate_action(id, None)`); el
        // popover de controles fija el estado antes de activarla para
        // retonar, y menú/atajos usan el último tono (inicial `note`).
        let action = if cmd == Command::InsertCallout {
            gio::SimpleAction::new_stateful(name, None, &glib::Variant::from(DEFAULT_CALLOUT_TONE))
        } else {
            gio::SimpleAction::new(name, None)
        };
        if cmd.requires_composition() {
            action.set_enabled(host.compositions_available());
        }
        let host = Rc::clone(host);
        action.connect_activate(move |action, _| {
            let tone = action
                .state()
                .and_then(|state| state.str().map(str::to_owned))
                .unwrap_or_else(|| DEFAULT_CALLOUT_TONE.to_owned());
            dispatch(cmd, &*host, &tone);
        });
        group.add_action(&action);
    }
    group
}

/// Reevalúa el estado `enabled` de las acciones que exigen composiciones
/// (§20, ISSUE-022) sobre `group` — el grupo «win» que `AppCmds` conserva
/// del [`install_actions`] — cuando cambia la lista de composiciones de la
/// ventana (B1, revisión PR #44: el watcher redescubre al cambiar
/// `.mdproject.toml` y las acciones no pueden quedarse con el estado de la
/// instalación). El resto de acciones no se toca.
pub fn refresh_composition_actions(group: &gio::SimpleActionGroup, disponible: bool) {
    for cmd in ALL_COMMANDS.iter().filter(|cmd| cmd.requires_composition()) {
        let name = cmd
            .action_id()
            .strip_prefix("win.")
            .expect("los action_id usan el prefijo «win.» (§36)");
        if let Some(action) = group.lookup_action(name) {
            if let Ok(action) = action.downcast::<gio::SimpleAction>() {
                action.set_enabled(disponible);
            }
        }
    }
}

/// Conecta `gio::SimpleAction`s de la ventana con `host` y fija los aceleradores.
///
/// `View(ViewMode)` usa tres acciones simples `win.view-editor|split|preview`
/// (forma simple verificada en gtk4 0.9; la variante con target sobre
/// `glib::Variant` no aporta nada aquí). Los aceleradores §39 se fijan con
/// `gtk::Application::set_accels_for_action`. Devuelve el grupo insertado
/// para que `AppCmds` pueda refrescar el `enabled` de las composiciones
/// (B1) guardándolo en referencia débil: las acciones capturan `host` y una
/// retención fuerte cerraría un ciclo con `AppCmds`.
pub fn install_actions<W: IsA<gtk::Window>>(
    window: &W,
    app: &gtk::Application,
    host: Rc<dyn Commands>,
) -> gio::SimpleActionGroup {
    let group = action_group(&host);
    window
        .upcast_ref::<gtk::Window>()
        .insert_action_group("win", Some(&group));
    for &(action_id, accel) in ACCELERATORS {
        app.set_accels_for_action(action_id, &[accel]);
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;

    /// §39: los atajos exigidos (incluido `Ctrl+P` Quick Open y `Ctrl+Shift+F`
    /// Project Search) están registrados y no hay aceleradores duplicados ni
    /// acciones sin registrar.
    #[test]
    fn aceleradores_39_sin_duplicados() {
        for (id, accel) in [
            ("win.new", "<Control>n"),
            ("win.open", "<Control>o"),
            ("win.quick-open", "<Control>p"),
            ("win.outline", "<Control><Alt>o"),
            ("win.follow-link", "<Control>Return"),
            ("win.backlinks", "<Control><Alt>b"),
            ("win.save", "<Control>s"),
            ("win.save-as", "<Control><Shift>s"),
            ("win.export-pdf", "<Control>e"),
            ("win.find", "<Control>f"),
            ("win.replace", "<Control>h"),
            ("win.project-search", "<Control><Shift>f"),
            ("win.toggle-explorer", "<Control>b"),
            // Fase 7 #29 (§UI): atajos nuevos sin conflicto con §39.
            ("win.command-palette", "<Control><Shift>p"),
            ("win.format-strong", "<Control><Shift>b"),
            ("win.format-emphasis", "<Control>i"),
            ("win.format-strikethrough", "<Control><Shift>x"),
            ("win.format-code", "<Control><Shift>c"),
            ("win.format-heading1", "<Control>1"),
            ("win.format-heading2", "<Control>2"),
            ("win.format-heading3", "<Control>3"),
            ("win.format-heading4", "<Control>4"),
            ("win.format-heading5", "<Control>5"),
            ("win.format-heading6", "<Control>6"),
            ("win.insert-link", "<Control>k"),
        ] {
            assert!(
                ACCELERATORS.contains(&(id, accel)),
                "falta el atajo §39 {id} → {accel}"
            );
        }

        let registrados: HashSet<&str> = ALL_COMMANDS.iter().map(|c| c.action_id()).collect();
        let mut acciones = HashSet::new();
        let mut aceleradores = HashSet::new();
        for &(id, accel) in ACCELERATORS {
            assert!(registrados.contains(id), "acción sin registrar: {id}");
            assert!(
                acciones.insert(id),
                "acción duplicada en ACCELERATORS: {id}"
            );
            assert!(
                aceleradores.insert(accel),
                "acelerador duplicado en ACCELERATORS: {accel}"
            );
        }
    }

    /// §36: cada comando tiene un `action_id` único con el prefijo «win.».
    #[test]
    fn action_ids_unicos_con_prefijo_win() {
        let mut vistos = HashSet::new();
        for &cmd in ALL_COMMANDS {
            let id = cmd.action_id();
            assert!(id.starts_with("win."), "{id} sin prefijo «win.»");
            assert!(vistos.insert(id), "action_id duplicado: {id}");
        }
        assert_eq!(vistos.len(), ALL_COMMANDS.len());
    }

    /// Fase 7 #29: los comandos nuevos tienen el `action_id` y la etiqueta
    /// fijados por el contrato §UI (menús Insert/Format, Command Palette).
    #[test]
    fn comandos_fase_7_29_ids_y_etiquetas() {
        for (cmd, id, label) in [
            (Command::CommandPalette, "win.command-palette", "Command Palette"),
            (Command::InsertLink, "win.insert-link", "Link"),
            (Command::InsertCallout, "win.insert-callout", "Callout"),
            (Command::FormatHeading1, "win.format-heading1", "Heading 1"),
            (Command::FormatHeading2, "win.format-heading2", "Heading 2"),
            (Command::FormatHeading3, "win.format-heading3", "Heading 3"),
            (Command::FormatHeading4, "win.format-heading4", "Heading 4"),
            (Command::FormatHeading5, "win.format-heading5", "Heading 5"),
            (Command::FormatHeading6, "win.format-heading6", "Heading 6"),
            (Command::FormatStrong, "win.format-strong", "Bold"),
            (Command::FormatEmphasis, "win.format-emphasis", "Italic"),
            (
                Command::FormatStrikethrough,
                "win.format-strikethrough",
                "Strikethrough",
            ),
            (Command::FormatCode, "win.format-code", "Code"),
        ] {
            assert_eq!(cmd.action_id(), id, "action_id de {label}");
            assert_eq!(cmd.label(), label, "etiqueta de {id}");
            assert!(
                ALL_COMMANDS.contains(&cmd),
                "{id} ausente de ALL_COMMANDS (§38)"
            );
        }
    }

    /// Fase 7 #30: los comandos nuevos tienen el `action_id` y la etiqueta
    /// fijados por el contrato §UI (Insert ▸ Image/Table, popover §44 y
    /// Command Palette §38) y NO registran aceleradores nuevos (el
    /// contrato §UI exige documentar que no los hay).
    #[test]
    fn comandos_fase_7_30_ids_y_etiquetas_sin_atajos() {
        let nuevos = [
            (Command::InsertImage, "win.insert-image", "Image"),
            (Command::InsertTable, "win.insert-table", "Table"),
            (Command::EditImage, "win.edit-image", "Edit Image"),
            (
                Command::EditTableAttrs,
                "win.edit-table-attrs",
                "Table Attributes",
            ),
        ];
        for (cmd, id, label) in nuevos {
            assert_eq!(cmd.action_id(), id, "action_id de {label}");
            assert_eq!(cmd.label(), label, "etiqueta de {id}");
            assert!(
                ALL_COMMANDS.contains(&cmd),
                "{id} ausente de ALL_COMMANDS (§38)"
            );
            assert!(!cmd.requires_composition(), "{id} sin composiciones (§20)");
            assert!(
                !ACCELERATORS.iter().any(|(accel_id, _)| *accel_id == id),
                "{id} no debe tener acelerador (#30 añade los comandos sin atajos)"
            );
        }
        // Imagen y Table están en el menú Insert junto a Link/Callout (§UI);
        // los dos de edición no llevan ítem de menú (popover + paleta).
        for id in ["win.insert-image", "win.insert-table"] {
            assert!(
                ALL_COMMANDS.iter().any(|cmd| cmd.action_id() == id),
                "{id} registrado"
            );
        }
    }

    /// Fase 7 #30: los cuatro comandos se activan como las MISMAS acciones
    /// `win.*` del grupo (menú, paleta §38 y popover §44) y llegan al
    /// receptor que main.rs enlaza con `TabManager::apply_format_active`
    /// (destino único de edición, contrato #30 §UI).
    #[test]
    fn comandos_30_llegan_al_receptor() {
        /// Receptor que solo registra los comandos #30 recibidos.
        struct Recorder(Rc<std::cell::RefCell<Vec<&'static str>>>);
        impl Commands for Recorder {
            fn new(&self) {}
            fn new_window(&self) {}
            fn open(&self) {}
            fn quick_open(&self) {}
            fn open_folder(&self) {}
            fn import_markdown(&self) {}
            fn save(&self) {}
            fn save_as(&self) {}
            fn export_pdf(&self) {}
            fn close_tab(&self) {}
            fn undo(&self) {}
            fn redo(&self) {}
            fn find(&self) {}
            fn replace(&self) {}
            fn project_search(&self) {}
            fn toggle_mode(&self) {}
            fn toggle_visual_syntax(&self) {}
            fn toggle_explorer(&self) {}
            fn set_view(&self, _mode: ViewMode) {}
            fn about(&self) {}
            fn shortcuts(&self) {}
            fn insert_image(&self) {
                self.0.borrow_mut().push("insert-image");
            }
            fn insert_table(&self) {
                self.0.borrow_mut().push("insert-table");
            }
            fn edit_image(&self) {
                self.0.borrow_mut().push("edit-image");
            }
            fn edit_table_attrs(&self) {
                self.0.borrow_mut().push("edit-table-attrs");
            }
        }

        let received = Rc::new(std::cell::RefCell::new(Vec::new()));
        let host: Rc<dyn Commands> = Rc::new(Recorder(Rc::clone(&received)));
        let group = action_group(&host);
        for id in ["insert-image", "insert-table", "edit-image", "edit-table-attrs"] {
            let action = group
                .lookup_action(id)
                .unwrap_or_else(|| panic!("falta la acción win.{id} (§36)"))
                .downcast::<gio::SimpleAction>()
                .expect("SimpleAction");
            assert!(action.is_enabled(), "win.{id} habilitada");
            action.activate(None::<&glib::Variant>);
        }
        assert_eq!(
            received.borrow().as_slice(),
            [
                "insert-image",
                "insert-table",
                "edit-image",
                "edit-table-attrs"
            ],
            "las cuatro activaciones llegan al receptor (dispatch §3.7)"
        );
    }

    /// Fase 7 #29: `win.insert-callout` es stateful con tono inicial `note`
    /// (activable con `activate_action(id, None)` §38) y la activación
    /// entrega el tono del estado — el popover lo fija antes de activarla.
    #[test]
    fn insert_callout_lleva_el_tono_en_su_estado() {
        /// Receptor que solo registra los tonos recibidos.
        struct Tones(Rc<std::cell::RefCell<Vec<String>>>);
        impl Commands for Tones {
            fn new(&self) {}
            fn new_window(&self) {}
            fn open(&self) {}
            fn quick_open(&self) {}
            fn open_folder(&self) {}
            fn import_markdown(&self) {}
            fn save(&self) {}
            fn save_as(&self) {}
            fn export_pdf(&self) {}
            fn close_tab(&self) {}
            fn undo(&self) {}
            fn redo(&self) {}
            fn find(&self) {}
            fn replace(&self) {}
            fn project_search(&self) {}
            fn toggle_mode(&self) {}
            fn toggle_visual_syntax(&self) {}
            fn toggle_explorer(&self) {}
            fn set_view(&self, _mode: ViewMode) {}
            fn about(&self) {}
            fn shortcuts(&self) {}
            fn insert_callout(&self, tone: &str) {
                self.0.borrow_mut().push(tone.to_owned());
            }
        }

        let received = Rc::new(std::cell::RefCell::new(Vec::new()));
        let host: Rc<dyn Commands> = Rc::new(Tones(Rc::clone(&received)));
        let group = action_group(&host);
        let action = group
            .lookup_action("insert-callout")
            .expect("win.insert-callout registrada")
            .downcast::<gio::SimpleAction>()
            .expect("SimpleAction");
        assert!(
            action.state_type().is_some(),
            "acción stateful (estado de tipo string)"
        );
        assert_eq!(
            action.state().and_then(|s| s.str().map(str::to_owned)),
            Some(DEFAULT_CALLOUT_TONE.to_owned()),
            "tono inicial note (§22)"
        );

        // Activación sin parámetro (menú/paleta §38) usa el estado actual.
        action.activate(None::<&glib::Variant>);
        // El popover fija el tono antes de activar la MISMA acción.
        action.set_state(&glib::Variant::from("tip"));
        action.activate(None::<&glib::Variant>);
        assert_eq!(
            received.borrow().as_slice(),
            ["note".to_owned(), "tip".to_owned()],
            "el tono viaja en el estado de la acción"
        );
    }

    /// §36: estructura exacta del menú — File (New/New Window/Open/Quick
    /// Open/Open Folder/Import Markdown/Save/Save As/Export PDF/Export
    /// Composition PDF/Close Tab, ISSUE-021/029/035/023), Edit (Undo/Redo/
    /// Find/Replace/Project Search, §40.1/§40.2), View (Markdown/Visual +
    /// Editor/Split/Preview + Command Palette §38), Insert (Link/Callout/
    /// Image/Table), Format (Heading 1–6/Bold/Italic/Strikethrough/Code),
    /// Project (Preview
    /// Composition, §20/ISSUE-022) y Help (Atajos/About) — en el orden §36
    /// `File Edit View Insert Format Project Help` (fase 7 #29/#30).
    #[test]
    fn estructura_menu_36() {
        let esperado: &[(&str, &[&[(&str, &str)]])] = &[
            (
                "File",
                &[&[
                    ("New", "win.new"),
                    ("New Window", "win.new-window"),
                    ("Open", "win.open"),
                    ("Quick Open", "win.quick-open"),
                    ("Open Folder", "win.open-folder"),
                    ("Import Markdown", "win.import-markdown"),
                    ("Save", "win.save"),
                    ("Save As", "win.save-as"),
                    ("Export PDF", "win.export-pdf"),
                    ("Export Composition PDF", "win.export-composition-pdf"),
                    ("Close Tab", "win.close-tab"),
                ]],
            ),
            (
                "Edit",
                &[&[
                    ("Undo", "win.undo"),
                    ("Redo", "win.redo"),
                    ("Find", "win.find"),
                    ("Replace", "win.replace"),
                    ("Project Search", "win.project-search"),
                    ("Open Link", "win.follow-link"),
                ]],
            ),
            (
                "View",
                &[
                    &[
                        ("Markdown/Visual", "win.toggle-mode"),
                        ("Toggle Visual Syntax", "win.visual-syntax"),
                        ("Toggle Explorer", "win.toggle-explorer"),
                        ("Outline", "win.outline"),
                        ("Backlinks", "win.backlinks"),
                        ("Command Palette", "win.command-palette"),
                    ],
                    &[
                        ("Editor", "win.view-editor"),
                        ("Split", "win.view-split"),
                        ("Preview", "win.view-preview"),
                    ],
                ],
            ),
            (
                "Insert",
                &[&[
                    ("Link", "win.insert-link"),
                    ("Callout", "win.insert-callout"),
                    ("Image", "win.insert-image"),
                    ("Table", "win.insert-table"),
                ]],
            ),
            (
                "Format",
                &[&[
                    ("Heading 1", "win.format-heading1"),
                    ("Heading 2", "win.format-heading2"),
                    ("Heading 3", "win.format-heading3"),
                    ("Heading 4", "win.format-heading4"),
                    ("Heading 5", "win.format-heading5"),
                    ("Heading 6", "win.format-heading6"),
                    ("Bold", "win.format-strong"),
                    ("Italic", "win.format-emphasis"),
                    ("Strikethrough", "win.format-strikethrough"),
                    ("Code", "win.format-code"),
                ]],
            ),
            (
                "Project",
                &[&[("Preview Composition", "win.preview-composition")]],
            ),
            (
                "Help",
                &[&[("Atajos", "win.shortcuts"), ("About", "win.about")]],
            ),
        ];
        assert_eq!(MENUBAR_SPEC, esperado);
    }

    /// §20/ISSUE-022: las acciones de composición (Project ▸ Preview
    /// Composition y File ▸ Export Composition PDF) son las que exigen
    /// composiciones y por eso se deshabilitan sin proyecto.
    #[test]
    fn comandos_de_composicion_marcados_como_requirientes() {
        for cmd in ALL_COMMANDS {
            let esperado = matches!(
                cmd.action_id(),
                "win.preview-composition" | "win.export-composition-pdf"
            );
            assert_eq!(
                cmd.requires_composition(),
                esperado,
                "requires_composition desalineado en {}",
                cmd.action_id()
            );
        }
    }

    /// `build_menubar()` materializa [`MENUBAR_SPEC`] en `gio::Menu` con los
    /// mismos títulos, secciones, ítems y `action_id` (GMenu no necesita
    /// display; la introspección de `gio::MenuModel` tampoco).
    #[test]
    fn build_menubar_materializa_estructura_36() {
        let menubar = build_menubar();
        assert_eq!(menubar.n_items(), MENUBAR_SPEC.len() as i32);
        for (i, &(title, sections)) in MENUBAR_SPEC.iter().enumerate() {
            let i = i as i32;
            let titulo = menubar
                .item_attribute_value(i, "label", None)
                .expect("título del submenú");
            assert_eq!(titulo.str(), Some(title));
            let submenu = menubar.item_link(i, "submenu").expect("submenú");
            assert_eq!(submenu.n_items(), sections.len() as i32);
            for (j, entries) in sections.iter().enumerate() {
                let j = j as i32;
                let section = submenu.item_link(j, "section").expect("sección");
                assert_eq!(section.n_items(), entries.len() as i32);
                for (k, &(label, action_id)) in entries.iter().enumerate() {
                    let k = k as i32;
                    let etiqueta = section
                        .item_attribute_value(k, "label", None)
                        .expect("etiqueta del ítem");
                    let accion = section
                        .item_attribute_value(k, "action", None)
                        .expect("acción del ítem");
                    assert_eq!(etiqueta.str(), Some(label));
                    assert_eq!(accion.str(), Some(action_id));
                }
            }
        }
    }

    /// Coherencia §36/§37: todo ítem del menú corresponde a un comando
    /// registrado con su etiqueta y todo comando es alcanzable desde el
    /// menú — salvo los dos comandos de constructo #30, que el contrato §UI
    /// mantiene fuera del menú (se activan desde el popover §44 y desde la
    /// Command Palette §38 sobre la MISMA acción `win.*`).
    #[test]
    fn menu_y_comandos_coherentes() {
        const SIN_ITEM_DE_MENU: [Command; 2] = [Command::EditImage, Command::EditTableAttrs];
        let mut en_menu = HashSet::new();
        for &(_, sections) in MENUBAR_SPEC {
            for entries in sections {
                for &(label, action_id) in *entries {
                    let cmd = ALL_COMMANDS
                        .iter()
                        .find(|c| c.action_id() == action_id)
                        .unwrap_or_else(|| panic!("ítem sin comando: {action_id}"));
                    assert_eq!(cmd.label(), label, "etiqueta desalineada en {action_id}");
                    en_menu.insert(action_id);
                }
            }
        }
        for &cmd in ALL_COMMANDS {
            if SIN_ITEM_DE_MENU.contains(&cmd) {
                continue;
            }
            assert!(
                en_menu.contains(cmd.action_id()),
                "comando sin ítem de menú §36: {}",
                cmd.action_id()
            );
        }
    }

    /// Gramática de aceleradores de GTK: grupos `<Modificador>` seguidos de
    /// una tecla. Un literal inválido lo ignora GTK en silencio (el atajo
    /// queda muerto), así que la tabla §39 se valida contra la gramática.
    fn accel_es_valido(accel: &str) -> bool {
        let mut rest = accel;
        while let Some(start) = rest.find('<') {
            if start != 0 {
                return false; // texto suelto antes de un grupo
            }
            let Some(end) = rest.find('>') else {
                return false; // grupo sin cerrar
            };
            let modifier = &rest[1..end];
            if !matches!(
                modifier,
                "Control" | "Shift" | "Alt" | "Meta" | "Super" | "Primary"
            ) {
                return false;
            }
            rest = &rest[end + 1..];
        }
        // Tecla final: sin restos de grupo ni espacios.
        !rest.is_empty() && !rest.contains('>') && !rest.contains('<') && !rest.contains(' ')
    }

    #[test]
    fn aceleradores_siguen_la_gramatica_de_gtk() {
        for (action, accel) in ACCELERATORS {
            assert!(
                accel_es_valido(accel),
                "acelerador {accel:?} inválido para {action} (§39)"
            );
        }
        // El validador no es vacío: conoce los fallos reales.
        for malo in [
            "<Control>Alt>m",
            "<Controll>n",
            "<Control>",
            "Control>n",
            "",
        ] {
            assert!(!accel_es_valido(malo), "{malo:?} no debería validar");
        }
    }

    /// B1 (revisión PR #44, #22): el `enabled` de las acciones de
    /// composición se reevalúa con la disponibilidad de la lista — el grupo
    /// «win» que la ventana conserva de [`install_actions`] — sin tocar al
    /// resto de acciones. La instalación parte del mismo criterio.
    #[test]
    fn acciones_de_composicion_se_habilitan_y_deshabilitan_acorde() {
        /// Receptor sin operación: solo aporta `compositions_available`.
        struct Hoste(bool);
        impl Commands for Hoste {
            fn new(&self) {}
            fn new_window(&self) {}
            fn open(&self) {}
            fn quick_open(&self) {}
            fn open_folder(&self) {}
            fn import_markdown(&self) {}
            fn save(&self) {}
            fn save_as(&self) {}
            fn export_pdf(&self) {}
            fn close_tab(&self) {}
            fn undo(&self) {}
            fn redo(&self) {}
            fn find(&self) {}
            fn replace(&self) {}
            fn project_search(&self) {}
            fn toggle_mode(&self) {}
            fn toggle_visual_syntax(&self) {}
            fn toggle_explorer(&self) {}
            fn set_view(&self, _mode: ViewMode) {}
            fn about(&self) {}
            fn shortcuts(&self) {}
            fn compositions_available(&self) -> bool {
                self.0
            }
        }
        const COMPOSICION: [&str; 2] = ["preview-composition", "export-composition-pdf"];
        /// Estado `enabled` de cada acción compositiva, en orden: la
        /// aserción va acción a acción (B1 — probar «al menos una» no
        /// acredita que las dos sigan a la lista).
        fn estados_compositivas(group: &gio::SimpleActionGroup) -> [bool; 2] {
            COMPOSICION.map(|id| {
                group
                    .lookup_action(id)
                    .unwrap_or_else(|| panic!("falta la acción {id}"))
                    .is_enabled()
            })
        }

        // Instalación: el estado inicial sigue a `compositions_available`.
        let sin: Rc<dyn Commands> = Rc::new(Hoste(false));
        let sin = action_group(&sin);
        assert_eq!(
            estados_compositivas(&sin),
            [false, false],
            "sin composiciones, las dos deshabilitadas (§20)"
        );
        assert!(sin.lookup_action("save").expect("save").is_enabled());

        let con: Rc<dyn Commands> = Rc::new(Hoste(true));
        let con = action_group(&con);
        assert_eq!(
            estados_compositivas(&con),
            [true, true],
            "con composiciones, las dos habilitadas (§20)"
        );

        // Refresco tras redescubrir (B1): el estado sigue a la lista.
        refresh_composition_actions(&con, false);
        assert_eq!(
            estados_compositivas(&con),
            [false, false],
            "B1: al quitar la lista, las dos"
        );
        assert!(
            con.lookup_action("save").expect("save").is_enabled(),
            "el resto de acciones no se toca"
        );
        refresh_composition_actions(&con, true);
        assert_eq!(estados_compositivas(&con), [true, true], "B1: al volver");
    }
}
