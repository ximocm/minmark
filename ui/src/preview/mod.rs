//! Panel Preview de minmark (ISSUE-020): página continua del documento en una
//! WebView WebKitGTK 6.0 (ADR-001).
//!
//! - [`ViewMode`] modela los modos de vista §9 (Editor / Split / Preview).
//! - [`render_document`] es el render **puro** fuente Markdown → página HTML
//!   con el CSS embebido (§10.2): es el HTML que carga la WebView del
//!   Preview. Todo el CSS va dentro de la página (§3.4: sin recursos de red).
//! - [`PreviewPane`] es el widget GTK (`ScrolledWindow` con la WebView).
//!
//! §7.2: el HTML emitido por `minmark_renderer` escapa siempre el contenido
//! del documento; este módulo solo transporta ese resultado, nunca inyecta
//! HTML del usuario sin escapar.

use gtk4 as gtk;
use gtk4::prelude::*;
use webkit6::prelude::*;

use minmark_config::Settings;

/// Modo de vista (§9): editor solo, split editor+preview, preview solo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Solo editor (§9).
    Editor,
    /// Editor y Preview lado a lado (§9).
    Split,
    /// Solo Preview (§9).
    Preview,
}

impl ViewMode {
    /// Etiqueta del modo para el menú View (§9): "Editor" / "Split" / "Preview".
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Editor => "Editor",
            ViewMode::Split => "Split",
            ViewMode::Preview => "Preview",
        }
    }
}

/// Render puro del fuente Markdown a la página del Preview (§10.2):
/// `minmark_parser::parse` + `minmark_renderer::render_preview`. Es el HTML
/// que carga la WebView y también el que la vista Visual reutiliza. El fuente
/// viaja al renderer para que las anclas de los headings (§26) sean las mismas
/// que resuelve el índice.
pub fn render_document(source: &str, settings: &Settings) -> String {
    minmark_renderer::render_preview_with_bibliography(
        &minmark_parser::parse_with(
            source,
            &minmark_parser::ParseOptions {
                wiki_links: settings.wiki_links,
                ..Default::default()
            },
        ),
        source,
        settings,
        &crate::academic::load(settings),
    )
}

pub fn render_document_at(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
) -> String {
    if settings.pagination_enabled {
        match crate::export::paginated_preview_at(source, settings, document, root) {
            Ok(html) => return html,
            Err(error) => {
                eprintln!("minmark: Preview paginado: {error}");
                let mut html = render_document(source, settings);
                let diagnostic = error
                    .to_string()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                html = html.replace(
                    "</body>",
                    &format!("<p role=\"status\">Preview paginado: {diagnostic}</p></body>"),
                );
                return html;
            }
        }
    }
    render_document(source, settings)
}

/// Base URI de la carga en la WebView: cualquier URL relativa degrada a rutas
/// `file://` locales y nunca a la red (§3.4). El CSS va embebido en la página,
/// así que el Preview funciona sin ningún recurso externo.
const BASE_URI: &str = "file:///";

/// Panel Preview (§10.2): [`gtk::ScrolledWindow`] con una [`webkit6::WebView`]
/// dentro (ADR-001) que muestra la página continua del documento.
pub struct PreviewPane {
    scrolled: gtk::ScrolledWindow,
    webview: webkit6::WebView,
    diagnostic_script: std::rc::Rc<std::cell::RefCell<String>>,
    scroll_byte: std::rc::Rc<std::cell::Cell<Option<(u32, bool)>>>,
    document_key: std::cell::RefCell<Option<(Option<std::path::PathBuf>, u64)>>,
    pending_html: std::rc::Rc<std::cell::RefCell<Option<String>>>,
}

impl PreviewPane {
    #[cfg(test)]
    pub fn test_snapshot(&self, path: &std::path::Path, done: std::rc::Rc<std::cell::Cell<bool>>) {
        let path = path.to_owned();
        self.webview.snapshot(webkit6::SnapshotRegion::FullDocument, webkit6::SnapshotOptions::NONE, None::<&gtk::gio::Cancellable>, move |result| {
            result.expect("WebKit snapshot").save_to_png(&path).expect("snapshot PNG");
            done.set(true);
        });
    }
    #[cfg(test)]
    pub fn test_click_first_link(&self, done: std::rc::Rc<std::cell::Cell<bool>>) {
        self.webview.evaluate_javascript(
            "document.querySelector('a[href]').click(); true",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                done.set(result.is_ok());
            },
        );
    }
    #[cfg(test)]
    pub fn test_is_loading(&self) -> bool {
        self.webview.is_loading()
    }
    #[cfg(test)]
    pub fn test_eval_number(
        &self,
        script: &str,
        result: std::rc::Rc<std::cell::Cell<Option<f64>>>,
    ) {
        self.webview.evaluate_javascript(
            script,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |value| {
                result.set(Some(value.expect("Preview JavaScript").to_double()));
            },
        );
    }
    fn run_script(view: &webkit6::WebView, script: &str) {
        view.evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }
    pub fn scroll_to_byte(&self, byte: u32) {
        self.request_scroll(byte, false);
    }
    /// Mantiene visible el bloque editado sin centrarlo con cada pulsación.
    pub fn follow_edit(&self, byte: u32) {
        self.request_scroll(byte, true);
    }
    fn request_scroll(&self, byte: u32, follow: bool) {
        if self.webview.is_loading() {
            self.scroll_byte.set(Some((byte, follow)));
        } else {
            Self::run_script(&self.webview, &scroll_script(byte, follow));
        }
    }
    pub fn set_link_diagnostics(&self, links: &[crate::intelligence::Link]) {
        let entries = links
            .iter()
            .filter_map(|l| {
                crate::intelligence::problem(&l.target)
                    .map(|p| format!("[{},{}]", js_string(&l.destination), js_string(&p)))
            })
            .collect::<Vec<_>>()
            .join(",");
        let script = format!("{{ const errors = new Map([{entries}]); document.querySelectorAll('a[href]').forEach(a => {{ const message = errors.get(a.getAttribute('href')); a.style.textDecorationStyle = message ? 'wavy' : ''; if (message) {{ a.setAttribute('data-mm-error',''); a.title = message; a.setAttribute('aria-label', a.textContent + ': ' + message); }} else if (a.hasAttribute('data-mm-error')) {{ a.removeAttribute('data-mm-error'); a.removeAttribute('title'); a.removeAttribute('aria-label'); }} }}); }}");
        *self.diagnostic_script.borrow_mut() = script.clone();
        Self::run_script(&self.webview, &script);
    }
    pub fn connect_link_activation(&self, activate: std::rc::Rc<dyn Fn(String)>) {
        let manager = self
            .webview
            .user_content_manager()
            .expect("WebView content manager");
        manager.register_script_message_handler("minmarkLink", None);
        manager.connect_script_message_received(Some("minmarkLink"), move |_, value| {
            activate(value.to_str().to_string());
        });
        manager.add_script(&webkit6::UserScript::new(
            "document.addEventListener('click', e => { const a = e.target.closest('a[href]'); if (a) { e.preventDefault(); const href = a.getAttribute('href'); if (href.startsWith('#mm-note-')) { document.getElementById(href.slice(1))?.scrollIntoView({block: 'center'}); } else { window.webkit.messageHandlers.minmarkLink.postMessage(href); } } });",
            webkit6::UserContentInjectedFrames::TopFrame,
            webkit6::UserScriptInjectionTime::End, &[], &[],
        ));
        // También bloquea navegación de teclado, ventanas nuevas y enlaces
        // que no pasen por el puente: la WebView conserva el documento.
        self.webview.connect_decide_policy(|_, decision, _| {
            if let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                if nav.navigation_action().is_some_and(|mut a| {
                    a.navigation_type() == webkit6::NavigationType::LinkClicked
                }) {
                    decision.ignore();
                    return true;
                }
            }
            false
        });
    }
    /// Crea el panel con la WebView ya montada en su ScrolledWindow.
    ///
    /// Requiere GTK inicializado en el hilo principal (lo hace `main`).
    pub fn new() -> Self {
        let webview = webkit6::WebView::new();
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&webview));
        let diagnostic_script = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let scroll_byte = std::rc::Rc::new(std::cell::Cell::new(None));
        let pending_html = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let script = diagnostic_script.clone();
        let scroll = scroll_byte.clone();
        let pending = pending_html.clone();
        webview.connect_load_changed(move |view, event| {
            if event == webkit6::LoadEvent::Finished {
                if let Some(html) = pending.borrow_mut().take() {
                    Self::run_script(view, &update_script(&html));
                }
                Self::run_script(view, &script.borrow());
                if let Some((byte, follow)) = scroll.take() {
                    Self::run_script(view, &scroll_script(byte, follow));
                }
            }
        });
        Self {
            scrolled,
            webview,
            diagnostic_script,
            scroll_byte,
            document_key: std::cell::RefCell::new(None),
            pending_html,
        }
    }

    /// Widget raíz del panel: el ScrolledWindow con la WebView dentro.
    pub fn widget(&self) -> gtk::Widget {
        self.scrolled.clone().upcast::<gtk::Widget>()
    }

    /// Carga un HTML ya renderizado: el integrador comparte un único render
    /// del buffer entre el Preview y la vista Visual (§6.1, §8.2) en vez de
    /// renderizar dos veces por refresco.
    pub fn update_html(
        &self,
        html: &str,
        base_uri: Option<&str>,
        document_key: (Option<std::path::PathBuf>, u64),
    ) {
        self.diagnostic_script.borrow_mut().clear();
        if self.document_key.borrow().as_ref() == Some(&document_key) {
            // Las ediciones actualizan el DOM en la misma página. Recargar
            // la WebView descartaría su posición de lectura en cada tecla.
            if self.webview.is_loading() {
                *self.pending_html.borrow_mut() = Some(html.to_owned());
            } else {
                Self::run_script(&self.webview, &update_script(html));
            }
            return;
        }
        *self.document_key.borrow_mut() = Some(document_key);
        self.pending_html.borrow_mut().take();
        self.scroll_byte.set(None);
        self.webview
            .load_html(html, Some(base_uri.unwrap_or(BASE_URI)));
    }
}

fn update_script(html: &str) -> String {
    format!(
        r#"{{
        const x = window.scrollX, y = window.scrollY;
        const next = new DOMParser().parseFromString({}, 'text/html');
        // Reutilizar imágenes idénticas conserva su tamaño ya cargado y
        // evita saltos adicionales al decodificarlas tras cada edición.
        const images = new Map();
        document.querySelectorAll('img').forEach(img => {{
            const key = img.outerHTML;
            if (!images.has(key)) images.set(key, []);
            images.get(key).push(img);
        }});
        next.querySelectorAll('img').forEach(img => {{
            const previous = images.get(img.outerHTML)?.shift();
            if (previous) img.replaceWith(previous);
        }});
        document.documentElement.dataset.theme = next.documentElement.dataset.theme;
        document.head.replaceChildren(...next.head.childNodes);
        document.body.replaceChildren(...next.body.childNodes);
        window.scrollTo(x, y);
    }}"#,
        js_string(html)
    )
}

fn js_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .encode_utf16()
            .map(|c| format!("\\u{c:04x}"))
            .collect::<String>()
    )
}
fn scroll_script(byte: u32, follow: bool) -> String {
    format!(
        r#"{{
        let best, distance = Infinity, size = Infinity;
        document.querySelectorAll('[data-mm-span]').forEach(e => {{
            const [a,b] = e.dataset.mmSpan.split(',').map(Number);
            const d = {byte} < a ? a - {byte} : Math.max(0, {byte} - b);
            if (d < distance || (d === distance && b-a < size)) {{
                best = e; distance = d; size = b-a;
            }}
        }});
        if (best) {{
            const rect = best.getBoundingClientRect();
            if (!{follow}) best.scrollIntoView({{block: 'center'}});
            else if (rect.height <= window.innerHeight - 48) {{
                if (rect.top < 24 || rect.bottom > window.innerHeight - 24)
                    best.scrollIntoView({{block: 'center'}});
            }} else {{
                const [a,b] = best.dataset.mmSpan.split(',').map(Number);
                const ratio = Math.max(0, Math.min(1, ({byte}-a) / Math.max(1,b-a)));
                const y = rect.top + ratio * rect.height;
                if (y < 24 || y > window.innerHeight - 24)
                    window.scrollBy(0, y - window.innerHeight / 2);
            }}
        }}
    }}"#
    )
}

impl Default for PreviewPane {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_config::Theme;

    /// §10.2: el HTML del Preview lleva los bloques con `data-mm-span`
    /// (puente para el scroll sync §9.1) y el contenido renderizado.
    #[test]
    fn render_document_genera_pagina_del_preview() {
        let html = render_document("# Hola\n\n*cuerpo*\n", &Settings::default());
        assert!(html.contains("<h1 data-mm-span="), "falta el ancla del h1");
        assert!(html.contains(">Hola</h1>"));
        assert!(html.contains("<em>cuerpo</em>"));
    }

    /// §7.2: nunca HTML crudo del documento; el contenido sale escapado.
    #[test]
    fn render_document_escapa_el_html_del_documento() {
        let html = render_document("<script>alert(1)</script>\n", &Settings::default());
        assert!(
            !html.contains("<script>"),
            "el HTML del documento no se escapó"
        );
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    /// §11/§10.2: el tema (`data-theme`) y el ancho máximo configurable
    /// (`preview_max_width_px`) se reflejan en la página renderizada.
    #[test]
    fn render_document_refleja_tema_y_ancho_maximo() {
        for (theme, attr) in [
            (Theme::Light, "light"),
            (Theme::Dark, "dark"),
            (Theme::System, "system"),
        ] {
            let settings = Settings {
                theme,
                preview_max_width_px: 640,
                ..Settings::default()
            };
            let html = render_document("# T\n", &settings);
            assert!(
                html.contains(&format!("<html data-theme=\"{attr}\">")),
                "falta data-theme={attr}"
            );
            assert!(html.contains("max-width: 640px"), "falta el ancho 640px");
        }

        let html = render_document("# T\n", &Settings::default());
        assert!(html.contains(&format!(
            "max-width: {}px",
            Settings::default().preview_max_width_px
        )));
    }

    /// §9: las etiquetas de los tres modos de vista para el menú View.
    #[test]
    fn view_mode_label_cubre_los_tres_modos() {
        assert_eq!(ViewMode::Editor.label(), "Editor");
        assert_eq!(ViewMode::Split.label(), "Split");
        assert_eq!(ViewMode::Preview.label(), "Preview");
    }
}
