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
use webkit6::prelude::WebViewExt;

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
/// que carga la WebView y también el que la vista Visual reutiliza.
pub fn render_document(source: &str, settings: &Settings) -> String {
    minmark_renderer::render_preview(&minmark_parser::parse(source), settings)
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
}

impl PreviewPane {
    /// Crea el panel con la WebView ya montada en su ScrolledWindow.
    ///
    /// Requiere GTK inicializado en el hilo principal (lo hace `main`).
    pub fn new() -> Self {
        let webview = webkit6::WebView::new();
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&webview));
        Self { scrolled, webview }
    }

    /// Widget raíz del panel: el ScrolledWindow con la WebView dentro.
    pub fn widget(&self) -> gtk::Widget {
        self.scrolled.clone().upcast::<gtk::Widget>()
    }

    /// Carga un HTML ya renderizado: el integrador comparte un único render
    /// del buffer entre el Preview y la vista Visual (§6.1, §8.2) en vez de
    /// renderizar dos veces por refresco.
    pub fn update_html(&self, html: &str) {
        self.webview.load_html(html, Some(BASE_URI));
    }
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
        assert!(!html.contains("<script>"), "el HTML del documento no se escapó");
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
        assert!(html.contains(&format!("max-width: {}px", Settings::default().preview_max_width_px)));
    }

    /// §9: las etiquetas de los tres modos de vista para el menú View.
    #[test]
    fn view_mode_label_cubre_los_tres_modos() {
        assert_eq!(ViewMode::Editor.label(), "Editor");
        assert_eq!(ViewMode::Split.label(), "Split");
        assert_eq!(ViewMode::Preview.label(), "Preview");
    }
}
