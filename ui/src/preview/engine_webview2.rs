//! Motor del Preview sobre WebView2 en Windows (ADR-005): la implementación
//! de [`super::PreviewEngine`] para el sistema operativo Windows, mientras
//! Linux usa WebKitGTK (`engine`) y macOS usa WKWebView (`engine_wkwebview`).
//!
//! Este módulo es el único punto del `ui` que puede nombrar `windows`,
//! `webview2-com` o `gdk4-win32`. El HTML y los scripts ya son agnósticos
//! del motor en `super`, así que aquí solo vive el alojamiento nativo.
//!
//! **Gate de integración: GTK + HWND.** El widget GTK no tiene ventana
//! propia: GTK4 dibuja toda la jerarquía en la GdkSurface del toplevel. El
//! WebView2 vive como HWND hijo de ese toplevel (`Win32Surface::handle`),
//! posicionado con `ICoreWebView2Controller::Bounds` en coordenadas del área
//! cliente de la superficie. Supuestos conocidos y no verificables en Linux
//! (QA manual en Windows, ADR-005):
//!
//! - La posición del widget en la surface se obtiene con `compute_bounds`
//!   sobre la `gtk::Window` raíz menos `Native::surface_transform` (márgenes
//!   CSD de la ventana); si GTK4-Win32 altera ese convenio, el HWND hijo se
//!   desalineará del todo.
//! - `size_allocate` cubre también el scroll del `ScrolledWindow` (§10.2):
//!   al reubicar el hijo, GTK realoca y volvemos a fijar Bounds.
//! - GTK usa coordenadas lógicas; Bounds de WebView2 exige píxeles físicos.
//!   Se aplica la escala de la superficie, también con DPI fraccionario.
//! - El HWND hijo se recorta al área cliente del padre; una caja GTK fuera
//!   del viewport no debe tapar al resto de la UI (se fija Bounds vacío).
//! - El foco GTK↔WebView2 (MoveFocus / WM_SETFOCUS) queda pendiente de QA;
//!   el trait `PreviewEngine` no lo expone todavía.
//!
//! **Hilos:** WebView2 entrega sus callbacks en el hilo que creó el entorno,
//! colgando de la cola de mensajes Win32 de ese hilo. El backend Win32 de
//! GTK4 bombea mensajes en su bucle principal, así que los callbacks caen en
//! el hilo principal de GTK (donde vive todo el estado de este motor, sin
//! bloqueos). `CoInitializeEx(COINIT_APARTMENTTHREADED)` se pide al realizar
//! el widget; `RPC_E_CHANGED_MODE` se tolera.
//!
//! **Toolchain y CI:** MSVC + GTK4 de gvsbuild (primera opción con
//! WebView2, ADR-005). El job de Windows debe instalar el toolset MSVC,
//! ejecutar gvsbuild con GTK 4, y contar con el runtime WebView2 Evergreen
//! (ya presente en los runners; los paquetes de release añaden el instalador
//! offline).
//!
//! **Carga:** nunca `NavigateToString` (límite de 2 MiB, ADR-005): el HTML
//! viaja a un archivo temporal local y se navega a su `file:///`
//! percent-escapado.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gdk4_win32::Win32Surface;
use gtk::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CapturePreviewCompletedHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    WebMessageReceivedEventHandler,
};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{
    CoInitializeEx, IStream, COINIT_APARTMENTTHREADED, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use super::PreviewEngine;

/// `ICoreWebView2` y compañía solo se tocan en el hilo principal, así que el
/// estado compartido no necesita `Send`/`Sync`: `Rc` + celdas basta.
struct Shared {
    /// Referencia débil al widget anfitrión (fuerte = ciclo con el `Rc`).
    host: RefCell<glib::WeakRef<Webview2Host>>,
    /// HWND del toplevel GDK; 0 mientras no esté realizado el widget.
    parent_hwnd: Cell<isize>,
    /// Entorno/controlador ya en marcha (o en proceso de creación).
    started: Cell<bool>,
    controller: RefCell<Option<ICoreWebView2Controller>>,
    webview: RefCell<Option<ICoreWebView2>>,
    is_loading: Cell<bool>,
    /// Visibilidad deseada (map/unmap del widget) aplicada al controlador.
    visible: Cell<bool>,
    block_navigation: Cell<bool>,
    /// Única URI permitida a navegación: la del temporal de `load_html`.
    allowed_uri: RefCell<String>,
    /// Carga pedida antes de que el controlador esté listo.
    pending_uri: RefCell<Option<String>>,
    /// Scripts ejecutados antes de que el controlador esté listo.
    pending_scripts: RefCell<Vec<String>>,
    load_finished: RefCell<Option<Rc<dyn Fn(&dyn PreviewEngine)>>>,
    link_activation: RefCell<Option<Rc<dyn Fn(String)>>>,
    temp_file: RefCell<Option<PathBuf>>,
    /// Últimos Bounds aplicados (evita llamadas a COM en cada frame).
    last_bounds: Cell<(i32, i32, i32, i32)>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            host: RefCell::new(glib::WeakRef::default()),
            parent_hwnd: Cell::new(0),
            started: Cell::new(false),
            controller: RefCell::new(None),
            webview: RefCell::new(None),
            is_loading: Cell::new(false),
            visible: Cell::new(false),
            block_navigation: Cell::new(false),
            allowed_uri: RefCell::new(String::new()),
            pending_uri: RefCell::new(None),
            pending_scripts: RefCell::new(Vec::new()),
            load_finished: RefCell::new(None),
            link_activation: RefCell::new(None),
            temp_file: RefCell::new(None),
            last_bounds: Cell::new((0, 0, 0, 0)),
        }
    }
}

impl Shared {
    /// Crea el entorno WebView2 al realizar el widget, con el HWND del
    /// toplevel GDK como padre. Asíncrono: todo lo pedido mientras el
    /// controlador no existe queda encolado en `pending_*`.
    fn start(shared: &Rc<Shared>, host: &Webview2Host) {
        if shared.started.get() {
            return;
        }
        // El hilo principal de GTK aún no será COM; si ya lo fuera en modo
        // distinto, el error RPC_E_CHANGED_MODE es inofensivo.
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let Some(surface) = host.native().and_then(|native| native.surface()) else {
            eprintln!(
                "minmark: WebView2: sin surface GDK todavía; se reintenta en map/size_allocate"
            );
            return;
        };
        let Ok(win32_surface) = surface.downcast::<Win32Surface>() else {
            eprintln!("minmark: WebView2: la surface del toplevel no es Win32; el Preview no se presentará");
            return;
        };
        let parent = win32_surface.handle();
        if parent.0 == 0 {
            eprintln!(
                "minmark: WebView2: HWND del toplevel nulo; se reintenta en map/size_allocate"
            );
            return;
        }
        shared.parent_hwnd.set(parent.0);
        shared.started.set(true);
        Shared::create_environment(shared, HWND(parent.0 as *mut _));
    }

    fn create_environment(shared: &Rc<Shared>, parent: HWND) {
        let shared_env = shared.clone();
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let (Ok(()), Some(environment)) = (result, environment) else {
                    eprintln!(
                        "minmark: WebView2: entorno no disponible \
                         (¿runtime WebView2 Evergreen instalado?)"
                    );
                    shared_env.started.set(false);
                    return Ok(());
                };
                Shared::create_controller(&shared_env, parent, &environment);
                Ok(())
            },
        ));
        // Carpeta de datos del usuario EXPLÍCITA (ISSUE-57): la predeterminada
        // de WebView2 es `<ejecutable>.WebView2` junto al .exe, y el MSI
        // `perMachine` instala en `Program Files`, donde un usuario normal no
        // puede escribir: el entorno no se creaba y la preview quedaba en
        // blanco para siempre. El directorio de datos por usuario del core es
        // escribible en los tres SO.
        let user_data: Option<PathBuf> =
            minmark_config::dirs::data_dir().map(|dir| dir.join("minmark").join("webview2"));
        let user_data_string = user_data
            .as_ref()
            .map(|path| HSTRING::from(path.to_string_lossy().as_ref()));
        let user_data_ptr = user_data_string.as_ref().map(|value| value.as_ptr());
        if let Some(path) = &user_data {
            let _ = std::fs::create_dir_all(path);
        } else {
            eprintln!(
                "minmark: WebView2: sin directorio de datos por usuario; se usa el del \
                 ejecutable, que fallará si no es escribible (p. ej. Program Files)"
            );
        }
        let created = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                user_data_ptr.map_or_else(PCWSTR::null, PCWSTR),
                None::<&ICoreWebView2EnvironmentOptions>,
                &handler,
            )
        };
        if let Err(error) = created {
            eprintln!("minmark: WebView2: CreateCoreWebView2EnvironmentWithOptions: {error}");
            shared.started.set(false);
        }
    }

    fn create_controller(
        shared: &Rc<Shared>,
        parent: HWND,
        environment: &ICoreWebView2Environment,
    ) {
        let shared_controller = shared.clone();
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                let (Ok(()), Some(controller)) = (result, controller) else {
                    eprintln!("minmark: WebView2: controlador no disponible");
                    shared_controller.started.set(false);
                    return Ok(());
                };
                let webview = match unsafe { controller.CoreWebView2() } {
                    Ok(webview) => webview,
                    Err(error) => {
                        eprintln!("minmark: WebView2: CoreWebView2: {error}");
                        return Ok(());
                    }
                };
                *shared_controller.controller.borrow_mut() = Some(controller);
                *shared_controller.webview.borrow_mut() = Some(webview.clone());
                Shared::install_handlers(&shared_controller, &webview);
                // El script de usuario debe quedar registrado antes de la
                // primera navegación; la carga pendiente se lanza en `ready`.
                Shared::add_link_script(&shared_controller, &webview);
                Ok(())
            },
        ));
        if let Err(error) = unsafe { environment.CreateCoreWebView2Controller(parent, &handler) } {
            eprintln!("minmark: WebView2: CreateCoreWebView2Controller: {error}");
            shared.started.set(false);
        }
    }

    /// Puentes nativos del trait: veto de navegación, fin de carga y canal
    /// `minmarkLink` (paridad con `connect_decide_policy`, `load_changed` y
    /// el message handler de `engine.rs`).
    fn install_handlers(shared: &Rc<Shared>, webview: &ICoreWebView2) {
        let mut token: i64 = 0;

        let navigation = shared.clone();
        let _ = unsafe {
            webview.add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    if args.Uri(&mut uri).is_err() {
                        return Ok(());
                    }
                    let uri = take_pwstr(uri);
                    let allowed = {
                        let current = navigation.allowed_uri.borrow();
                        !current.is_empty() && current.eq_ignore_ascii_case(&uri)
                    };
                    // §3.4: la WebView conserva su documento; solo se permite
                    // navegar al propio temporal. Los cambios de fragmento no
                    // disparan NavigationStarting, así que el scroll in-page
                    // (#mm-note-, §26) sigue funcionando.
                    if navigation.block_navigation.get() && !allowed {
                        let _ = args.SetCancel(true);
                        return Ok(());
                    }
                    navigation.is_loading.set(true);
                    Ok(())
                })),
                &mut token,
            )
        };

        let completion = shared.clone();
        let _ = unsafe {
            webview.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                    completion.is_loading.set(false);
                    let success = args.as_ref().is_some_and(|args| {
                        let mut is_success: BOOL = false.into();
                        args.IsSuccess(&mut is_success).is_ok() && is_success.as_bool()
                    });
                    if !success {
                        return Ok(());
                    }
                    let host = completion.host.borrow().upgrade();
                    let callback = completion.load_finished.borrow().clone();
                    if let (Some(callback), Some(host)) = (callback, host) {
                        // El callback recibe un handle nuevo del mismo motor
                        // (misma técnica que WebkitEngine) para aplicar sus
                        // scripts pendientes con el mismo orden que en Linux:
                        // actualización diferida, diagnóstico y scroll.
                        callback(&Webview2Engine {
                            host,
                            shared: completion.clone(),
                        });
                    }
                    Ok(())
                })),
                &mut token,
            )
        };

        let messages = shared.clone();
        let _ = unsafe {
            webview.add_WebMessageReceived(
                &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut href = PWSTR::null();
                    if args.TryGetWebMessageAsString(&mut href).is_err() {
                        return Ok(());
                    }
                    let href = take_pwstr(href);
                    if let Some(activate) = messages.link_activation.borrow().as_ref() {
                        activate(href);
                    }
                    Ok(())
                })),
                &mut token,
            )
        };
    }

    /// Script inicial equivalente al `UserScript` de `engine.rs` (fin de la
    /// fase de carga del documento): los clics de anclas se interceptan y el
    /// href viaja por `window.chrome.webview.postMessage`, la contraparte de
    /// `window.webkit.messageHandlers.minmarkLink` en este backend.
    fn add_link_script(shared: &Rc<Shared>, webview: &ICoreWebView2) {
        let script = HSTRING::from(link_bridge_script());
        let shared_ready = shared.clone();
        let handler = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
            move |_identifier, _script_identifier| {
                Shared::ready(&shared_ready);
                Ok(())
            },
        ));
        if let Err(error) =
            unsafe { webview.AddScriptToExecuteOnDocumentCreated(&script, &handler) }
        {
            eprintln!("minmark: WebView2: AddScriptToExecuteOnDocumentCreated: {error}");
            Shared::ready(shared);
        }
    }

    /// Controlador listo: Bounds y visibilidad actuales, cola de scripts y
    /// carga pendiente (en ese orden: la navegación nueva vacía la página).
    fn ready(shared: &Rc<Shared>) {
        Shared::apply_bounds(shared);
        Shared::apply_visible(shared);
        let pending_scripts = std::mem::take(&mut *shared.pending_scripts.borrow_mut());
        let Some(webview) = shared.webview.borrow().clone() else {
            return;
        };
        for script in pending_scripts {
            execute_script(&webview, &script);
        }
        if let Some(uri) = shared.pending_uri.borrow_mut().take() {
            if let Err(error) = unsafe { webview.Navigate(&HSTRING::from(uri)) } {
                eprintln!("minmark: WebView2: Navigate: {error}");
                shared.is_loading.set(false);
            }
        }
    }

    /// Bounds del controlador = caja del widget en coordenadas de la surface
    /// del toplevel, recortada a su área cliente (supuestos de composición
    /// documentados arriba). `size_allocate` la invoca también al hacer
    /// scroll el `ScrolledWindow`, que realoca a su hijo.
    fn apply_bounds(shared: &Rc<Shared>) {
        let Some(controller) = shared.controller.borrow().clone() else {
            return;
        };
        let Some(host) = shared.host.borrow().upgrade() else {
            return;
        };
        let parent = HWND(shared.parent_hwnd.get() as *mut _);
        if parent.0.is_null() {
            eprintln!(
                "minmark: WebView2: HWND del toplevel nulo al colocar; el Preview no se presentará"
            );
            return;
        };
        let Some(root) = host.root() else {
            eprintln!("minmark: WebView2: widget sin raíz al colocar; el Preview no se presentará");
            return;
        };
        let Ok(window) = root.downcast::<gtk::Window>() else {
            eprintln!("minmark: WebView2: la raíz no es una ventana al colocar; el Preview no se presentará");
            return;
        };
        let Some(bounds) = host.compute_bounds(&window) else {
            eprintln!(
                "minmark: WebView2: sin bounds del widget al colocar; el Preview no se presentará"
            );
            return;
        };
        // widget (raíz) → surface: el transform es surface→widget, se resta.
        let (tx, ty) = host
            .native()
            .map(|native| native.surface_transform())
            .unwrap_or((0.0, 0.0));
        let Some(surface) = host.native().and_then(|native| native.surface()) else {
            return;
        };
        // `scale` admite fracciones desde GTK 4.12; el mínimo soportado
        // (4.10) solo expone `scale-factor` entero.
        let scale = if surface.find_property("scale").is_some() {
            surface.property::<f64>("scale")
        } else {
            f64::from(surface.scale_factor())
        };
        let mut client = RECT::default();
        if unsafe { GetClientRect(parent, &mut client) }.is_err() {
            eprintln!(
                "minmark: WebView2: GetClientRect falló al colocar; el Preview no se presentará"
            );
            return;
        }
        let (left, top, right, bottom) = super::native_layout::physical_bounds(
            (
                f64::from(bounds.x()),
                f64::from(bounds.y()),
                f64::from(bounds.width()),
                f64::from(bounds.height()),
            ),
            (tx, ty),
            scale,
            (client.left, client.top, client.right, client.bottom),
        );
        // Caja del widget totalmente fuera del viewport → Bounds vacío: el
        // HWND hijo no debe tapar el resto de la ventana de GTK.
        let bounds = if right > left && bottom > top {
            RECT {
                left,
                top,
                right,
                bottom,
            }
        } else {
            RECT::default()
        };
        if shared.last_bounds.get() == (bounds.left, bounds.top, bounds.right, bounds.bottom) {
            return;
        }
        match unsafe { controller.SetBounds(bounds) } {
            Ok(()) => {
                shared
                    .last_bounds
                    .set((bounds.left, bounds.top, bounds.right, bounds.bottom))
            }
            Err(error) => eprintln!("minmark: WebView2: SetBounds: {error}"),
        }
    }

    fn apply_visible(shared: &Rc<Shared>) {
        if let Some(controller) = shared.controller.borrow().clone() {
            let _ = unsafe { controller.SetIsVisible(shared.visible.get()) };
        }
    }

    /// Widget desrealizado (cierre de ventana): el controlador cierra su
    /// proceso renderizador; una nueva realización recrearía el entorno.
    fn close(shared: &Rc<Shared>) {
        if let Some(controller) = shared.controller.borrow_mut().take() {
            let _ = unsafe { controller.Close() };
        }
        shared.webview.borrow_mut().take();
        shared.is_loading.set(false);
        shared.started.set(false);
        shared.parent_hwnd.set(0);
        shared.last_bounds.set((0, 0, 0, 0));
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if let Some(path) = self.temp_file.borrow_mut().take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

glib::wrapper! {
    /// Widget GTK (una `gtk::Box`) que aloja el WebView2 como HWND hijo del
    /// toplevel. Su clase subclaseada traduce realize/map/unmap y
    /// size_allocate al estado [`Shared`] del motor.
    pub struct Webview2Host(ObjectSubclass<imp::Host>)
        @extends gtk::Widget, gtk::Box;
}

impl Webview2Host {
    fn new(shared: Rc<Shared>) -> Self {
        let host: Self = glib::Object::new();
        host.imp().shared.replace(Some(shared));
        // GtkBox instala BoxLayout: con un layout manager GTK NO invoca
        // WidgetImpl::size_allocate. Este host no tiene hijos GTK; quitarlo
        // permite recolocar la vista nativa al maximizar o mover el separador.
        host.set_layout_manager(None::<gtk::LayoutManager>);
        // El ScrolledWindow (§10.2) debe llenar su viewport con la caja: el
        // contenido real vive en el HWND hijo, no en el dibujo de GTK.
        host.set_hexpand(true);
        host.set_vexpand(true);
        host
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Host {
        pub(super) shared: RefCell<Option<Rc<Shared>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Host {
        const NAME: &'static str = "MinmarkPreviewWebview2Host";
        type Type = super::Webview2Host;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Host {
        fn dispose(&self) {
            if let Some(shared) = self.shared.borrow().clone() {
                Shared::close(&shared);
            }
        }
    }

    impl BoxImpl for Host {}

    impl WidgetImpl for Host {
        fn realize(&self) {
            self.parent_realize();
            if let Some(shared) = self.shared.borrow().clone() {
                Shared::start(&shared, &self.obj());
            }
        }

        fn unrealize(&self) {
            if let Some(shared) = self.shared.borrow().clone() {
                Shared::close(&shared);
            }
            self.parent_unrealize();
        }

        fn map(&self) {
            self.parent_map();
            if let Some(shared) = self.shared.borrow().clone() {
                // ISSUE-57: `start` es idempotente; si el realize llegó antes
                // de que existiera la surface o el HWND, este es el reintento
                // (paridad con `ensure_attached` de WKWebView).
                Shared::start(&shared, &self.obj());
                shared.visible.set(true);
                Shared::apply_visible(&shared);
                Shared::apply_bounds(&shared);
            }
        }

        fn unmap(&self) {
            if let Some(shared) = self.shared.borrow().clone() {
                shared.visible.set(false);
                Shared::apply_visible(&shared);
            }
            self.parent_unmap();
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(shared) = self.shared.borrow().clone() {
                // ISSUE-57: reintento de `start` (idempotente), por si el
                // realize se quedó sin surface/HWND.
                Shared::start(&shared, &self.obj());
                Shared::apply_bounds(&shared);
            }
        }
    }
}

/// `WebView` del Preview en Windows: el widget anfitrión GTK + el estado
/// COM compartido. Los handles que pide el trait (`connect_load_finished`
/// entrega uno nuevo) solo clonan estas dos piezas.
pub(super) struct Webview2Engine {
    host: Webview2Host,
    shared: Rc<Shared>,
}

impl Webview2Engine {
    /// Crea el widget anfitrión. El entorno/controlador WebView2 se pide en
    /// asíncrono al realizarse el widget en el toplevel (HWND del surface).
    pub(super) fn new() -> Self {
        let shared = Rc::new(Shared::default());
        let host = Webview2Host::new(shared.clone());
        shared.host.replace(host.downgrade());
        Self { host, shared }
    }
}

impl PreviewEngine for Webview2Engine {
    fn widget(&self) -> gtk::Widget {
        self.host.clone().upcast::<gtk::Widget>()
    }

    fn load_html(&self, html: &str, _base_uri: &str) {
        // §3.4 / ADR-005: nunca `NavigateToString` (límite de 2 MiB); el
        // HTML viaja como archivo temporal local y la navegación usa su
        // `file:///` percent-escapado, de modo que cualquier URL relativa
        // degrada a un archivo local y nunca a la red. `base_uri` es
        // `file:///` y lo aporta el propio archivo temporal.
        let path = write_temp_html(html);
        let uri = file_uri(&path);
        if let Some(previous) = self.shared.temp_file.borrow_mut().replace(path) {
            // Mejor esfuerzo: si WebView2 aún retiene el anterior (carga en
            // curso), se limpiá en el siguiente reemplazo o al descartar.
            let _ = std::fs::remove_file(previous);
        }
        *self.shared.allowed_uri.borrow_mut() = uri.clone();
        *self.shared.pending_uri.borrow_mut() = Some(uri.clone());
        // Paridad con WebKitGTK: `load_html` deja `is_loading` en true hasta
        // que la navegación termina, para que el panel encole sus cambios.
        self.shared.is_loading.set(true);
        if let Some(webview) = self.shared.webview.borrow().clone() {
            *self.shared.pending_uri.borrow_mut() = None;
            if let Err(error) = unsafe { webview.Navigate(&HSTRING::from(uri)) } {
                eprintln!("minmark: WebView2: Navigate: {error}");
                self.shared.is_loading.set(false);
            }
        }
    }

    fn run_script(&self, script: &str) {
        if let Some(webview) = self.shared.webview.borrow().clone() {
            execute_script(&webview, script);
        } else {
            // El controlador se crea en asíncrono: los scripts se encolan en
            // el mismo orden en que llegarían a la WebView (dispara y olvida,
            // igual que `evaluate_javascript` de WebKitGTK).
            self.shared
                .pending_scripts
                .borrow_mut()
                .push(script.to_owned());
        }
    }

    fn is_loading(&self) -> bool {
        self.shared.is_loading.get()
    }

    fn connect_load_finished(&self, callback: Rc<dyn Fn(&dyn PreviewEngine)>) {
        *self.shared.load_finished.borrow_mut() = Some(callback);
    }

    fn connect_link_activation(&self, activate: Rc<dyn Fn(String)>) {
        // El receiver es `WebMessageReceived`; el script que hace postMessage
        // se inyecta con el controlador (ver `add_link_script`).
        *self.shared.link_activation.borrow_mut() = Some(activate);
    }

    fn block_navigation(&self) {
        self.shared.block_navigation.set(true);
    }

    /// `ICoreWebView2::CapturePreview` a PNG en un `IStream` de memoria que
    /// se vuelca al fichero pedido.
    #[cfg(test)]
    fn snapshot(&self, path: &Path, done: Rc<Cell<bool>>) {
        let Some(webview) = self.shared.webview.borrow().clone() else {
            return;
        };
        let Some(stream) = (unsafe { SHCreateMemStream(None) }) else {
            return;
        };
        let stream_for_call = stream.clone();
        let path = path.to_owned();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
            if result.is_ok() {
                match read_stream(&stream) {
                    Ok(bytes) => match std::fs::write(&path, bytes) {
                        Ok(()) => done.set(true),
                        Err(error) => eprintln!("minmark: WebView2: snapshot: {error}"),
                    },
                    Err(error) => eprintln!("minmark: WebView2: snapshot: {error}"),
                }
            }
            Ok(())
        }));
        let captured = unsafe {
            webview.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                Some(&stream_for_call),
                &handler,
            )
        };
        if let Err(error) = captured {
            eprintln!("minmark: WebView2: CapturePreview: {error}");
        }
    }

    #[cfg(test)]
    fn eval_ok(&self, script: &str, done: Rc<Cell<bool>>) {
        let Some(webview) = self.shared.webview.borrow().clone() else {
            eprintln!("minmark: eval sin WebView2 (controlador sin crear)");
            return;
        };
        let handler = ExecuteScriptCompletedHandler::create(Box::new(move |result, _json| {
            done.set(result.is_ok());
            Ok(())
        }));
        if let Err(error) = unsafe { webview.ExecuteScript(&HSTRING::from(script), &handler) } {
            eprintln!("minmark: WebView2: ExecuteScript: {error}");
        }
    }

    fn eval_number(&self, script: &str, result: Rc<Cell<Option<f64>>>) {
        let Some(webview) = self.shared.webview.borrow().clone() else {
            eprintln!("minmark: eval sin WebView2 (controlador sin crear)");
            return;
        };
        // ExecuteScript serializa el valor como JSON: un número llega como
        // su literal JSON y `null`/errores como texto no numérico.
        let handler = ExecuteScriptCompletedHandler::create(Box::new(move |status, json| {
            result.set(status.ok().and_then(|()| json.trim().parse::<f64>().ok()));
            Ok(())
        }));
        if let Err(error) = unsafe { webview.ExecuteScript(&HSTRING::from(script), &handler) } {
            eprintln!("minmark: WebView2: ExecuteScript: {error}");
        }
    }
}

/// Dispara y olvida `ExecuteScript` (el resultado JSON no interesa aquí;
/// `eval_ok`/`eval_number` usan el handler completo solo en tests).
fn execute_script(webview: &ICoreWebView2, script: &str) {
    let handler = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
    if let Err(error) = unsafe { webview.ExecuteScript(&HSTRING::from(script), &handler) } {
        eprintln!("minmark: WebView2: ExecuteScript: {error}");
    }
}

/// Lectura completa de un `IStream` desde el principio (para `snapshot`).
fn read_stream(stream: &IStream) -> windows::core::Result<Vec<u8>> {
    unsafe {
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8 * 1024];
        loop {
            let mut read = 0u32;
            stream
                .Read(
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    Some(&mut read),
                )
                .ok()?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read as usize]);
        }
        Ok(bytes)
    }
}

// `take_pwstr` viene de `webview2_com` (importado arriba): copia el
// `PWSTR` —los out-params de WebView2 se allocan con `CoTaskMemAlloc`— y
// libera el original al soltarlo.

/// Contador para temporales únicos por proceso y carga.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Escribe el HTML del Preview en un temporal local. El BOM UTF-8 permite a
/// WebView2 detectar la codificación (el render es UTF-8, §10.2) sin depender
/// del `<meta charset>` de la página.
fn write_temp_html(html: &str) -> PathBuf {
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "minmark-preview-{}-{sequence}.html",
        std::process::id()
    ));
    let mut bytes = Vec::with_capacity(html.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(html.as_bytes());
    if let Err(error) = std::fs::write(&path, bytes) {
        eprintln!("minmark: WebView2: no se pudo escribir el temporal del Preview: {error}");
    }
    path
}

/// `file:///` percent-escapado de una ruta absoluta Windows (barras
/// invertidas a `/`, todo lo que no sea ASCII seguro en %XX con hex
/// mayúscula, como espera el convenio de URI de WebView2).
fn file_uri(path: &Path) -> String {
    let slashed = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file:///");
    for byte in slashed.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// Equivalente exacto del `UserScript` de `engine.rs` (WebKitGTK) con el
/// puente de mensajes propio de WebView2: clic en ancla ⇒ preventDefault,
/// anclas `#mm-note-` ⇒ scroll in-page (§26) y el resto ⇒ href al canal
/// `minmarkLink` vía `window.chrome.webview.postMessage`.
fn link_bridge_script() -> String {
    String::from(
        "document.addEventListener('click', e => { const a = e.target.closest('a[href]'); if (a) { e.preventDefault(); const href = a.getAttribute('href'); if (href.startsWith('#mm-note-')) { document.getElementById(href.slice(1))?.scrollIntoView({block: 'center'}); } else { window.chrome.webview.postMessage(href); } } });",
    )
}
