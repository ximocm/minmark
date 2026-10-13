# Markdown Intelligence — fase 4

Implementación de las tareas [#5–#11](https://github.com/ximocm/minmark/issues/3),
sobre la resolución común de #4. Las consultas no escriben Markdown ni crean
archivos. Los buffers abiertos prevalecen sobre el índice de disco.

## Uso

- **Ctrl+Click**, **Ctrl+Enter** o **Edit → Open Link** abre el enlace bajo el
  cursor en Markdown y Visual. Preview activa enlaces con clic o teclado.
  Se reutiliza una pestaña abierta, incluidos sus cambios sin guardar; un
  fragmento coloca el cursor y desplaza editor y Preview al destino.
- Las ediciones del documento activo actualizan el DOM de Preview y mantienen
  visible el bloque donde se escribe, sin desplazarlo en cada pulsación si ya
  está visible. Los refrescos generales conservan el scroll y se reutilizan las
  imágenes sin cambios. Cambiar de documento carga una página independiente;
  navegar a un heading sigue desplazando Preview.
- Al escribir `[texto](destino)`, aparecen archivos relativos al documento.
  Tras `#`, aparecen los headings del destino, incluidos cambios sin guardar.
  Flechas recorren la lista; Enter/Tab aceptan y Escape cancela. La aceptación
  sustituye únicamente el destino y se deshace en una operación.
- Los enlaces rotos llevan subrayado ondulado. Ctrl+Enter explica si falta el
  archivo, el fragmento o hay ambigüedad. Preview también ofrece la explicación
  como título y nombre accesible del enlace. No se comprueban URLs por red.
- **View → Outline**, **Ctrl+Alt+O**: headings H1–H6, orden y jerarquía del
  documento activo. **View → Backlinks**, **Ctrl+Alt+B**: archivo, línea,
  columna y contexto de cada ocurrencia que enlaza al documento activo.
  Son ventanas temporales: Escape las cierra; se actualizan con el documento
  y el índice. Un documento sin ruta no tiene backlinks.

## Configuración

Wiki links están desactivados por defecto. Se pueden habilitar con
`wiki_links = true` en el nivel raíz de `.mdproject.toml` o de la configuración
global, `$XDG_CONFIG_HOME/minmark/config.toml` (por defecto
`~/.config/minmark/config.toml`). Proyecto prevalece sobre global; el documento
no puede habilitarlos. Los cambios se reflejan en los buffers abiertos.
`[[nombre]]` y `[[nombre#heading]]` comparten navegación, sugerencias,
diagnósticos y backlinks. Un nombre ambiguo no elige un archivo arbitrario.
Las operaciones de renombrado/movimiento reciben el mismo ajuste efectivo.

Las etiquetas de `{{ref:ID}}` usan `language` del front matter del documento,
del proyecto o de la aplicación, por ese orden. `es` produce Figura/Tabla/
Ecuación; `en` produce Figure/Table/Equation. El valor por defecto es `es`.
Los ajustes escalares se leen con el vocabulario y la precedencia de
`minmark-config`; no se interpretan las tablas TOML de composiciones como
ajustes del proyecto.

`heading_numbering = true` activa la numeración jerárquica, desactivada por
defecto. `counter_numbering = "by-chapter"` reinicia los contadores por H1;
el valor predeterminado `"continuous"` mantiene la numeración continua.

## Referencias cruzadas

Preview y PDF expanden las referencias a IDs explícitos de headings, imágenes,
tablas y ecuaciones desde una copia derivada del AST. Ejemplo:

```markdown
# Método {#metodo}

![Esquema](esquema.svg){#fig:esquema}

| Variable | Valor |
| --- | --- |
| x | 1 |
{#tbl:datos}

$$
x = 1
$$
{#eq:uno}

Ver {{ref:metodo}}, {{ref:fig:esquema}}, {{ref:tbl:datos}} y {{ref:eq:uno}}.
```

Los IDs duplicados y las referencias ausentes tienen un diagnóstico visible.
Las imágenes con ID `fig:` muestran debajo su texto alternativo como pie de
figura numerado (por ejemplo, «Figura 1. Esquema») en Preview y PDF, con el
mismo idioma y ámbito de numeración que sus referencias.
El modo Visual conserva los tokens editables. Reordenar el documento recalcula
las etiquetas sin escribir números en el fuente. Las referencias entre archivos
compuestos y su numeración combinada siguen perteneciendo a la fase 6.

El PDF recibe las imágenes locales como recursos en memoria con rutas virtuales;
Typst no obtiene acceso libre al disco ni a la red. Las rutas se validan contra
el proyecto (o el directorio del documento en una ventana independiente).

## Índice y ejecución

Se amplía el índice existente, sin una segunda fuente persistente: el formato
v3 incluye IDs de objetos con nivel 0, excluidos de las búsquedas de headings,
y stamps con nanosegundos para detectar ediciones rápidas del mismo tamaño.
Las cachés anteriores se reconstruyen fuera del proyecto. Se reutiliza el parser
del workspace desde `minmark-fs`; no hay dependencias externas nuevas ni cambios
de versiones en los lockfiles.

Resolución, filtrado y backlinks se ejecutan en workers. Las consultas pendientes
se agrupan y los resultados se descartan al cambiar el documento. La selección
de una sugerencia vuelve a verificar documento, contenido y cursor. Se mantienen
las reglas de `.gitignore`, límites del proyecto y symlinks.

## Verificación

Entorno: Linux Mint 22.3, Rust 1.98.1, GTK 4.14.5, WebKitGTK 2.52.6, X11.
Base: `4728d9e`; cambios de trabajo de la rama `fase4/markdown-intelligence`.

```sh
cargo test --locked --manifest-path core/Cargo.toml --workspace
cargo test --locked --manifest-path ui/Cargo.toml
cargo test --locked --manifest-path ui/Cargo.toml \
  gtk_navigation_outline_completion_and_dirty_buffers \
  -- --ignored --test-threads=1 --nocapture
```

Resultado: 335 pruebas del core y 89 de la UI pasan; la prueba gráfica
también pasa (1 prueba adicional). Los archivos Rust modificados pasan
`rustfmt --check` y el diff pasa `git diff --check`.

La última prueba abre una ventana real y comprueba Ctrl+Enter en Markdown y
Visual, clic DOM en Preview, reutilización de pestañas modificadas, outline,
Escape, Tab y undo. Está ignorada por defecto porque necesita una sesión gráfica.
Las pruebas de lógica cubren Unicode/CRLF, contextos excluidos del completado,
headings modificados, ignorados, ambigüedad wiki, backlinks, idiomas, numeración,
IDs de objetos y exportación PDF con recursos locales.

Revisión manual por pasos: el usuario ha confirmado el paso 1 (navegación)
con el conjunto local `dist/revision-fase4`: Ctrl+Click y Ctrl+Enter en Markdown
y Visual, clic en Preview, destinos de archivo y fragmentos, espacios/tildes,
referencias y reutilización de pestañas con cambios sin guardar.
El usuario también ha confirmado el paso 2 (autocompletado): sugerencias de
archivos y headings, selección con flechas, aceptación con Tab/Enter,
cancelación con Escape, deshacer y exclusión de código/comentarios.
El usuario ha confirmado también el paso 3 (diagnósticos), siguiendo la revisión
de enlaces a archivos y headings inexistentes, explicación en Preview y
actualización al corregir el fragmento o crear el archivo de destino.
El usuario ha confirmado el paso 4 (wiki links) tras revisar el documento de
prueba y alternar `wiki_links` en la configuración del proyecto.
El usuario ha confirmado el paso 5 (backlinks), siguiendo la revisión de
ocurrencias, navegación al origen, actualización con cambios sin guardar,
cierre con Escape y estado vacío.
El usuario ha confirmado el paso 6 (outline): orden y jerarquía de headings,
inclusión de Setext, exclusión de código/comentarios, navegación y actualización
con títulos sin guardar, y cierre con Escape sin modificar el documento.
El paso 7 está validado en los casos revisados: el usuario ha confirmado la exportación
con Ctrl+E del documento activo con cambios sin guardar, pies de figura y
referencias resueltas, sin incluir el contenido de la otra pestaña abierta.
El usuario también ha confirmado el cambio de español a inglés en las
referencias y pies de figura de Preview/PDF, conservando los tokens del editor.
El usuario ha confirmado la renumeración al reordenar figuras: pies y
referencias coinciden en Preview/PDF, los tokens se conservan y deshacer
restaura la numeración original. Ha confirmado también los diagnósticos de
referencia inexistente e ID duplicado en Preview/PDF y su desaparición al
corregir el destino y desambiguar el ID, incluida una nueva exportación.
En esa revisión se detectó la ausencia de pies de figura; se corrigió su salida
en Preview/PDF y se añadieron regresiones de posición, idioma y numeración.
Verificación de la corrección: 33 pruebas de renderer/PDF y una exportación
real de figura local a PDF pasan; el usuario ha confirmado los pies en el PDF.
También se corrigió el reinicio del scroll de Preview al escribir. Pasan las
89 pruebas de UI y la integración GTK ampliada, que verifica conservación del
scroll en refrescos generales, seguimiento del bloque editado, estabilidad al
seguir escribiendo en él, contenido actualizado, navegación explícita y cambio
de documento. Pendiente de confirmación manual del usuario.

La comprobación del diálogo PDF (#2) se realiza sobre el desarrollo de la PR
#36 (`6abe9ee`), no sobre el binario publicado v0.1.1. El usuario ha confirmado
el menú File, nombre/filtro, cancelación conservando cambios del documento,
cancelación de sobrescritura conservando el PDF anterior y sobrescritura
aceptada con contenido actualizado. También ha confirmado que exportar a un
destino sin permiso de escritura muestra el error esperado. La carpeta local
de prueba `sin-permiso-pdf` tiene permisos 0555 y se comprobó previamente que
rechaza la escritura. El usuario ha confirmado también el error del motor con
una imagen SVG dañada: pudo continuar editando y, al sustituirla por una imagen
válida, exportar correctamente. Estos escenarios del diálogo están validados
en la versión de desarrollo indicada; no constituyen una prueba de v0.1.1.

Para el cierre definitivo del milestone queda la revisión con
lector de pantalla y una sesión Wayland. La prueba gráfica automatizada no
sustituye esas comprobaciones. No se
ha cerrado ninguna issue automáticamente ni se ha publicado una release.

La prueba manual con 1000 notas y 20 secciones por nota detectó que las flechas
seleccionaban resultados fuera de vista sin desplazar la lista. Se corrigió
el seguimiento de la selección en Quick Open, outline/backlinks y completado,
sin mover el foco desde el campo de entrada/editor. Pasan las 89 pruebas de UI
y la integración GTK ampliada, que baja 30 filas y vuelve al inicio en listas
de resultados y sugerencias. El usuario ha confirmado la comprobación manual
del proyecto grande tras la corrección.
Ante el informe de solo 50 backlinks, la integración gráfica comprueba además
la consulta real de 1000 documentos: la lista contiene 1000 resultados, permite
navegar hasta la fila 76 y filtrar/abrir la nota 0999. La prueba pasa y el usuario
ha confirmado después que la lista funciona correctamente; el límite de 50
sigue correspondiendo únicamente a las sugerencias de autocompletado.
