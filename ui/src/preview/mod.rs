//! Panel Preview de minmark (ISSUE-020): página continua del documento en el
//! webview del motor nativo (ADR-001, ADR-005).
//!
//! - [`ViewMode`] modela los modos de vista §9 (Editor / Split / Preview).
//! - [`render_document`] es el render **puro** fuente Markdown → página HTML
//!   con el CSS embebido (§10.2): es el HTML que carga la WebView del
//!   Preview. Todo el CSS va dentro de la página (§3.4: sin recursos de red).
//! - [`PreviewPane`] es el widget GTK (`ScrolledWindow` con la WebView).
//! - El webview concreto vive tras el trait privado `PreviewEngine`: hoy su
//!   única implementación es WebKitGTK 6.0 en `engine` (Linux, ADR-005);
//!   WebView2 y WKWebView entrarán por el mismo trait.
//!
//! §7.2: el HTML emitido por `minmark_renderer` escapa siempre el contenido
//! del documento; este módulo solo transporta ese resultado, nunca inyecta
//! HTML del usuario sin escapar.

use gtk4 as gtk;
use gtk4::prelude::*;

use minmark_config::Settings;

// El motor WebKitGTK 6.0 solo existe como librería del sistema en Linux
// (ADR-005: sin binarios para Windows ni port usable de macOS, y los BSD no
// empaquetan WebKitGTK 6.0), de ahí `target_os = "linux"` y no `unix`. Los
// motores WebView2 (Windows) y WKWebView (macOS) se añadirán entonces en
// sus propios módulos; hasta que lleguen, el `ui` compila en Linux.
#[cfg(target_os = "linux")]
mod engine;
#[cfg(target_os = "windows")]
mod engine_webview2;
#[cfg(target_os = "macos")]
mod engine_wkwebview;

/// Motor del Preview según el sistema operativo (ADR-005): WebKitGTK en
/// Linux, WebView2 en Windows y WKWebView en macOS.
#[cfg(target_os = "linux")]
fn create_engine() -> Box<dyn PreviewEngine> {
    Box::new(engine::WebkitEngine::new())
}

#[cfg(target_os = "windows")]
fn create_engine() -> Box<dyn PreviewEngine> {
    Box::new(engine_webview2::Webview2Engine::new())
}

#[cfg(target_os = "macos")]
fn create_engine() -> Box<dyn PreviewEngine> {
    Box::new(engine_wkwebview::WkwebviewEngine::new())
}

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
    #[cfg(test)]
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
        let meta = minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse_with(
            source,
            &minmark_parser::ParseOptions {
                wiki_links: settings.wiki_links,
                ..Default::default()
            },
        ));
        match crate::export::paginated_preview_at(source, settings, document, root, &meta) {
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

/// Motor del webview detrás de [`PreviewPane`] (ADR-005): transporte del HTML
/// y del JS, ciclo de carga y puentes nativos. Es privado a propósito: la
/// navegación, los scripts y el render ya son agnósticos del motor aquí, y el
/// backend concreto queda confinado en su módulo.
///
/// La carga es «HTML con URI base `file:///`» (§3.4), no una navegación por
/// cadena: ningún backend puede apoyarse en `NavigateToString` de WebView2
/// (límite de 2 MiB), así que el motor Windows alojará la misma página en un
/// archivo temporal local.
trait PreviewEngine {
    /// Widget GTK nativo del motor, para montarlo en el `ScrolledWindow`.
    fn widget(&self) -> gtk::Widget;
    /// Carga `html` como documento nuevo con `base_uri` como URI base.
    fn load_html(&self, html: &str, base_uri: &str);
    /// Evalúa `script` sin esperar resultado (dispara y olvida).
    fn run_script(&self, script: &str);
    /// `true` mientras el motor está cargando un documento.
    fn is_loading(&self) -> bool;
    /// Aviso de que la carga terminó: el panel aplica aquí las actualizaciones
    /// diferidas, el diagnóstico de enlaces y el scroll pendiente. El callback
    /// recibe un handle al motor para poder evaluar sus scripts.
    fn connect_load_finished(&self, callback: std::rc::Rc<dyn Fn(&dyn PreviewEngine)>);
    /// Script de usuario inicial + canal de mensajes `minmarkLink` (el
    /// postMessage concreto del puente lo define cada backend).
    fn connect_link_activation(&self, activate: std::rc::Rc<dyn Fn(String)>);
    /// Veta la navegación que salga del documento: la WebView conserva la
    /// página (§3.4), igual que el resto del código.
    fn block_navigation(&self);
    /// Captura del documento completo a PNG para la evidencia de QA.
    #[cfg(test)]
    fn snapshot(&self, path: &std::path::Path, done: std::rc::Rc<std::cell::Cell<bool>>);
    /// Evalúa `script` y reporta si la evaluación terminó sin error.
    #[cfg(test)]
    fn eval_ok(&self, script: &str, done: std::rc::Rc<std::cell::Cell<bool>>);
    /// Evalúa `script` y entrega su resultado numérico.
    #[cfg(test)]
    fn eval_number(&self, script: &str, result: std::rc::Rc<std::cell::Cell<Option<f64>>>);
}

/// Panel Preview (§10.2): [`gtk::ScrolledWindow`] con el webview del motor
/// (`PreviewEngine`) dentro (ADR-001, ADR-005) que muestra la página
/// continua del documento.
pub struct PreviewPane {
    scrolled: gtk::ScrolledWindow,
    engine: Box<dyn PreviewEngine>,
    diagnostic_script: std::rc::Rc<std::cell::RefCell<String>>,
    scroll_byte: std::rc::Rc<std::cell::Cell<Option<(u32, bool)>>>,
    document_key: std::cell::RefCell<Option<(Option<std::path::PathBuf>, u64)>>,
    pending_html: std::rc::Rc<std::cell::RefCell<Option<String>>>,
}

impl PreviewPane {
    #[cfg(test)]
    pub fn test_snapshot(&self, path: &std::path::Path, done: std::rc::Rc<std::cell::Cell<bool>>) {
        self.engine.snapshot(path, done);
    }
    #[cfg(test)]
    pub fn test_click_first_link(&self, done: std::rc::Rc<std::cell::Cell<bool>>) {
        self.engine
            .eval_ok("document.querySelector('a[href]').click(); true", done);
    }
    #[cfg(test)]
    pub fn test_is_loading(&self) -> bool {
        self.engine.is_loading()
    }
    #[cfg(test)]
    pub fn test_eval_number(
        &self,
        script: &str,
        result: std::rc::Rc<std::cell::Cell<Option<f64>>>,
    ) {
        self.engine.eval_number(script, result);
    }
    pub fn scroll_to_byte(&self, byte: u32) {
        self.request_scroll(byte, false);
    }
    /// Mantiene visible el bloque editado sin centrarlo con cada pulsación.
    pub fn follow_edit(&self, byte: u32) {
        self.request_scroll(byte, true);
    }
    /// Destino interno `#fragmento` (S1, ISSUE-024, revisión PR #44):
    /// desplaza in-page hasta el elemento con ese `id` (§26), con el mismo
    /// patrón que el `#mm-note-` del listener inyectado, sin salir del
    /// documento. Los enlaces que no son fragmentos no hacen nada (enlaces a
    /// archivos: fuera de alcance, §3.4). Devuelve si era un fragmento.
    pub fn scroll_to_fragment(&self, destination: &str) -> bool {
        let Some(script) = fragment_script(destination) else {
            return false;
        };
        self.engine.run_script(&script);
        true
    }
    fn request_scroll(&self, byte: u32, follow: bool) {
        if self.engine.is_loading() {
            self.scroll_byte.set(Some((byte, follow)));
        } else {
            self.engine.run_script(&scroll_script(byte, follow));
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
        self.engine.run_script(&script);
    }
    pub fn connect_link_activation(&self, activate: std::rc::Rc<dyn Fn(String)>) {
        self.engine.connect_link_activation(activate);
        self.engine.block_navigation();
    }
    /// Crea el panel con la WebView ya montada en su ScrolledWindow.
    ///
    /// Requiere GTK inicializado en el hilo principal (lo hace `main`).
    pub fn new() -> Self {
        let engine = create_engine();
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_child(Some(&engine.widget()));
        let diagnostic_script = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let scroll_byte = std::rc::Rc::new(std::cell::Cell::new(None));
        let pending_html = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let script = diagnostic_script.clone();
        let scroll = scroll_byte.clone();
        let pending = pending_html.clone();
        engine.connect_load_finished(std::rc::Rc::new(move |engine| {
            if let Some(html) = pending.borrow_mut().take() {
                engine.run_script(&update_script(&html));
            }
            engine.run_script(&script.borrow());
            if let Some((byte, follow)) = scroll.take() {
                engine.run_script(&scroll_script(byte, follow));
            }
        }));
        Self {
            scrolled,
            engine,
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
            if self.engine.is_loading() {
                *self.pending_html.borrow_mut() = Some(html.to_owned());
            } else {
                self.engine.run_script(&update_script(html));
            }
            return;
        }
        *self.document_key.borrow_mut() = Some(document_key);
        self.pending_html.borrow_mut().take();
        self.scroll_byte.set(None);
        self.engine.load_html(html, base_uri.unwrap_or(BASE_URI));
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

/// Script del scroll in-page a un destino `#fragmento` (S1, ISSUE-024,
/// revisión PR #44): `getElementById` + `scrollIntoView`, con el id pasado
/// por [`js_string`] (§7.2: nada del documento crudo en el script). `None`
/// si `destination` no es un fragmento (los enlaces a archivos quedan como
/// estaban: fuera de alcance, §3.4).
fn fragment_script(destination: &str) -> Option<String> {
    let id = destination.strip_prefix('#')?;
    if id.is_empty() {
        return None;
    }
    Some(format!(
        "{{ const target = document.getElementById({}); if (target) target.scrollIntoView({{block: 'center'}}); }}",
        js_string(id)
    ))
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

    /// S1/ISSUE-024 (revisión PR #44): un destino `#fragmento` genera el
    /// script de scroll in-page (`getElementById` + `scrollIntoView`, §26)
    /// con el id escapado (§7.2); lo que no es fragmento no genera script
    /// (los enlaces a archivos quedan como estaban, §3.4).
    #[test]
    fn fragmento_interno_genera_scroll_in_page() {
        let script = fragment_script("#destino").expect("fragmento con id");
        assert!(script.contains("getElementById"));
        assert!(script.contains("scrollIntoView"));
        assert!(
            script.contains(&js_string("destino")),
            "el id no viaja al getElementById: {script}"
        );
        // §7.2: un id hostil no puede romper la cadena del script.
        let hostil = fragment_script("#x\");alert(1);//").expect("fragmento hostil");
        assert!(!hostil.contains("alert"), "el id va escapado: {hostil}");
        // No-fragmento ⇒ sin script (fuera de alcance, §3.4).
        assert!(fragment_script("capitulo.md").is_none());
        assert!(fragment_script("capitulo.md#destino").is_none());
        assert!(fragment_script("#").is_none());
    }
}
