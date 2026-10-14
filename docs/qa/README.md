# QA de fase 5 — Academic

Suite preparada el 2026-10-14. Cubre las tareas #14–#20 del
[plan de fase 5](https://github.com/ximocm/minmark/issues/13), SRS §§24–30 y §58.
Los resultados están en el [registro de ejecución](registro-ejecucion.md).

## Preparación

Conservar documentos y configuración reales. Copiar [fixtures/](fixtures/) a
una carpeta temporal `minmark-qa-academic`; no trabajar sobre documentos reales.
La copia debe conservar `academic.md`, `references.bib`, `extra.bib` y
`author-year.csl` juntos. Todos los archivos están incluidos; no se necesitan
cuentas, servicios remotos ni datos de producción.

Linux con GTK ≥4.10 y WebKitGTK 6.0. Perfil: usuario local. Interfaz en inglés;
documento en español; zona Europe/Madrid. Abrir la carpeta mediante
**File → Open Folder**, después `academic.md` desde el explorador. Usar **View →
Split**. Las copias de QA y los PDF pueden sobrescribirse; cancelar cualquier
confirmación sobre archivos reales.

Orden: ACA-01 → ACA-02 → ACA-03 → ACA-04 → ACA-05 → ACA-06 → ACA-07 → ACA-08.
Restaurar `academic.md` y los `.bib` de las fixtures antes de cada variante.
ACA-09 y ACA-10 son comprobaciones adicionales del entorno.

## Casos

### ACA-01 · Ecuaciones y errores

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** fixture original en Split.

1. Revisar energía, fracción, raíz, integral y matriz en Preview.
2. Añadir `$\sum_{i=1}^{n}i$`, `$\hat{x}+\bar{x}+\vec{x}$` y `$\sin x+\cos x$`.
3. Añadir `$\foo{x}$` y `$\frac{1}$`; exportar con **File → Export PDF**.

**Esperado:** fórmulas compuestas legibles, sin fondo que oculte símbolos;
comandos desconocidos/incompletos muestran marcadores en Preview y PDF.
El editor conserva el fuente y sigue aceptando cambios.

### ACA-02 · Bibliografía, citas y CSL

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** dos `.bib` activos.

1. Revisar citas simples, múltiples, locator `p. 42` y bibliografía final.
2. Añadir `bibliography_style: author-year.csl` al front matter.
3. Exportar el buffer sin guardarlo.
4. Cambiar `Energy Research` a `Energy Research Updated` en la copia `.bib`.

**Esperado:** IEEE usa [1]/[2]; CSL alternativo muestra Smith, 2025 y Jones/García,
2024. Locator 42 permanece. HTML y PDF tienen las mismas referencias y orden.
El cambio externo aparece sin editar ni marcar dirty el Markdown.

### ACA-03 · Archivos y claves inválidos

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** copias desechables.

1. Añadir `[@missing]` al documento.
2. Activar un `.bib` ausente; después un `.bib` con `@book{` incompleto.
3. Repetir `smith2025` en un segundo archivo activo.
4. Usar un CSL ausente y después uno con contenido inválido.

**Esperado:** mensajes visibles identifican el recurso o clave; ninguna clave
ambigua se resuelve silenciosamente. CSL inválido vuelve a IEEE con aviso.
Se preservan buffer, selección y posibilidad de corregir el documento.

### ACA-04 · Autocompletado y undo

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** fixture con bibliografía.

1. Escribir `β [@Sm, p. 42]`; situar el cursor después de `Sm`.
2. Usar flechas y Tab/Enter para aceptar `smith2025`.
3. Deshacer y rehacer una vez; cancelar otra sugerencia con Escape.
4. Buscar por autor, título y año. Repetir dentro de código y sintaxis escapada.

**Esperado:** solo se reemplaza la clave; `β`, locator y saltos LF/CRLF se
conservan. Una operación revierte la aceptación. Código/escapes no ofrecen
citas; cancelar no modifica texto. El foco permanece en el editor.

### ACA-05 · Notas y páginas

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** fixture original.

1. Pulsar la nota 1 y su enlace de vuelta en Preview continuo.
2. Revisar que la referencia repetida conserva el número 1.
3. Añadir `pagination: true` y exportar PDF.
4. Probar nota ausente, duplicada y larga; restaurar la fixture después.

**Esperado:** continuo incluye dos notas enlazadas al final. Paginado/PDF sitúa
la nota de energía al pie de la página 1 y la segunda al pie de la página 2.
Ausentes/duplicadas muestran diagnóstico. El número depende de la primera
aparición; se recalcula tras reordenar el texto.

### ACA-06 · Numeración y captions

**Prioridad:** P1. **Perfil:** usuario local. **Antes:** fixture original.

1. Revisar heading `1 Energía`, tabla 1.1, ecuaciones 1.1 y 2.1.
2. Cambiar `counter_numbering` a `continuous` y a `by_chapter_reset`.
3. Desactivar `heading_numbering`; mover/reordenar un bloque y revisar PDF.

**Esperado:** modo continuo cuenta ecuaciones 1/2; reinicio simple usa 1/1.
Headings dejan de numerarse al desactivar el ajuste. Captions y referencias
coinciden en HTML/PDF; los captions quedan debajo de los objetos.

### ACA-07 · TOC, profundidad y literales

**Prioridad:** P1. **Perfil:** usuario local. **Antes:** fixture original.

1. Revisar los cuatro enlaces del TOC y navegar a Resultados.
2. Cambiar `toc_max_level` a 6; revisar el heading H4.
3. Revisar versiones continua/paginada/PDF y un documento sin headings.
4. Revisar `{{toc}}` dentro de código y escapado; eliminar el token real.

**Esperado:** profundidad 3 genera cuatro entradas; profundidad 6 añade H4.
Páginas del PDF: Energía/Resultados/Detalle en 1, Segundo capítulo en 2.
Sin headings se genera un índice vacío; sin token real no aparece TOC.
Código/escapes permanecen literales.

### ACA-08 · Integridad y recuperación

**Prioridad:** P0. **Perfil:** usuario local. **Antes:** copia del documento.

1. Modificar texto, alternar **View → Markdown/Visual** dos veces y exportar sin guardar.
2. Revisar texto, selección, dirty y undo/redo; volver a abrir el archivo del disco.
3. Corregir un error de ecuación/CSL/cita y revisar Preview y exportación.

**Esperado:** representaciones derivadas y exportación no escriben el archivo.
La edición sin guardar llega al PDF; alternar modos conserva el buffer y el
historial. Corregir un error permite recuperar la salida.

### ACA-09 · Accesibilidad y Wayland

**Prioridad:** P1. **Perfil:** usuario con lector de pantalla. **Antes:** sesión
Wayland y servicio de accesibilidad disponibles.

1. Recorrer editor, sugerencias, diagnósticos y notas con teclado y lector de pantalla.
2. Repetir ACA-04/05 en Wayland.

**Esperado:** foco, etiquetas y errores perceptibles; acciones accesibles y
resultados equivalentes. Si falta el entorno, registrar Bloqueado, nunca Pasado.

### ACA-10 · Glifos CJK en PDF

**Prioridad:** P2. **Perfil:** usuario local. **Antes:** fixture original.

1. Comparar `東京` en fuente, Preview continuo, paginado y PDF.

**Esperado:** los dos caracteres se conservan y son legibles. La ausencia de
fuentes CJK se registra como limitación del motor tipográfico.

## Cierre

P0 pasados; incidencias y bloqueos restantes identificados. La evidencia de
lógica, integración gráfica y revisión visual se registra separadamente. No
inferir lector de pantalla ni Wayland a partir de Xvfb. Para repetir la campaña
automatizada y generar evidencia, usar los comandos del registro.
