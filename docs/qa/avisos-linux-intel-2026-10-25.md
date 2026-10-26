# Diagnóstico gráfico de Linux — 2026-10-25

Entorno: X11, GTK 4.14.5 (`4.14.5+ds-0ubuntu0.10`), WebKitGTK 2.52.6,
Mesa 25.2.8 e Intel Vulkan. El tema de escritorio es Mint-Y-Aqua.

Se abrieron instancias de prueba bajo GDB y se editaron archivos temporales.
La instancia original del usuario permaneció abierta. Se descartaron las
modificaciones de código que no evitaron el aviso; no se conserva un parche
del editor sin validación.

## Intel: alternativa comprobada

El diagnóstico integrado pasó con el renderer predeterminado, aunque aparecían
los dos avisos `FINISHME` de `anv_formats.c`. Forzar `GSK_RENDERER=gl` también
pasó el diagnóstico y siguió emitiéndolos.

Con `WEBKIT_DISABLE_DMABUF_RENDERER=1`, el diagnóstico pasó sin esos dos avisos:

```sh
WEBKIT_DISABLE_DMABUF_RENDERER=1 ./target/release/minmark --check-preview
```

Esto comprueba una alternativa al transporte DMA-BUF de WebKit; no constituye
una corrección del controlador Intel ni del aviso de GTK. Puede afectar al
rendimiento de la vista previa. No se cambió el valor predeterminado de la app.

Evidencia: [salida del diagnóstico](evidence/intel-no-dmabuf-preview.log).

## GTK: origen confirmado, corrección pendiente

Se reprodujo el aviso al abrir un documento largo, sustituir el contenido por
texto corto/largo, desplazar y redimensionar la ventana. La captura se hizo
en la entrada de `g_log_structured_standard`, antes de emitir el mensaje.
Los getters de GTK permitieron identificar esta jerarquía:

```text
GtkGizmo (trough)
  GtkRange (range)
    GtkScrollbar (scrollbar)
      GtkScrolledWindow (scrolledwindow)
        GtkBox (box del editor)
          GtkPaned
            GtkBox
              GtkApplicationWindow
```

El control afectado es la barra vertical del editor. El aviso se emite durante
el recorrido de dibujo de GTK, cuando el elemento aún requiere asignación de
geometría. La captura identifica el elemento y el punto de emisión; no captura
la modificación anterior que invalidó su geometría.

Evidencia: [traza y jerarquía originales](evidence/gtk-scrollbar-warning.log).

Se reprodujo también con barras sin superposición, política vertical `Always`,
tema Adwaita y animaciones desactivadas. Ninguna de esas opciones se considera
una solución. Las pruebas que inicialmente no emitían el aviso no bastaban:
su aparición es intermitente.

El diagnóstico integrado de abrir, editar, redimensionar, guardar, cerrar,
crear un documento y mostrar la vista previa pasó. No se observó un cierre
espontáneo de Minmark por este aviso; GDB detuvo deliberadamente las instancias
de captura. Un fallo causado por una primera lectura incorrecta de argumentos
en el depurador se descartó y no se atribuye a la aplicación.

Las listas locales de APT ofrecían como candidatas las mismas versiones de
GTK y Mesa instaladas. No se comprobó que otra versión de GTK elimine el aviso.
