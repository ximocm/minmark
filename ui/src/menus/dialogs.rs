//! Diálogos GTK4 asíncronos de minmark (ISSUE-020).
//!
//! - [`open_file`] / [`save_file_as`]: `gtk::FileDialog` con filtros Markdown
//!   (*.md / *.markdown); el de guardar sugiere nombre (§12.4 Save As).
//! - [`open_image`]: selector de imagen con los formatos §18.1 (fase 7 #30).
//! - [`open_with`]: #72 (§14.8) selector de aplicaciones del sistema para
//!   un archivo; devuelve la app elegida o `None` al cancelar/cerrar.
//! - [`form_dialog`]: formulario multi-campo modal (fase 7 #30) en el patrón
//!   de [`prompt_text`]: Escape/Cancelar/cierre ⇒ `None` (cero cambios).
//! - [`equation_dialog`]: diálogo multilinea de LaTeX (fase 7 #31, §24) con
//!   diagnóstico en vivo de `minmark_math::translate` (ADR-002: el marcador
//!   de «math no soportado» avisa sin bloquear) para `win.insert-equation`
//!   y `win.edit-equation`.
//! - [`citation_dialog`]: diálogo de items clave + localizador (fase 7 #31,
//!   §25.2) con añadir/quitar filas y autocompletado bibliográfico §25.3
//!   reutilizando [`crate::intelligence::citation_completions`], para
//!   `win.insert-citation` y `win.edit-citation`.
//! - [`confirm_discard`]: confirmación explícita antes de descartar cambios
//!   sin guardar (§3.7: nada destructivo es silencioso).
//! - [`confirm_overwrite_external`]: sobrescritura solo con consentimiento
//!   explícito cuando el archivo cambió en disco (§12.7).
//!
//! Todos devuelven futuros vía la API real de gtk4 0.9 (`open_future`,
//! `save_future` y `choose_future` de `gtk::FileDialog` / `gtk::AlertDialog`,
//! GTK ≥ 4.10); sin main loop bloqueante y sin fetch de red (§3.4).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

fn file_dialog_available(parent: &impl IsA<gtk::Window>) -> bool {
    if let Err(message) = crate::runtime::check_file_dialog() {
        gtk::AlertDialog::builder()
            .message("No se puede abrir el selector de archivos")
            .detail(&message)
            .build()
            .show(Some(parent));
        false
    } else {
        true
    }
}

/// Filtro Markdown (*.md / *.markdown + MIME `text/markdown`) para los diálogos.
fn markdown_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Markdown (*.md, *.markdown)"));
    filter.add_pattern("*.md");
    filter.add_pattern("*.markdown");
    filter.add_mime_type("text/markdown");
    filter
}

fn pdf_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PDF (*.pdf)"));
    filter.add_pattern("*.pdf");
    filter.add_mime_type("application/pdf");
    filter
}

/// Filtro de imagen §18.1 (fase 7 #30): PNG, JPG/JPEG, SVG, WebP y GIF
/// estático — los únicos formatos que soportan Preview y PDF.
fn image_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(
        "Imágenes (*.png, *.jpg, *.jpeg, *.svg, *.webp, *.gif)",
    ));
    for pattern in ["*.png", "*.jpg", "*.jpeg", "*.svg", "*.webp", "*.gif"] {
        filter.add_pattern(pattern);
    }
    for mime in [
        "image/png",
        "image/jpeg",
        "image/svg+xml",
        "image/webp",
        "image/gif",
    ] {
        filter.add_mime_type(mime);
    }
    filter
}

/// Diálogo de apertura (§36, File ▸ Open): filtra archivos Markdown
/// (*.md / *.markdown), con «Todos los archivos» como respaldo. Devuelve el
/// path elegido o `None` si se cancela.
pub async fn open_file(parent: &impl IsA<gtk::Window>) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Abrir archivo");
    dialog.set_modal(true);

    let markdown = markdown_filter();
    let all = gtk::FileFilter::new();
    all.set_name(Some("Todos los archivos"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&markdown);
    filters.append(&all);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&markdown));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.open_future(Some(parent)).await.ok()?;
    file.path()
}

/// Diálogo de apertura de carpeta (ISSUE-021, File ▸ Open Folder): la
/// carpeta elegida se abre en una ventana de proyecto nueva. Devuelve la
/// carpeta o `None` si se cancela.
pub async fn open_folder(parent: &impl IsA<gtk::Window>) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Abrir carpeta");
    dialog.set_modal(true);
    let parent = parent.upcast_ref::<gtk::Window>();
    let folder = dialog.select_folder_future(Some(parent)).await.ok()?;
    folder.path()
}

/// Diálogo de apertura de imagen (§18.1, fase 7 #30): solo los formatos
/// soportados (PNG, JPG/JPEG, SVG, WebP, GIF). Devuelve el path elegido o
/// `None` si se cancela — cancelar aquí es cero cambios (no se copia nada).
pub async fn open_image(parent: &impl IsA<gtk::Window>) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Insertar imagen");
    dialog.set_modal(true);

    let filter = image_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.open_future(Some(parent)).await.ok()?;
    file.path()
}

/// #72 «Open With…» (§14.8): selector de aplicaciones del sistema sobre
/// `path`. Devuelve la app elegida; Cancelar, Escape o cerrar el diálogo ⇒
/// `None` (cero efectos, §3.7). El diálogo es modal sobre `parent` y se
/// destruye en todos los caminos.
///
/// `GtkAppChooserDialog` está marcado como obsoleto desde GTK 4.10, pero es
/// el selector de aplicaciones del sistema que pide #72: los portales cubren
/// abrir/guardar archivos, no «abrir con…». El futuro mantiene vivo el
/// diálogo mientras espera su respuesta y lo destruye al recibir una.
#[allow(deprecated)]
pub async fn open_with(parent: &impl IsA<gtk::Window>, path: &Path) -> Option<gio::AppInfo> {
    let file = gio::File::for_path(path);
    let dialog = gtk::AppChooserDialog::new(Some(parent), gtk::DialogFlags::MODAL, &file);
    dialog.set_title(Some("Open With…"));
    let respuesta = {
        let dialog = dialog.clone();
        gio::GioFuture::new(&dialog, |dialog, _cancellable, send| {
            // `resolve` consume el emisor: solo el primer response cuenta.
            let send = std::cell::RefCell::new(Some(send));
            dialog.connect_response(move |_, response| {
                if let Some(send) = send.borrow_mut().take() {
                    send.resolve(response);
                }
            });
        })
    };
    dialog.present();
    let response: gtk::ResponseType = respuesta.await;
    // Solo el botón de aceptar entrega app; el resto, cero lanzamientos.
    let app = (response == gtk::ResponseType::Accept)
        .then(|| dialog.app_info())
        .flatten();
    dialog.destroy();
    app
}

/// Diálogo «Guardar como» (§12.4): sugiere `suggested` como nombre inicial.
/// Devuelve el path elegido o `None` si se cancela.
pub async fn save_file_as(parent: &impl IsA<gtk::Window>, suggested: &str) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Guardar como");
    dialog.set_modal(true);
    if !suggested.is_empty() {
        dialog.set_initial_name(Some(suggested));
    }

    let markdown = markdown_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&markdown);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&markdown));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.save_future(Some(parent)).await.ok()?;
    file.path()
}

/// Diálogo «Exportar PDF» (§32.1, ISSUE-035): sugiere `suggested` como nombre
/// inicial y filtra `*.pdf`. Devuelve el path elegido o `None` si se cancela.
pub async fn save_pdf(parent: &impl IsA<gtk::Window>, suggested: &str) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Exportar PDF");
    dialog.set_modal(true);
    if !suggested.is_empty() {
        dialog.set_initial_name(Some(suggested));
    }

    let pdf = pdf_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&pdf);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&pdf));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.save_future(Some(parent)).await.ok()?;
    file.path()
}

/// Confirmación de descarte de cambios sin guardar (§3.7).
///
/// `title` identifica el documento afectado. Solo el botón «Descartar»
/// devuelve `true`; Cancelar, Escape o cerrar el diálogo devuelven `false`
/// (nunca se descarta en silencio).
pub async fn confirm_discard(parent: &impl IsA<gtk::Window>, title: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("¿Descartar cambios sin guardar?")
        .detail(format!("Se perderán los cambios de {title}."))
        .buttons(["Descartar", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// §12.7: el archivo cambió en disco; sobrescribir solo con consentimiento explícito.
///
/// Solo el botón «Sobrescribir» devuelve `true`; en cualquier otro caso se
/// conserva el archivo externo y el usuario decide cómo continuar.
pub async fn confirm_overwrite_external(parent: &impl IsA<gtk::Window>, path: &Path) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("El archivo cambió en disco")
        .detail(format!(
            "«{}» fue modificado fuera de minmark. ¿Sobrescribirlo con tu versión?",
            path.display()
        ))
        .buttons(["Sobrescribir", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// Confirmación de envío a la papelera (§14.9, §3.7: nada destructivo en
/// silencio). Solo «Enviar a la papelera» devuelve `true`.
pub async fn confirm_trash(parent: &impl IsA<gtk::Window>, name: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("¿Enviar a la papelera?")
        .detail(format!(
            "«{name}» se enviará a la papelera del sistema (§14.9); no se \
             elimina permanentemente."
        ))
        .buttons(["Enviar a la papelera", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// Prompt de texto modal para operaciones del explorador (§14.8): nombre de
/// Markdown/carpeta, nuevo nombre en `Rename` o destino en `Move`.
///
/// `on_submit(Some(text))` al aceptar (botón o Enter), `on_submit(None)` al
/// cancelar. Callback y no futuro: las operaciones del explorador son
/// cadenas de diálogos con estado propio (ISSUE-023).
pub fn prompt_text(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    initial: &str,
    on_submit: Rc<dyn Fn(Option<String>)>,
) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(360)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&entry);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    {
        let window = window.clone();
        let entry = entry.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let text = entry.text().to_string();
            window.destroy();
            on_submit(Some(text));
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape cancela (§3.7 / #29: mismo camino que «Cancelar» y el cierre
    // de la ventana ⇒ `on_submit(None)`, cero cambios en el documento).
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}

/// Campo de un formulario multi-campo modal (fase 7 #30): etiqueta visible
/// y valor inicial. El orden de los campos es el orden de los valores que
/// entrega [`form_dialog`].
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub value: String,
}

impl FormField {
    pub fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
        }
    }
}

/// Validador de [`form_dialog`] (fase 7 #30): `Some(mensaje)` retiene el
/// formulario abierto mostrando el error; `None` valida y se entregan los
/// valores. Fn pointer: los validadores son funciones puras.
pub type FormValidator = fn(&[String]) -> Option<String>;

/// Formulario multi-campo modal (fase 7 #30), en el patrón de
/// [`prompt_text`]: una ventana modal con una [`FormField`] por fila,
/// «Cancelar», Escape y el cierre de la ventana ⇒ `on_submit(None)` (cero
/// cambios, #29/#30); «Aceptar» (o Enter, `activates_default`) ⇒
/// `on_submit(Some(valores))` en orden de `fields`.
///
/// `validate` (si se da) recibe los valores y devuelve `Some(mensaje)` para
/// RETENER el formulario abierto mostrando el error — ni acepta ni cancela;
/// `None` valida y se entrega. Enter en cualquier campo activa el botón
/// por defecto («Aceptar») con foco en la ventana (GTK nativo).
pub fn form_dialog(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    fields: &[FormField],
    validate: Option<FormValidator>,
    on_submit: Rc<dyn Fn(Option<Vec<String>>)>,
) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(400)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let mut entries: Vec<gtk::Entry> = Vec::with_capacity(fields.len());
    for field in fields {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&field.label));
        label.set_xalign(0.0);
        let entry = gtk::Entry::new();
        entry.set_text(&field.value);
        entry.set_activates_default(true);
        entry.set_hexpand(true);
        entry.update_property(&[gtk::accessible::Property::Label(field.label.as_str())]);
        row.append(&label);
        row.append(&entry);
        vbox.append(&row);
        entries.push(entry);
    }

    // Error de validación (fase 7 #30): visible bajo los campos mientras el
    // formulario sigue abierto.
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&error);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    // Aceptar: si `validate` objecta, el error se muestra y el formulario
    // sigue abierto; si no, destruye y entrega los valores.
    {
        let window = window.clone();
        let error = error.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let values: Vec<String> = entries
                .iter()
                .map(|entry| entry.text().to_string())
                .collect();
            if let Some(message) = validate.and_then(|validate| validate(&values)) {
                error.set_text(&message);
                error.set_visible(true);
                return;
            }
            window.destroy();
            on_submit(Some(values));
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape cancela (#29/#30: mismo camino que «Cancelar» y el cierre de
    // la ventana ⇒ `on_submit(None)`, cero cambios en el documento).
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}

/// Texto completo del [`gtk::TextView`] `view` (patrón de lectura de los
/// diálogos #31).
fn text_view_text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

/// Diálogo de ecuación LaTeX (fase 7 #31, §24): campo multilinea con
/// diagnóstico EN VIVO de [`minmark_math::translate`] — fuera del
/// subconjunto (ADR-002) se muestra el marcador
/// `⚠ [math no soportado: …]` como aviso NO bloqueante: aceptar sigue
/// siendo válido y el contenido se conserva visible y editable (§24).
///
/// `initial` precarga el campo (`win.edit-equation` pasa el LaTeX del
/// contexto; `win.insert-equation` arranca vacío). Aceptar ⇒
/// `on_submit(Some(latex))` VERBATIM (un valor intacto sigue siendo no-op);
/// LaTeX vacío retiene el diálogo con el error visible; Cancelar, Escape o
/// el cierre ⇒ `on_submit(None)` (cero cambios, #29/#30/#31).
pub fn equation_dialog(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    initial: &str,
    on_submit: Rc<dyn Fn(Option<String>)>,
) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(480)
        .default_height(240)
        .resizable(true)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_monospace(true);
    view.update_property(&[gtk::accessible::Property::Label("LaTeX")]);
    view.buffer().set_text(initial);
    {
        let buffer = view.buffer();
        let end = buffer.end_iter();
        buffer.place_cursor(&end);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(140);
    scroll.set_child(Some(&view));

    // Diagnóstico en vivo (ADR-002): marcador visible, sin bloquear.
    let warning = gtk::Label::new(None);
    warning.set_xalign(0.0);
    warning.set_wrap(true);
    warning.add_css_class("warning");
    warning.set_visible(false);
    let refresh_warning = {
        let warning = warning.clone();
        let view = view.clone();
        Rc::new(move || {
            let translation = minmark_math::translate(&text_view_text(&view));
            if translation.is_supported() {
                warning.set_text("");
                warning.set_visible(false);
            } else {
                warning.set_text(&translation.marker());
                warning.set_visible(true);
            }
        })
    };
    refresh_warning();
    view.buffer().connect_changed({
        let refresh_warning = Rc::clone(&refresh_warning);
        move |_| refresh_warning()
    });

    // Error de validación (LaTeX vacío) reteniendo el diálogo, como
    // `form_dialog` (#30).
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&scroll);
    vbox.append(&warning);
    vbox.append(&error);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    {
        let window = window.clone();
        let error = error.clone();
        let view = view.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let latex = text_view_text(&view);
            if latex.trim().is_empty() {
                error.set_text("El LaTeX no puede estar vacío (§24)");
                error.set_visible(true);
                return;
            }
            window.destroy();
            on_submit(Some(latex));
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape cancela (#29/#30: mismo camino que «Cancelar» ⇒ `None`,
    // cero cambios); el aviso ADR-002 nunca bloquea este camino.
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}

/// Fila de items del diálogo de citas (#31): clave + localizador + quitar.
struct CitationRow {
    /// Caja de la fila: su identidad para quitarla del contenedor.
    row: gtk::Box,
    key: gtk::Entry,
    locator: gtk::Entry,
    remove: gtk::Button,
}

/// Offset de byte del cursor `char_position` (GtkEditable mide en
/// caracteres) dentro de `text`.
fn byte_at_char(text: &str, char_position: i32) -> usize {
    let chars = char_position.max(0) as usize;
    text.char_indices()
        .map(|(offset, _)| offset)
        .nth(chars)
        .unwrap_or(text.len())
}

/// Popup de completado bibliográfico (§25.3) sobre la entrada `key`: arma
/// el prefijo canónico `[@…` y delega en
/// [`crate::intelligence::citation_completions`] (NUNCA reimplementado) con
/// el patrón de teclado del completion del editor (#29): Escape/Up/Down
/// sobre la lista, Enter o Tab acepta la fila seleccionada, cualquier otra
/// tecla cierra el popup; un clic en la fila también acepta.
fn attach_citation_completion(
    key: &gtk::Entry,
    bibliography: &Rc<minmark_renderer::Bibliography>,
) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_parent(key);
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    popover.set_can_focus(false);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_width(320);
    scroll.set_max_content_height(240);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    crate::list_scroll::follow_selection(&list);
    popover.set_child(Some(&scroll));

    let items: Rc<RefCell<Vec<crate::intelligence::Completion>>> =
        Rc::new(RefCell::new(Vec::new()));
    // Guardia de reescritura: `set_text` al aceptar no relanza el cálculo.
    let applying = Rc::new(Cell::new(false));

    let accept_into = {
        let key = key.clone();
        let applying = Rc::clone(&applying);
        Rc::new(move |text: &str| {
            applying.set(true);
            key.set_text(text);
            key.set_position(text.chars().count() as i32);
            applying.set(false);
        })
    };

    // Clic en una fila: acepta ESA clave (patrón `row_activated`).
    {
        let items = Rc::clone(&items);
        let popover = popover.clone();
        let accept_into = Rc::clone(&accept_into);
        list.connect_row_activated(move |_, row| {
            let item = items.borrow().get(row.index() as usize).cloned();
            popover.popdown();
            if let Some(item) = item {
                accept_into(&item.text);
            }
        });
    }

    // Cálculo compartido: `changed` Y el movimiento del cursor dentro de la
    // clave re-filtran las sugerencias §25.3 (el cursor se mueve con las
    // flechas o al precargar sin que el texto cambie).
    let recompute = {
        let key = key.clone();
        let popover = popover.clone();
        let list = list.clone();
        let items = Rc::clone(&items);
        let applying = Rc::clone(&applying);
        let bibliography = Rc::clone(bibliography);
        Rc::new(move || {
            if applying.get() {
                return;
            }
            let text = key.text().to_string();
            let cursor = byte_at_char(&text, key.position());
            let source = format!("[@{text}");
            let completions =
                crate::intelligence::citation_completions(&source, 2 + cursor, &bibliography);
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            if completions.is_empty() {
                items.borrow_mut().clear();
                popover.popdown();
                return;
            }
            for (index, item) in completions.iter().enumerate() {
                let label = gtk::Label::new(Some(&item.label));
                label.set_xalign(0.0);
                list.append(&label);
                if let Some(row) = list.row_at_index(index as i32) {
                    row.update_property(&[gtk::accessible::Property::Label(item.label.as_str())]);
                }
            }
            *items.borrow_mut() = completions;
            if let Some(row) = list.row_at_index(0) {
                list.select_row(Some(&row));
            }
            popover.popup();
        })
    };
    key.connect_changed({
        let recompute = Rc::clone(&recompute);
        move |_| recompute()
    });
    {
        let recompute = Rc::clone(&recompute);
        let popover = popover.clone();
        key.connect_local("notify::cursor-position", false, move |_| {
            // Mover el cursor solo RE-filtra si el popup ya está abierto:
            // §25.3 se dispara al escribir, no al pinchar en la clave.
            if popover.is_visible() {
                recompute();
            }
            None
        });
    }

    // Teclado (foco en la entrada; captura antes de los bindings de GTK).
    {
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let popover = popover.clone();
        let list = list.clone();
        let items = Rc::clone(&items);
        let accept_into = Rc::clone(&accept_into);
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            if !popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let selected = list.selected_row().map(|row| row.index()).unwrap_or(0);
            if key == gtk::gdk::Key::Escape {
                popover.popdown();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Up || key == gtk::gdk::Key::Down {
                let next = if key == gtk::gdk::Key::Up {
                    selected - 1
                } else {
                    selected + 1
                };
                if let Some(row) = list.row_at_index(next.max(0)) {
                    list.select_row(Some(&row));
                }
                return glib::Propagation::Stop;
            }
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::Tab)
                && modifiers.is_empty()
            {
                let item = items.borrow().get(selected as usize).cloned();
                popover.popdown();
                if let Some(item) = item {
                    accept_into(&item.text);
                }
                return glib::Propagation::Stop;
            }
            popover.popdown();
            glib::Propagation::Proceed
        });
        key.add_controller(controller);
    }

    popover
}

/// Accesible por posición (#31: nombres accesibles que siguen a las filas
/// tras añadir/quitar).
fn renumber_citation_rows(rows: &[CitationRow]) {
    for (index, row) in rows.iter().enumerate() {
        let n = index + 1;
        row.key
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Clave del ítem {n}"
            ))]);
        row.locator
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Localizador del ítem {n}"
            ))]);
        row.remove
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Quitar ítem {n}"
            ))]);
    }
}

/// Añade una fila (clave + localizador + quitar) al diálogo de citas y
/// devuelve la entrada de clave (para enfocarla al añadir).
fn append_citation_row(
    container: &gtk::Box,
    rows: &Rc<RefCell<Vec<CitationRow>>>,
    bibliography: &Rc<minmark_renderer::Bibliography>,
    item: Option<&minmark_document::CitationItem>,
) -> gtk::Entry {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let key = gtk::Entry::new();
    key.set_hexpand(true);
    key.set_placeholder_text(Some("Clave"));
    key.set_activates_default(true);
    let locator = gtk::Entry::new();
    locator.set_hexpand(true);
    locator.set_placeholder_text(Some("Localizador (p. 42)"));
    locator.set_activates_default(true);
    let remove = gtk::Button::with_label("Quitar");
    remove.set_tooltip_text(Some("Quitar esta fila de la cita"));
    if let Some(item) = item {
        key.set_text(&item.key);
        locator.set_text(item.locator.as_deref().unwrap_or(""));
    }
    attach_citation_completion(&key, bibliography);
    row.append(&key);
    row.append(&locator);
    row.append(&remove);
    container.append(&row);
    rows.borrow_mut().push(CitationRow {
        row: row.clone(),
        key: key.clone(),
        locator,
        remove: remove.clone(),
    });
    {
        let container = container.clone();
        let rows = Rc::clone(rows);
        remove.connect_clicked(move |_| {
            let index = {
                let rows = rows.borrow();
                rows.iter().position(|stored| stored.row == row)
            };
            let Some(index) = index else { return };
            let removed = { rows.borrow_mut().remove(index) };
            container.remove(&removed.row);
            renumber_citation_rows(&rows.borrow());
        });
    }
    renumber_citation_rows(&rows.borrow());
    key
}

/// Claves de cita según la gramática consumidora (§25.2, espejo de
/// `valid_citation_key`/`try_citation`): alfanuméricos ASCII más
/// `: . _ -` y `* + / !`; cualquier otra cosa no es sintaxis.
fn valid_citation_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-' | '*' | '+' | '/' | '!'))
}

/// Localizador según la gramática consumidora (§25.2, espejo de
/// `valid_citation_locator`): texto libre que no puede estar vacío ni
/// contener el separador de items `;`, el cierre `]` ni saltos de línea.
fn valid_citation_locator(locator: &str) -> bool {
    !locator.is_empty() && !locator.contains([';', ']', '\n'])
}

/// §25.2: valida los items contra la gramática que consume el parser y el
/// plan núcleo — `Some(mensaje)` RETIENE el diálogo abierto con el error
/// visible (patrón `FormValidator` de `form_dialog`, #30); `None` = apto
/// para `CitationInsert`/`CitationEdit`, que exige lo mismo, así que un
/// valor aceptado siempre aplica y uno rechazado es cero cambios.
pub fn validate_citation_items(items: &[minmark_document::CitationItem]) -> Option<String> {
    if items.is_empty() {
        return Some("Añade al menos un ítem a la cita (§25.2)".to_owned());
    }
    if items.iter().any(|item| item.key.trim().is_empty()) {
        return Some("Cada ítem necesita una clave (§25.2)".to_owned());
    }
    if let Some(item) = items
        .iter()
        .find(|item| !valid_citation_key(item.key.trim()))
    {
        return Some(format!(
            "Clave «{}» no válida (§25.2): solo letras, dígitos y «: . _ - * + / !»",
            item.key.trim()
        ));
    }
    if let Some(item) = items
        .iter()
        .find(|item| {
            item.locator
                .as_deref()
                .is_some_and(|locator| !valid_citation_locator(locator.trim()))
        })
    {
        return Some(format!(
            "Localizador de «{}» no válido (§25.2): no puede estar vacío ni contener «;», «]» ni saltos de línea",
            item.key.trim()
        ));
    }
    None
}

/// Diálogo de citas Pandoc (fase 7 #31, §25.2): lista de items (clave +
/// localizador) con añadir/quitar filas y autocompletado bibliográfico
/// §25.3 por clave/autor/título/año en cada entrada de clave.
///
/// `items` precarga las filas (`win.edit-citation` pasa los items del
/// contexto; `win.insert-citation` arranca con una fila vacía).
/// Aceptar ⇒ `on_submit(Some(items))` (clave y localizador recortados);
/// Cancelar, Escape o el cierre ⇒ `on_submit(None)` (cero cambios).
/// El diálogo es idéntico para insertar y editar: solo cambia la acción que
/// main.rs aplica (`CitationInsert` / `CitationEdit`).
pub fn citation_dialog(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    items: &[minmark_document::CitationItem],
    bibliography: Rc<minmark_renderer::Bibliography>,
    on_submit: Rc<dyn Fn(Option<Vec<minmark_document::CitationItem>>)>,
) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(560)
        .resizable(true)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let rows: Rc<RefCell<Vec<CitationRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(100);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&rows_box));
    if items.is_empty() {
        append_citation_row(&rows_box, &rows, &bibliography, None);
    } else {
        for item in items {
            append_citation_row(&rows_box, &rows, &bibliography, Some(item));
        }
    }

    let add = gtk::Button::with_label("Añadir ítem");
    add.update_property(&[gtk::accessible::Property::Label("Añadir ítem")]);
    add.connect_clicked({
        let rows_box = rows_box.clone();
        let rows = Rc::clone(&rows);
        let bibliography = Rc::clone(&bibliography);
        move |_| {
            let key = append_citation_row(&rows_box, &rows, &bibliography, None);
            key.grab_focus();
        }
    });

    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&scroll);
    vbox.append(&add);
    vbox.append(&error);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    let collect_items = {
        let rows = Rc::clone(&rows);
        Rc::new(move || -> Vec<minmark_document::CitationItem> {
            rows.borrow()
                .iter()
                .map(|row| {
                    let key = row.key.text().to_string();
                    let locator = row.locator.text().to_string();
                    minmark_document::CitationItem {
                        key: key.trim().to_owned(),
                        locator: (!locator.trim().is_empty())
                            .then(|| locator.trim().to_owned()),
                    }
                })
                .collect()
        })
    };

    {
        let window = window.clone();
        let error = error.clone();
        let collect_items = Rc::clone(&collect_items);
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let items = collect_items();
            if let Some(message) = validate_citation_items(&items) {
                error.set_text(&message);
                error.set_visible(true);
                return;
            }
            window.destroy();
            on_submit(Some(items));
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape cancela (#29/#30/#31): con el popup de completado abierto la
    // captura de la entrada lo cierra primero (Stop) y el segundo Escape
    // llega aquí ⇒ `on_submit(None)`, cero cambios.
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}

/// Selección de composición (§20, ISSUE-022/023): lista modal con las
/// composiciones del proyecto, en el patrón de [`prompt_text`].
///
/// `on_submit(Some(nombre))` al aceptar la fila seleccionada (doble click o
/// Aceptar), `on_submit(None)` al cancelar (botón, Escape o cierre). El
/// diálogo es callback y no futuro para encadenarse con los futuros del
/// flujo de exportación.
pub fn choose_composition(
    parent: &impl IsA<gtk::Window>,
    names: &[String],
    on_submit: Rc<dyn Fn(Option<String>)>,
) {
    let window = gtk::Window::builder()
        .title("Composición")
        .modal(true)
        .transient_for(parent)
        .default_width(360)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    for name in names {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_min_content_height(180);
    scroll.set_vexpand(true);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&scroll);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    // Nombre de la fila seleccionada; `None` si no hay fila. La lista es
    // propia del diálogo (los callbacks GTK son `'static`).
    let nombres = names.to_vec();
    let seleccionada = {
        let list = list.clone();
        Rc::new(move || -> Option<String> {
            let row = list.selected_row()?;
            let row = row.index().max(0) as usize;
            nombres.get(row).cloned()
        })
    };
    {
        let window = window.clone();
        let seleccionada = seleccionada.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let nombre = seleccionada();
            window.destroy();
            on_submit(nombre);
        });
    }
    // Doble click sobre una fila: acepta ESA fila (la selecciona primero).
    {
        let para_seleccionar = list.clone();
        let seleccionada = seleccionada.clone();
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        list.connect_row_activated(move |_, row| {
            para_seleccionar.select_row(Some(row));
            let nombre = seleccionada();
            window.destroy();
            on_submit(nombre);
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape y el cierre de la ventana cancelan (§3.7: nunca en silencio
    // seleccionan una por defecto).
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}
