# Estabilidad de Windows — 2026-10-25

Base: `219f813` (v0.4.2). Seguimiento: [paridad funcional de fase 8, #35](https://github.com/ximocm/minmark/issues/35).
Se adelanta esta validación para corregir fallos de uso básico del paquete.
No equivale al cierre de fase 8: siguen siendo necesarios Windows 10/11,
MSI y ZIP finales, y la paridad con los controles pendientes de fase 7.

## Fallos reproducidos en el paquete instalado

1. Abrir un archivo existente, editar y guardar: funciona.
2. Crear «Sin título» y pulsar Ctrl+S: el proceso se cierra. Registro capturado:
   `GLib-GIO-ERROR: No GSettings schemas are installed on the system`.
   El paquete contiene DLL de GTK, pero no sus schemas.
3. Maximizar la ventana: el separador GTK se desplaza mientras la WebView
   conserva su caja anterior y deja una zona blanca/desalineada.

La revisión del código identifica además dos préstamos de `RefCell`
retenidos durante llamadas que vuelven a acceder al modelo: Ctrl+W sobre
una pestaña guardada y la recarga tras una sobrescritura confirmada.

## Correcciones y regresiones

| Caso | Cobertura añadida |
| --- | --- |
| Selector de guardado | Schema GTK requerido antes de abrir; aviso recuperable si falta. `--check-preview` abre y cancela un selector real en Windows/Linux, sin escribir. En macOS requiere comprobación manual. |
| ZIP/MSI sin recursos | Copia y compilación estricta de schemas. El empaquetado se detiene si faltan; pruebas con schemas ausentes y ruta Unicode/con espacios. |
| Dependencia del equipo de build | Smoke y diagnóstico del paquete con PATH aislado y sin variables de recursos GTK/GIO/XDG heredadas. |
| Tamaño de Preview | Quitar `BoxLayout` del host sin hijos permite recibir `size_allocate`; se compara el viewport web con el tamaño del widget tras varios tamaños, vistas y barra lateral. Windows incluye maximizar/restaurar. |
| DPI | Pruebas de conversión lógica/física a 100, 125, 150 y 200 %, transformación de superficie y recorte al cliente. |
| Guardado/conflicto/cierre | La autocomprobación escribe y relee su fixture, detecta un cambio externo, sobrescribe por el camino confirmado y cierra la pestaña guardada. |

GTK documenta que [`size_allocate`](https://docs.gtk.org/gtk4/vfunc.Widget.size_allocate.html)
solo se invoca si no hay un layout manager, y que las coordenadas de superficie
requieren aplicar su [escala para obtener píxeles físicos](https://docs.gtk.org/gdk4/method.Surface.get_scale_factor.html).

## Ejecución local

Entorno de pruebas: Ubuntu 26.04 en WSL, Rust/Cargo 1.93.1, GTK 4.22.4,
WebKitGTK 2.52.6 y Xvfb con `GDK_BACKEND=x11` y renderizado por software.
Las pruebas de permisos se ejecutan como usuario
normal. Los documentos empleados son fixtures temporales.

| Batería | Resultado |
| --- | --- |
| Core, siete crates y pruebas de integración | 422 correctas |
| Lógica de UI | 122 correctas; seis escenarios gráficos se ejecutan aparte |
| Política de release y empaquetado | 27 correctas |
| `MINMARK_QA=1`, `qa_preview` | Correcta; selector, guardado y redimensionado reales |
| Seis escenarios gráficos de integración | Seis correctos: cambios externos, navegación/academic, composiciones, scroll interno, pestañas y diagnóstico Preview |
| Recursos de GTK ausentes, ejecutable real | Error legible y salida 1; sin abort de GLib |
| Build y paquete Windows corregido | CI #80 correcta; ZIP portable comprobado también en el equipo Windows |

Comandos reproducibles desde la raíz:

```sh
cargo test --locked --manifest-path core/Cargo.toml --workspace
cargo test --locked --manifest-path ui/Cargo.toml
python3 scripts/test_release.py
GDK_BACKEND=x11 GSK_RENDERER=cairo WEBKIT_DISABLE_COMPOSITING_MODE=1 MINMARK_QA=1 xvfb-run -a cargo test --locked --manifest-path ui/Cargo.toml --test qa_preview
```

Cada escenario GTK ignorado por la suite normal debe ejecutarse en su propio
proceso (`--ignored --exact`), con el nombre devuelto por
`cargo test --locked --manifest-path ui/Cargo.toml --bin minmark -- --ignored --list`.
La CI ejecuta los seis escenarios sobre Xvfb en Linux; Windows y macOS
ejecutan `qa_preview` y el diagnóstico del paquete.
La apertura/cancelación programática del selector Cocoa se deja fuera del
diagnóstico automático: en CI #83/#84 el panel o el procesamiento posterior
no devolvieron el control de forma fiable. El resto del escenario macOS,
incluido guardado del fixture, cierre y layout real, sigue siendo obligatorio.
La interacción del selector de macOS queda pendiente de validación manual;
no se cuenta entre las comprobaciones superadas.
El diagnóstico exige varios tamaños reales del viewport. El tamaño exterior
solicitado puede quedar limitado por el gestor de ventanas del runner;
Split/Preview y la barra lateral también ejercitan la recolocación nativa.

En WSL, `xvfb-run` por sí solo no impide que GTK elija el Wayland heredado:
se observaron viewports 0×0 al probar ventanas ocultas en ese compositor.
Forzar X11 permite comprobar el layout visible de forma reproducible. La
prueba académica espera el layout y el desplazamiento antes de medir notas.

Clippy termina sin errores y conserva avisos preexistentes. Los archivos Rust
modificados pasan rustfmt. El formato global del repositorio presenta
diferencias anteriores a esta corrección, también en crates del core; no se
mezcla su reformateo con estos arreglos.

## Comprobación del paquete Windows en el equipo

Artefacto `windows-x86_64` de [CI #80](https://github.com/ximocm/minmark/actions/runs/40197357521),
commit `7cd03ba`. SHA-256 del artefacto:
`1768b7c9c7d9b3a68db0a410a8dfb4aa2921f82bca939e5b5aefe0585eddb4f7`.
Se verificaron también los hashes de MSI, ZIP y bundle incluidos en SHA256SUMS.

Windows 25H2, build 26200.9711, escritorio de 1920×1080. El ZIP extraído
se ejecutó desde otra carpeta y su `--check-preview` terminó con salida 0.
Desde la interfaz real se creó un documento con `$r_0$`, se guardó mediante
el selector nativo como un archivo nuevo y se comprobó el texto UTF-8 en
disco. El título perdió el indicador de cambios y Ctrl+W cerró la pestaña
guardada sin cerrar la aplicación. Al maximizar se comprobó visualmente
que editor, separador y Preview ocupan las posiciones correctas.

La instalación MSI existente no se ha reemplazado: esta comprobación utiliza
el paquete portable de la rama de corrección.

## Pendiente para cerrar fase 8

- Validar el paquete corregido en Windows 10 y 11, tanto MSI como ZIP,
  incluida instalación/desinstalación y apertura desde otra carpeta.
- Comprobar cambios entre monitores y DPI fraccionario real; las pruebas
  geométricas no sustituyen a esa verificación.
- Completar el resto de la matriz funcional de #35 y los controles de fase 7.
- Comprobar manualmente guardar/cancelar con el selector nativo de macOS.
