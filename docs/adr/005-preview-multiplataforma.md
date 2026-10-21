# ADR-005: Preview multiplataforma y ejecutables por sistema operativo

**Fecha:** 2026-10-20
**Estado:** Aceptada
**Contexto:** Publicar ejecutables de Windows, macOS y Linux desde GitHub Actions,
con MSI en Windows; SRS §50.2, Fase 8; continúa ADR-001 (GTK4 + WebKitGTK 6.0)

## Contexto

La release actual solo publica el `.tar.gz` de Linux. Se exige publicar
ejecutables de cada sistema operativo de escritorio (Windows, macOS y Linux),
con un instalador MSI (o equivalente) en Windows, construidos en los runners de
GitHub.

ADR-001 fijó GTK4 + WebKitGTK 6.0; el uso de `webkit6` está confinado al
backend Linux de Preview (hoy tras el trait `PreviewEngine` y su módulo de
motor), y el resto de la UI es GTK4 puro. El HTML del Preview
(`minmark_renderer`), los scripts de actualización de DOM, la política de
navegación y el PDF por Typst son independientes del motor del webview.

Hechos que enmarcan la decisión:

- WebKitGTK 6.0 no tiene distribución utilizable en Windows: la petición en
  MSYS2 sigue abierta (msys2/MINGW-packages#19965) y el port MinGW existente
  (`jralls/mingw-w64-webkitgtk`) es webkitgtk 2.4.9 (WebKit1/GTK3), insuficiente
  para la API `webkitgtk-6.0`. En macOS no hay binarios; el port GTK de WebKit
  es de Linux en la práctica. Compilar WebKit desde fuentes en ambos sistemas
  está fuera de lo que un workflow puede sostener.
- `minmark-fs` está acoplado a Linux (watcher inotify propio §46, symlinks
  Unix, papelera estilo XDG) y los directorios de configuración/estado/índice
  se derivan de XDG.
- GTK4 sí es portable: en Windows se construye con MSYS2/gvsbuild y en macOS
  con Homebrew. gtk-rs expone los identificadores nativos necesarios
  (`gdk4-win32` con `gdk_win32_surface_get_handle`; `gdk_macos_surface_get_native_window`
  desde GTK 4.8).

## Opciones evaluadas

1. **`wry` como webview único (WebView2/WKWebView/WebKitGTK)**: descartado. Su
   backend de Linux usa GTK3 (`webkit2gtk` 2.x), `build_as_child` solo admite
   X11 y `build_gtk` acepta contenedores GTK3: adoptarlo rompe ADR-001 y el
   soporte Wayland actual.
2. **Compilar WebKitGTK 6.0 desde fuentes en Windows/macOS**: descartado. Sin
   soporte upstream, build de horas por ejecución y ninguna garantía de API
   `webkitgtk-6.0` en Windows (el único port existente es de la generación 2.4).
3. **Webview nativo por sistema operativo tras una interfaz común**: elegida.
   Mantiene GTK4 para toda la UI y el mismo HTML/CSS de Preview en los tres
   sistemas.
4. **Publicar sin Preview embebido en Windows/macOS**: descartado. Degradaría el
   producto de forma visible (§56).

## Decisión

**Un trait `PreviewEngine` en `ui/src/preview/` con tres implementaciones:
WebKitGTK (`webkit6`) en Linux sin cambio de comportamiento, WebView2 en
Windows como HWND hijo de la superficie GDK, y WKWebView en macOS como NSView
hijo de la ventana nativa de GTK.**

### Superficie común

- Carga del HTML con URI base `file:///` (§3.4: sin recursos de red), sin
  `NavigateToString` de WebView2 (límite de 2 MiB): la página viaja como
  archivo temporal local.
- Actualización del mismo documento preservando scroll y reutilizando imágenes
  (script `update_script`), scroll por posición en bytes (`scroll_script`),
  evaluación de JS, estado de carga, script de usuario inicial (interceptación
  de enlaces `minmarkLink`) y bloqueo de navegación saliente.
- Render, CSS, puentes JS, diagnósticos de enlaces y exportación PDF
  (`minmark_pdf`/Typst) se comparten tal cual; solo cambia quien aloja la
  vista web.

### Empaquetado

- **Linux**: `.tar.gz` actual sin cambios (GTK/WebKitGTK del sistema).
- **Windows**: ejecutable + GTK4 y sus DLLs + WebView2 Evergreen con
  instalador de prerrequisito **offline** incluido en el bundle; se publican
  MSI y ZIP portable (§50.2). La toolchain MSI será WiX v6 u otra con soporte
  vigente: WiX v5 terminó su soporte para consumidores el 2026-02-05. La
  elección concreta se valida en la primera ejecución del job.
- **macOS**: `.app` autocontenido (GTK por Homebrew copiado al bundle) dentro
  de `.dmg` y/o `.tar.gz`; firma ad-hoc en CI. Developer ID y notarización
  quedan como paso documentado, pendiente de secretos de firma.
- `SHA256SUMS` cubre los artefactos de los tres sistemas.

### Watching y directorios

- Watcher: inotify propio se conserva en Linux; en Windows/macOS se usa el
  backend nativo del crate `notify` (ReadDirectoryChangesW / FSEvents) tras el
  mismo contrato `Watcher` (§46: notificaciones nativas, nunca rescan).
- Directorios centralizados en un único módulo del core: Linux conserva sus
  rutas XDG actuales byte a byte; Windows usa `%APPDATA%\minmark` (config y
  estado) y `%LOCALAPPDATA%\minmark` (índice y snapshots); macOS
  `~/Library/Application Support/minmark`.

### Razones

- Mantiene ADR-001 y el comportamiento observable en Linux; el mismo HTML/CSS
  implica el mismo render salvo diferencias de motor.
- WebView2 y WKWebView son los motores del sistema: sin dependencias de
  compilación exóticas en los runners y con actualización de seguridad a cargo
  del sistema.
- La interfaz común limita el código por plataforma al alojamiento del webview,
  el transporte de HTML/JS y el ciclo de vida nativo.

## Consecuencias

- **Privacy & Control Promise**: el runtime WebView2 Evergreen de Microsoft
  puede recopilar diagnósticos del propio runtime que minmark no controla ni
  desactiva. minmark no añade telemetría ni acceso a red (§3.4), pero Windows
  queda sujeto a esta desviación documentada; debe constar en README y en la
  documentación del instalador, y queda a criterio del propietario aceptarla o
  exigir un runtime de versión fija / otro engine.
- El webview es una ventana nativa hija del widget GTK: composición, foco,
  recorte, HiDPI y comportamiento al redimensionar son gates de integración no
  probados en este entorno. Se validan en los runners y en QA manual antes de
  publicar.
- Dos backends nuevos = dos rutas de QA (Windows, macOS) sin QA gráfico
  automatizado. El registro distingue «build verde» de «confirmación manual».
- `macos-14` se retira el 2026-11-02: los jobs nuevos usan `macos-15`.
- Dependencias nuevas justificadas (misma línea que ADR-003): `notify`,
  bindings de WebView2, bindings de WKWebView y un backend de papelera para
  Windows/macOS.
