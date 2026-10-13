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
    minmark_renderer::render_preview(
        &minmark_parser::parse_with(
            source,
            &minmark_parser::ParseOptions {
                wiki_links: settings.wiki_links,
                ..Default::default()
            },
        ),
        source,
        settings,
    )
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
    scroll_byte: std::rc::Rc<std::cell::Cell<Option<u32>>>,
}

impl PreviewPane {
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
    fn run_script(view: &webkit6::WebView, script: &str) {
        view.evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }
    pub fn scroll_to_byte(&self, byte: u32) {
        self.scroll_byte.set(Some(byte));
        Self::run_script(&self.webview, &scroll_script(byte));
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
            "document.addEventListener('click', e => { const a = e.target.closest('a[href]'); if (a) { e.preventDefault(); window.webkit.messageHandlers.minmarkLink.postMessage(a.getAttribute('href')); } });",
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
        let script = diagnostic_script.clone();
        let scroll = scroll_byte.clone();
        webview.connect_load_changed(move |view, event| {
            if event == webkit6::LoadEvent::Finished {
                Self::run_script(view, &script.borrow());
                if let Some(byte) = scroll.get() {
                    Self::run_script(view, &scroll_script(byte));
                }
            }
        });
        Self {
            scrolled,
            webview,
            diagnostic_script,
            scroll_byte,
        }
    }

    /// Widget raíz del panel: el ScrolledWindow con la WebView dentro.
    pub fn widget(&self) -> gtk::Widget {
        self.scrolled.clone().upcast::<gtk::Widget>()
    }

    /// Carga un HTML ya renderizado: el integrador comparte un único render
    /// del buffer entre el Preview y la vista Visual (§6.1, §8.2) en vez de
    /// renderizar dos veces por refresco.
    pub fn update_html(&self, html: &str, base_uri: Option<&str>) {
        self.diagnostic_script.borrow_mut().clear();
        self.scroll_byte.set(None);
        self.webview
            .load_html(html, Some(base_uri.unwrap_or(BASE_URI)));
    }
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
fn scroll_script(byte: u32) -> String {
    format!("{{ let best; document.querySelectorAll('[data-mm-span]').forEach(e => {{ const [a,b] = e.dataset.mmSpan.split(',').map(Number); if (a <= {byte} && {byte} < b) best=e; }}); if(best) best.scrollIntoView({{block:'center'}}); }}")
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
