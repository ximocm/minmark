# Changelog

Cambios notables para usuarios y colaboradores. `Unreleased` reúne el trabajo
que todavía no se ha publicado como release.

El [historial técnico de las fases 0–3](docs/DEVELOPMENT_HISTORY.md) conserva las
entradas originales con decisiones, referencias a la especificación y pruebas.
El estado del trabajo pendiente se mantiene en GitHub Issues.

## [Unreleased]

Sin cambios pendientes de publicación.

## [0.2.0] — 2026-10-13

### Añadido

- Navegación de enlaces con Ctrl+Click/Ctrl+Enter en Markdown y Visual y desde
  Preview; autocompletado de archivos y headings, diagnósticos de enlaces rotos,
  wiki links opcionales y vistas temporales de outline y backlinks.
- Referencias cruzadas a IDs de headings, figuras, tablas y ecuaciones,
  coherentes en Preview/PDF, con idioma español/inglés y numeración configurable.
- Pies de figura numerados a partir del texto alternativo. El PDF incorpora
  las imágenes locales mediante recursos en memoria.
- Resolución compartida de destinos y anclas, con buffers sin guardar,
  archivos ignorados, Unicode y límites del proyecto. Índice reconstruible v3
  con IDs de objetos y timestamps con nanosegundos.
- CI del core en Linux, Windows y macOS; compilación y paquete de la app Linux
  x86_64 con licencias y checksums.

### Corregido

- Preview se actualiza sin recargar la página y sigue el bloque editado cuando
  queda fuera de vista, conservando el scroll en otros refrescos.
- Las flechas mantienen visible la selección en Quick Open, outline, backlinks
  y autocompletado, sin quitar el foco al campo de escritura.

### Verificación y limitaciones

- 335 pruebas del core, 89 de UI y una integración gráfica GTK pasan. Revisión
  manual de los siete pasos de Markdown Intelligence y proyecto de 1000 notas.
- Exportación PDF revisada con buffer sin guardar, menú/atajo, cancelación,
  sobrescritura, destino no escribible y recuperación tras un error del motor.
- Pendientes las comprobaciones con lector de pantalla, Wayland y confirmación
  manual específica del seguimiento de Preview al escribir; este último tiene
  cobertura gráfica automatizada. Detalles en
  [la guía de fase 4](docs/MARKDOWN_INTELLIGENCE.md).
- La aplicación distribuida es para Linux x86_64 (Ubuntu 24.04 y compatibles),
  con GTK ≥ 4.10 y WebKitGTK 6.0. Windows/macOS tienen pruebas de core, no binarios
  de escritorio. Funciones académicas ampliadas, composición y controles
  contextuales siguen en desarrollo.

## [0.1.1] — 2026-10-13

### Cambiado

- Exportación PDF desde `File → Export PDF` y `Ctrl+E`, sin botón adicional
  en la barra de menús.
- Core y UI comparten `target/` para reutilizar dependencias compiladas.
  Desarrollo omite información de depuración y caché incremental; release
  elimina los símbolos del ejecutable para reducir el espacio ocupado.
- Guía de compilación e instalación actualizada, con instrucciones para
  liberar los archivos de compilación después de copiar el ejecutable.

### Eliminado

- Prototipos de la Fase 0 y benchmarks históricos, incluidos los corpus locales.
  Las decisiones de arquitectura conservan sus métricas de referencia.

### Corregido

- Error de compilación por la referencia a `gtk::Button` del botón PDF retirado.

## [0.1.0]

### Añadido

- `CODEOWNERS` con `@ximocm` como revisor de todos los archivos y flujo de
  contribución por pull request para el repositorio público.

- Aplicación Linux en Rust con GTK4 y WebKitGTK: pestañas, ventanas múltiples,
  apertura de archivos y carpetas, diálogos nativos y atajos de teclado.
- Edición Markdown con deshacer/rehacer y modo Visual editable sobre el mismo
  documento. Visual admite encabezados, énfasis, enlaces, código, citas, listas,
  tablas, etiquetas de imagen, callouts y colores inline; permite revelar la
  sintaxis del elemento bajo el cursor.
- Parser CommonMark/GFM con extensiones propias, vista previa HTML y temas
  claro, oscuro y del sistema.
- Modo proyecto: explorador, importación de Markdown, operaciones de archivos,
  actualización de referencias al renombrar o mover, índice incremental y
  detección de cambios externos.
- Búsqueda y reemplazo en archivo y proyecto, opciones de expresiones regulares
  y reemplazo masivo deshacible.
- Quick Open con búsqueda fuzzy, archivos recientes, salto a línea y búsqueda
  de encabezados del documento o proyecto.
- Guardado atómico, preservación de LF/CRLF, validación de rutas y
  almacenamiento de snapshots en el core.
- Motor PDF básico del core basado en Typst, fuentes embebidas y traducción
  de un subconjunto de LaTeX a Typst para matemáticas.
- Especificación, decisiones de arquitectura y
  prototipos con resultados de benchmarks.
- Guías de compilación, contribución y publicación en GitHub, índice de
  documentación y plantillas de errores, propuestas y pull requests.

### Cambiado

- Licencia del proyecto cambiada de PolyForm Noncommercial 1.0.0 a Apache-2.0.
  Actualizados los manifiestos, la documentación y el diálogo Acerca de;
  añadido `NOTICE` con la atribución de autoría.

- Seguimiento trasladado a GitHub Issues; eliminados los archivos de issues
  locales y actualizado el flujo de contribución y las plantillas.

- Historial Git reiniciado con un commit inicial limpio; corpus de benchmarks
  conservados localmente y excluidos del repositorio.

- README reorganizado para distinguir funciones disponibles de trabajo futuro.
- Documentación de arquitectura y prototipos actualizada; comandos desde la
  raíz del repositorio con manifiestos explícitos y lockfiles conservados.
- Historial técnico extenso separado de las notas de cambios para usuarios.
- Especificación actualizada hasta SRS v1.3: gramática de extensiones,
  configuración y gestión de límites de snapshots.

### Corregido

- Comandos con diálogos que no respondían por una referencia débil inválida.
- Fallo de arranque de WebKit en sistemas donde no funciona `bwrap`: se detecta
  la situación y se desactiva su sandbox. La limitación de aislamiento se
  explica en [BUILDING.md](BUILDING.md).
