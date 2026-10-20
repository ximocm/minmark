//! Motor del Preview sobre WKWebView (macOS; ADR-005): la implementación de
//! [`super::PreviewEngine`] para Darwin, donde WebKitGTK 6.0 no es usable
//! (ADR-005). El transporte de HTML/JS, el veto de navegación, el ciclo de
//! carga y el puente de enlaces replican el comportamiento de `engine.rs`
//! (WebKitGTK); solo cambia quién aloja la vista web.
//!
//! # Composición GTK/AppKit (gate de integración documentado en ADR-005)
//!
//! El widget del trait es un [`gtk::Box`] subclaseado ([`PreviewHost`]) cuyos
//! vfuncs de GTK crean la `WKWebView` al `realize`, la ocultan/muestran en
//! `unmap`/`map` y la redimensionan en `size_allocate`, colgándola como
//! subvista `NSView` de la ventana nativa de GTK. Supuestos (todos solo
//! verificables en un Mac; si el enganche no puede hacerse se reintenta en
//! `map`/`size_allocate`, que son idempotentes, y se avisa por stderr):
//!
//! 1. `widget.native()` (GtkNative) comparte GObject con la ventana toplevel
//!    y `NativeExt::surface()` devuelve la `gdk::Surface` de esa ventana, que
//!    en macOS es una `gdk_macos::MacosSurface`.
//! 2. `MacosSurface::native()` (gdk4-macos con la feature `v4_8` →
//!    `gdk_macos_surface_get_native_window`, GTK ≥ 4.8 como fija ADR-005)
//!    devuelve el `NSWindow` subyacente. GTK advierte de que su `contentView`
//!    es un detalle de implementación y no publica una API para la NSView
//!    propia de cada widget, así que la `WKWebView` se engancha a
//!    `contentView` y se sitúa con `gtk_widget_compute_bounds` (bounds del
//!    widget en el espacio de la ventana toplevel = espacio de la superficie
//!    GDK: origen arriba-izquierda, en puntos).
//! 3. `contentView.bounds()` cubre exactamente la superficie GDK; el eje Y se
//!    convierte según `isFlipped()` en tiempo de ejecución (AppKit usa origen
//!    abajo-izquierda salvo vista flipped).
//! 4. GTK realiza la toplevel antes que a sus hijos, de modo que al `realize`
//!    del widget la `NSWindow` ya existe.
//!
//! §3.4: la página viaja en memoria con `loadHTMLString:baseURL:` (misma
//! semántica `base_uri` que `load_html` de `engine.rs`, normalmente
//! `file:///`), sin ficheros temporales y sin ningún recurso de red.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use block2::{DynBlock, RcBlock};
use gdk4_macos::MacosSurface;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use objc2::MainThreadMarker;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage, NSWindow,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString, NSURL};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationType, WKScriptMessage, WKScriptMessageHandler, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};

use super::PreviewEngine;

/// Nombre del canal `window.webkit.messageHandlers.<nombre>` del puente de
/// enlaces: idéntico al de `engine.rs` (el WebKit de ambas plataformas expone
/// el mismo camino de mensajes).
const LINK_HANDLER_NAME: &str = "minmarkLink";

/// Interceptador de clics del Preview: copia byte a byte del `UserScript` que
/// `engine.rs` inyecta con `webkit6` (misma semántica: `preventDefault` de
/// todo ancla, scroll propio para `#mm-note-` y `postMessage` para el resto).
/// `mod.rs` no expone un builder compartido, así que este texto puede derivar
/// respecto del de Linux si alguien cambia uno de los dos (riesgo de deriva
/// conocido; WebView2 usa además `window.chrome.webview.postMessage`).
const LINK_INTERCEPTION_SCRIPT: &str = "document.addEventListener('click', e => { const a = e.target.closest('a[href]'); if (a) { e.preventDefault(); const href = a.getAttribute('href'); if (href.startsWith('#mm-note-')) { document.getElementById(href.slice(1))?.scrollIntoView({block: 'center'}); } else { window.webkit.messageHandlers.minmarkLink.postMessage(href); } } });";

thread_local! {
    /// Puentes ObjC vivos → su estado. Los ivars del puente solo guardan un
    /// `u64` (tipo `Copy`), así que `define_class!` no necesita drenar estado
    /// Rust en el `dealloc`: la entrada se borra en `Drop for Inner`. Esto
    /// evita retener un `Rc` dentro de ivars (ciclo con el
    /// `WKUserContentController`, que retiene el handler con fuerza) y no
    /// depende de semántica de drop de ivars de objc2.
    static BRIDGES: RefCell<HashMap<u64, Weak<Inner>>> = RefCell::new(HashMap::new());
    static NEXT_BRIDGE_ID: Cell<u64> = const { Cell::new(1) };
}

/// Estado del motor, compartido entre el widget GTK ([`imp::PreviewHostObj`]),
/// los delegates ObjC y los handles [`WkwebviewEngine`].
struct Inner {
    /// Identidad en [`BRIDGES`] (compartida con los ivars del puente).
    id: u64,
    /// Widget anfitrión como ref débil: el ciclo fuerte
    /// host → imp → `Rc<Inner>` no debe cerrarse con host → `Rc<Inner>`.
    host: RefCell<Option<glib::object::WeakRef<PreviewHost>>>,
    /// La vista nativa; `None` hasta el `realize` y tras el `unrealize`.
    webview: RefCell<Option<Retained<WKWebView>>>,
    /// `WKUserContentController` de la configuración de la vista (compartida
    /// con ella por referencia) para instalar el puente de enlaces.
    content: RefCell<Option<Retained<WKUserContentController>>>,
    /// Delegate de navegación y de mensajes; lo retiene solo este motor
    /// (`navigationDelegate` es débil en WKWebView).
    bridge: RefCell<Option<Retained<PreviewBridge>>>,
    /// Último documento pedido: se (re)carga al crear la vista y al volver a
    /// realizarla, porque `PreviewPane` no vuelve a llamar a `load_html` si la
    /// clave del documento no cambia.
    document: RefCell<Option<(String, String)>>,
    /// Scripts pedidos antes de que exista la vista (disparo y olvida).
    pending_scripts: RefCell<Vec<String>>,
    /// Aviso de carga terminada registrado por `PreviewPane` (orden: el panel
    /// aplica aquí actualización pendiente + diagnósticos + scroll).
    load_finished: RefCell<Option<Rc<dyn Fn(&dyn PreviewEngine)>>>,
    /// Activación de enlaces registrada por `PreviewPane`.
    link_activation: RefCell<Option<Rc<dyn Fn(String)>>>,
    /// `block_navigation` activo (el veto se aplica en la política de
    /// navegación; el delegate existe siempre porque gestiona la carga).
    block_navigation: Cell<bool>,
    /// `true` desde `load_html` hasta `webView:didFinishNavigation:`. Es
    /// síncrono (a diferencia de `isLoading` de WebKit), de modo que también
    /// cubre la carga encolada antes del `realize`. `didFail` no lo baja: los
    /// fallos de navegación cancelada (al recargar) llegarían con él y
    /// pondrían el panel en peligro de perder el HTML pendiente; las cargas
    /// locales de `loadHTMLString` no fallan en la práctica.
    loading: Cell<bool>,
    /// El puente de enlaces ya está instalado en `content`.
    bridge_installed: Cell<bool>,
}

impl Inner {
    /// Carga `html` con `base_uri` como URI base (§3.4). Sin vista creada el
    /// documento queda encolado en `document` y se carga en el `realize`.
    fn load_html(&self, html: &str, base_uri: &str) {
        *self.document.borrow_mut() = Some((html.to_owned(), base_uri.to_owned()));
        self.loading.set(true);
        let view = self.webview.borrow().clone();
        if let Some(view) = view {
            self.load_document(&view, html, base_uri);
        }
    }

    /// `loadHTMLString:baseURL:`: transporte en memoria con la misma semántica
    /// que `load_html` de WebKitGTK (`base_uri` normalmente `file:///`). Se
    /// elige este camino, y no un fichero temporal `loadFileURL`, porque el
    /// contrato del trait es «HTML con URI base» y así no hay I/O de disco ni
    /// posibilidad de salirse a la red (§3.4).
    fn load_document(&self, view: &WKWebView, html: &str, base_uri: &str) {
        let html = NSString::from_str(html);
        let base = NSURL::URLWithString(&NSString::from_str(base_uri));
        let _ = unsafe { view.loadHTMLString_baseURL(&html, base.as_deref()) };
    }

    /// Evalúa `script` sin esperar resultado; sin vista se encola hasta el
    /// `realize` (después se descarga contra el documento en blanco, igual que
    /// haría WebKitGTK antes de cargar nada).
    fn run_script(&self, script: &str) {
        let view = self.webview.borrow().clone();
        match view {
            Some(view) => evaluate(&view, script, None),
            None => self.pending_scripts.borrow_mut().push(script.to_owned()),
        }
    }

    /// Registra el callback de activación de enlaces e instala el puente si la
    /// vista ya existe (si no, se instala al crearla; `PreviewPane` llama
    /// antes de mostrar la ventana, igual que en Linux).
    fn connect_link_activation(&self, activate: Rc<dyn Fn(String)>) {
        *self.link_activation.borrow_mut() = Some(activate);
        self.install_bridge_if_needed();
    }

    /// Crea la `WKWebView` (si aún no existe) y la engancha a la jerarquía
    /// `NSView` de la ventana nativa de GTK. Idempotente: es el reintente del
    /// enganche inicial.
    fn ensure_attached(&self, host: &PreviewHost) {
        if self.webview.borrow().is_some() {
            return;
        }
        let Some(window) = gtk_ns_window(host) else {
            eprintln!("minmark: WKWebView sin NSWindow de GTK; se reintenta en map/size-allocate");
            return;
        };
        let Some(content) = window.contentView() else {
            eprintln!(
                "minmark: WKWebView sin contentView de GTK; se reintenta en map/size-allocate"
            );
            return;
        };

        let mtm = MainThreadMarker::new()
            .expect("minmark: la WKWebView debe crearse en el hilo principal");
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let controller = unsafe { configuration.userContentController() };
        let web_view = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), CGRect::ZERO, &configuration)
        };

        // El mismo puente objeta la política de navegación (veto + fin de
        // carga) y el canal de mensajes de enlaces.
        let bridge = PreviewBridge::new(self.id, mtm);
        unsafe { web_view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*bridge))) };
        // Nace oculta: el toplevel todavía no ha hecho `map`.
        web_view.setHidden(true);
        content.addSubview(&web_view);

        *self.content.borrow_mut() = Some(controller);
        *self.bridge.borrow_mut() = Some(bridge);
        *self.webview.borrow_mut() = Some(web_view.clone());

        self.install_bridge_if_needed();

        let document = self.document.borrow().clone();
        if let Some((html, base_uri)) = document {
            self.loading.set(true);
            self.load_document(&web_view, &html, &base_uri);
        }
        let queued = std::mem::take(&mut *self.pending_scripts.borrow_mut());
        for script in &queued {
            evaluate(&web_view, script, None);
        }
        self.layout(host);
    }

    /// Desmonta la vista al `unrealize` (la ventana nativa va a desaparecer).
    fn detach(&self) {
        let view = self.webview.borrow_mut().take();
        if let Some(view) = view {
            unsafe { view.setNavigationDelegate(None) };
            view.removeFromSuperview();
        }
        *self.bridge.borrow_mut() = None;
        *self.content.borrow_mut() = None;
        // Los scripts de usuario viven en el controller de la vista muerta:
        // hay que reinstalarlos en la próxima.
        self.bridge_installed.set(false);
    }

    /// Ubica la vista sobre el widget según su allocation actual.
    fn layout(&self, host: &PreviewHost) {
        let view = self.webview.borrow().clone();
        let Some(view) = view else { return };
        place_view(&view, host);
    }

    /// Muestra/oculta la vista con el mapa del widget (WKWebView sin window no
    /// pinta, y al desmapear no debe quedar una vista nativa encima de GTK).
    fn set_mapped(&self, mapped: bool) {
        if !mapped {
            let view = self.webview.borrow().clone();
            if let Some(view) = view {
                view.setHidden(true);
            }
            return;
        }
        // Reintente del enganche por si en `realize` aún no había NSWindow.
        let host = self
            .host
            .borrow()
            .as_ref()
            .and_then(glib::object::WeakRef::upgrade);
        if let Some(host) = host {
            self.ensure_attached(&host);
        }
        let view = self.webview.borrow().clone();
        if let Some(view) = view {
            view.setHidden(false);
        }
    }

    /// Instala `minmarkLink` (handler + script de usuario) la primera vez que
    /// haya callback y controller. Quitar el handler antes de añadirlo evita
    /// la excepción ObjC por nombre duplicado.
    fn install_bridge_if_needed(&self) {
        if self.bridge_installed.get() || self.link_activation.borrow().is_none() {
            return;
        }
        let controller = self.content.borrow().clone();
        let bridge = self.bridge.borrow().clone();
        let (Some(controller), Some(bridge)) = (controller, bridge) else {
            return;
        };
        install_link_bridge(&controller, &bridge);
        self.bridge_installed.set(true);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        BRIDGES.with(|map| {
            map.borrow_mut().remove(&self.id);
        });
    }
}

/// Estado del widget anfitrión; lo reparten el objeto GTK y el motor.
mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PreviewHostObj {
        /// Se fija una sola vez desde `WkwebviewEngine::new`.
        pub inner: OnceCell<Rc<Inner>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PreviewHostObj {
        const NAME: &'static str = "MinmarkPreviewHost";
        type Type = super::PreviewHost;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for PreviewHostObj {}

    // Requerido por `ParentType = gtk::Box`: glib exige
    // `gtk::Box: IsSubclassable<PreviewHostObj>` y esa impl solo existe para
    // `T: BoxImpl` (paridad con `imp::Host` de `engine_webview2.rs`).
    impl BoxImpl for PreviewHostObj {}

    impl WidgetImpl for PreviewHostObj {
        fn realize(&self) {
            self.parent_realize();
            // GTK garantiza que la toplevel (y su NSWindow) está realizada
            // antes que el hijo; si aun así faltara, `ensure_attached` se
            // reintenta desde `map`/`size_allocate`.
            if let Some(inner) = self.inner.get() {
                inner.ensure_attached(&self.obj());
            }
        }

        fn unrealize(&self) {
            if let Some(inner) = self.inner.get() {
                inner.detach();
            }
            self.parent_unrealize();
        }

        fn map(&self) {
            self.parent_map();
            if let Some(inner) = self.inner.get() {
                inner.set_mapped(true);
            }
        }

        fn unmap(&self) {
            if let Some(inner) = self.inner.get() {
                inner.set_mapped(false);
            }
            self.parent_unmap();
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(inner) = self.inner.get() {
                inner.layout(&self.obj());
            }
        }
    }
}

glib::wrapper! {
    /// Widget anfitrión del Preview: un `gtk::Box` sin hijos GTK (así mide 0×0
    /// y el `ScrolledWindow` le da el viewport, con el scroll real dentro de
    /// la `WKWebView`); la vista nativa se superpone desde los vfuncs de
    /// [`imp::PreviewHostObj`].
    struct PreviewHost(ObjectSubclass<imp::PreviewHostObj>) @extends gtk::Box, gtk::Widget;
}

/// Motor del Preview sobre WKWebView: comparte [`Inner`] con el widget
/// anfitrión y con los delegates ObjC. Un handle es barato de clonar
/// ([`Rc`]) y siempre apunta al mismo motor.
pub(super) struct WkwebviewEngine {
    host: PreviewHost,
    inner: Rc<Inner>,
}

impl WkwebviewEngine {
    /// Crea la [`gtk::Box`] anfitriona y su estado. La `WKWebView` nativa no
    /// se crea aquí, sino al `realize` del widget, y todo lo pedido antes
    /// (`load_html`/`run_script`/callbacks) queda encolado hasta entonces.
    ///
    /// Requiere GTK inicializado en el hilo principal (lo hace `main`).
    pub(super) fn new() -> Self {
        let inner = new_state();
        let host: PreviewHost = glib::Object::new();
        let _ = host.imp().inner.set(inner.clone());
        *inner.host.borrow_mut() = Some(host.downgrade());
        Self { host, inner }
    }

    /// Handle al mismo motor para los callbacks del trait (como hace
    /// `engine.rs`: el callback de carga recibe un motor con el que evaluar).
    fn handle(inner: &Rc<Inner>) -> Option<Self> {
        let host = inner.host.borrow();
        let host = host.as_ref()?.upgrade()?;
        Some(Self {
            host,
            inner: inner.clone(),
        })
    }
}

impl PreviewEngine for WkwebviewEngine {
    fn widget(&self) -> gtk::Widget {
        self.host.clone().upcast::<gtk::Widget>()
    }

    fn load_html(&self, html: &str, base_uri: &str) {
        self.inner.load_html(html, base_uri);
    }

    fn run_script(&self, script: &str) {
        self.inner.run_script(script);
    }

    fn is_loading(&self) -> bool {
        self.inner.loading.get()
    }

    fn connect_load_finished(&self, callback: Rc<dyn Fn(&dyn PreviewEngine)>) {
        *self.inner.load_finished.borrow_mut() = Some(callback);
    }

    fn connect_link_activation(&self, activate: Rc<dyn Fn(String)>) {
        self.inner.connect_link_activation(activate);
    }

    fn block_navigation(&self) {
        self.inner.block_navigation.set(true);
    }

    #[cfg(test)]
    fn snapshot(&self, path: &std::path::Path, done: Rc<Cell<bool>>) {
        let view = self.inner.webview.borrow().clone();
        let Some(view) = view else {
            eprintln!("minmark: snapshot sin WKWebView (widget sin realizar)");
            return;
        };
        let target = path.to_owned();
        let flag = done;
        // `takeSnapshotWithConfiguration` con configuración nil captura el
        // área visible de la vista (WebKitGTK captura el documento completo;
        // WKWebView no expone captura «full document», diferencia conocida de
        // la evidencia de QA en macOS).
        let block: RcBlock<dyn Fn(*mut NSImage, *mut NSError)> =
            RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                if image.is_null() || !error.is_null() {
                    eprintln!("minmark: takeSnapshot sin imagen");
                    return;
                }
                let image = unsafe { &*image };
                let Some(bytes) = png_bytes(image) else {
                    eprintln!("minmark: takeSnapshot sin codificación PNG");
                    return;
                };
                match std::fs::write(&target, bytes) {
                    Ok(()) => flag.set(true),
                    Err(write_error) => eprintln!("minmark: snapshot: {write_error}"),
                }
            });
        unsafe { view.takeSnapshotWithConfiguration_completionHandler(None, &block) };
    }

    #[cfg(test)]
    fn eval_ok(&self, script: &str, done: Rc<Cell<bool>>) {
        let view = self.inner.webview.borrow().clone();
        let Some(view) = view else {
            eprintln!("minmark: eval sin WKWebView (widget sin realizar)");
            return;
        };
        let flag = done;
        // Paridad con `engine.rs`: la evaluación termina sin error ⇒ true.
        // Un fallo no panic dentro del bloque (paniquear atraviesa frames
        // ObjC); `done` queda false y la prueba lo detecta por su espera.
        let block: RcBlock<dyn Fn(*mut AnyObject, *mut NSError)> =
            RcBlock::new(move |_value: *mut AnyObject, error: *mut NSError| {
                flag.set(error.is_null());
            });
        evaluate(&view, script, Some(&block));
    }

    #[cfg(test)]
    fn eval_number(&self, script: &str, result: Rc<Cell<Option<f64>>>) {
        let view = self.inner.webview.borrow().clone();
        let Some(view) = view else {
            eprintln!("minmark: eval sin WKWebView (widget sin realizar)");
            return;
        };
        let cell = result;
        // Paridad con `engine.rs`: número ⇒ Some(valor); error ⇒ panic allí y
        // None aquí (ver comentario de `eval_ok`).
        let block: RcBlock<dyn Fn(*mut AnyObject, *mut NSError)> =
            RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                if !error.is_null() || value.is_null() {
                    cell.set(None);
                    return;
                }
                let number = unsafe { &*value }.downcast_ref::<NSNumber>();
                cell.set(number.map(|number| number.doubleValue()));
            });
        evaluate(&view, script, Some(&block));
    }
}

/// Crea el estado y lo registra en [`BRIDGES`] bajo una id nueva.
fn new_state() -> Rc<Inner> {
    let id = NEXT_BRIDGE_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    let inner = Rc::new(Inner {
        id,
        host: RefCell::new(None),
        webview: RefCell::new(None),
        content: RefCell::new(None),
        bridge: RefCell::new(None),
        document: RefCell::new(None),
        pending_scripts: RefCell::new(Vec::new()),
        load_finished: RefCell::new(None),
        link_activation: RefCell::new(None),
        block_navigation: Cell::new(false),
        loading: Cell::new(false),
        bridge_installed: Cell::new(false),
    });
    BRIDGES.with(|map| {
        map.borrow_mut().insert(id, Rc::downgrade(&inner));
    });
    inner
}

/// Estado asociado al puente ObjC; `None` si el motor ya se destruyó (los
/// callbacks en vuelo hacen la acción por defecto: permitir / ignorar).
fn bridge_inner(bridge: &PreviewBridge) -> Option<Rc<Inner>> {
    BRIDGES.with(|map| {
        map.borrow()
            .get(&bridge.ivars().inner_id)
            .and_then(Weak::upgrade)
    })
}

/// Evalúa `script`; `handler` opcional para las variantes de prueba. Disparo
/// y olvida en el camino normal (`None`), igual que `evaluate_javascript` de
/// `engine.rs`.
fn evaluate(
    view: &WKWebView,
    script: &str,
    handler: Option<&DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
) {
    let script = NSString::from_str(script);
    unsafe { view.evaluateJavaScript_completionHandler(&script, handler) };
}

/// `NSWindow` nativa de GTK para el widget: GtkNative (la toplevel) → su
/// superficie GDK → `gdk_macos_surface_get_native_window` (feature `v4_8` de
/// gdk4-macos). Ver supuestos 1/2/4 de la cabecera; el puntero lo posee GDK,
/// no se retiene aquí.
fn gtk_ns_window(host: &PreviewHost) -> Option<&NSWindow> {
    let native = host.native()?;
    let surface = native.surface()?;
    let macos = surface.downcast_ref::<MacosSurface>()?;
    let pointer = macos.native();
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { &*(pointer as *mut NSWindow) })
}

/// Fija el frame de la `WKWebView` al allocation actual del widget (espacio
/// de la superficie GDK → espacio de `contentView`, con comprobación de
/// `isFlipped()`; ver supuestos 2/3 de la cabecera).
fn place_view(view: &WKWebView, host: &PreviewHost) {
    let Some(native) = host.native() else {
        return;
    };
    let Some(bounds) = host.compute_bounds(&native) else {
        return;
    };
    let Some(window) = gtk_ns_window(host) else {
        return;
    };
    let Some(content) = window.contentView() else {
        return;
    };
    let x = f64::from(bounds.x());
    let y = f64::from(bounds.y());
    let width = f64::from(bounds.width());
    let height = f64::from(bounds.height());
    let origin_y = if content.isFlipped() {
        y
    } else {
        content.bounds().size.height - y - height
    };
    view.setFrame(CGRect::new(
        CGPoint::new(x, origin_y),
        CGSize::new(width, height),
    ));
}

/// Política de navegación (paridad con `block_navigation` de `engine.rs`):
/// sin veto, todo pasa; con veto se cancela la navegación de ventana nueva y
/// los clics de enlace que salen del documento, dejando pasar las cargas
/// propias (`loadHTMLString`, tipo `Other`) y los fragmentos de la misma
/// página. El `decisionHandler` se llama siempre exactamente una vez.
fn navigation_policy(
    view: &WKWebView,
    action: &WKNavigationAction,
    veto: bool,
) -> WKNavigationActionPolicy {
    if !veto {
        return WKNavigationActionPolicy::Allow;
    }
    // `targetFrame` nil ⇒ navegación a ventana/frame nuevo: fuera.
    if unsafe { action.targetFrame() }.is_none() {
        return WKNavigationActionPolicy::Cancel;
    }
    // Paridad con engine.rs: solo los enlaces activados que no son
    // same-document se cancelan (allí se ignora `NavigationType::LinkClicked`).
    if unsafe { action.navigationType() } != WKNavigationType::LinkActivated {
        return WKNavigationActionPolicy::Allow;
    }
    if same_document(view, action) {
        WKNavigationActionPolicy::Allow
    } else {
        WKNavigationActionPolicy::Cancel
    }
}

/// ¿La navegación pedida apunta al documento actual (mismo recurso, quizá con
/// otro `#fragmento`)? Compara la URL del `request` con la URL comprometida
/// ignorando el fragmento.
fn same_document(view: &WKWebView, action: &WKNavigationAction) -> bool {
    let request = unsafe { action.request() };
    let Some(next) = request.URL() else {
        return false;
    };
    let current = unsafe { view.URL() };
    let Some(current) = current else {
        return false;
    };
    let (Some(next), Some(current)) = (next.absoluteString(), current.absoluteString()) else {
        return false;
    };
    let next = ns_to_string(&next);
    let current = ns_to_string(&current);
    let next = document_part(&next);
    let current = document_part(&current);
    !next.is_empty() && next == current
}

/// Recurso del URL sin `#fragmento`.
fn document_part(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// `NSString` → `String` seguro dentro de una autorelease pool.
fn ns_to_string(value: &NSString) -> String {
    autoreleasepool(|pool| unsafe { value.to_str(pool) }.to_owned())
}

/// Instala handler `minmarkLink` y script de usuario (inyección al final del
/// documento y solo en el marco principal, como `engine.rs`) en el controller
/// compartido con la vista.
fn install_link_bridge(controller: &WKUserContentController, bridge: &PreviewBridge) {
    // `WKUserScript::alloc` exige el marcador de hilo principal, igual que la
    // propia vista: el puente solo se instala desde el hilo de GTK.
    let mtm = MainThreadMarker::new()
        .expect("minmark: el puente de enlaces debe instalarse en el hilo principal");
    let name = NSString::from_str(LINK_HANDLER_NAME);
    // Quitar antes de añadir: repetir el nombre lanza una excepción ObjC.
    unsafe { controller.removeScriptMessageHandlerForName(&name) };
    unsafe { controller.addScriptMessageHandler_name(&ProtocolObject::from_ref(bridge), &name) };
    let script = NSString::from_str(LINK_INTERCEPTION_SCRIPT);
    let user_script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            WKUserScript::alloc(mtm),
            &script,
            WKUserScriptInjectionTime::AtDocumentEnd,
            true,
        )
    };
    unsafe { controller.addUserScript(&user_script) };
}

/// Codifica la captura como PNG (NSImage → TIFF → bitmap → PNG) y devuelve los
/// bytes; `None` si alguna conversión falla.
fn png_bytes(image: &NSImage) -> Option<Vec<u8>> {
    let tiff = image.TIFFRepresentation()?;
    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let properties: Retained<NSDictionary<NSBitmapImageRepPropertyKey, AnyObject>> =
        NSDictionary::from_slices(&[] as &[&NSString], &[] as &[&AnyObject]);
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }?;
    Some(png.to_vec())
}

/// Delegate ObjC del motor: política de navegación, fin de carga y canal de
/// mensajes de enlaces. Solo guarda su identidad en [`BRIDGES`] (ivars
/// `Copy`), de ahí que la clase no necesite `Drop`.
define_class!(
    // SAFETY:
    // - El superclase NSObject no impone requisitos de subclase.
    // - `PreviewBridge` no implementa `Drop` (sus ivars son un `u64`).
    // - Los métodos de protocolo replican las firmas declaradas por objc2
    //   (`unsafe fn`), en el hilo principal (GTK y WebKit son main-thread).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BridgeIvars]
    struct PreviewBridge;

    unsafe impl NSObjectProtocol for PreviewBridge {}

    unsafe impl WKNavigationDelegate for PreviewBridge {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn webView_decidePolicyForNavigationAction_decisionHandler(
            &self,
            web_view: &WKWebView,
            navigation_action: &WKNavigationAction,
            decision_handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let veto = bridge_inner(self)
                .map(|inner| inner.block_navigation.get())
                .unwrap_or(false);
            let policy = navigation_policy(web_view, navigation_action, veto);
            decision_handler.call((policy,));
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        unsafe fn webView_didFinishNavigation(
            &self,
            _web_view: &WKWebView,
            _navigation: Option<&WKNavigation>,
        ) {
            let Some(inner) = bridge_inner(self) else {
                return;
            };
            inner.loading.set(false);
            // Callback y handle clonados antes de invocar: el panel puede
            // volver a entrar en el motor (run_script) sin RefCell prestado.
            let callback = inner.load_finished.borrow().clone();
            let Some(handle) = WkwebviewEngine::handle(&inner) else {
                return;
            };
            if let Some(callback) = callback {
                callback(&handle);
            }
        }
    }

    unsafe impl WKScriptMessageHandler for PreviewBridge {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        unsafe fn userContentController_didReceiveScriptMessage(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            let Some(inner) = bridge_inner(self) else {
                return;
            };
            let body = message.body();
            let Some(href) = body.downcast_ref::<NSString>() else {
                return;
            };
            let href = autoreleasepool(|pool| unsafe { href.to_str(pool) }.to_owned());
            let callback = inner.link_activation.borrow().clone();
            if let Some(callback) = callback {
                callback(href);
            }
        }
    }
);

/// Ivars del puente: solo la identidad en [`BRIDGES`] (tipo `Copy`, sin
/// estado Rust que drenar en `dealloc`).
struct BridgeIvars {
    inner_id: u64,
}

impl PreviewBridge {
    /// Instancia e inicializa el puente con la id de su [`Inner`]. Como toda
    /// clase `MainThreadOnly`, `alloc` exige el [`MainThreadMarker`].
    fn new(inner_id: u64, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BridgeIvars { inner_id });
        unsafe { msg_send![super(this), init] }
    }
}
