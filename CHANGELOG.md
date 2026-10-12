# Changelog

Cambios notables para usuarios y colaboradores. `Unreleased` reúne el trabajo
que todavía no se ha publicado como release; `0.1.0` en los manifiestos Cargo
no implica que exista una release de esa versión. Al publicar, estas entradas
se agruparán bajo la versión y fecha correspondientes.

El [historial técnico de las fases 0–3](docs/DEVELOPMENT_HISTORY.md) conserva las
entradas originales con decisiones, referencias a la especificación y pruebas.
El estado del trabajo pendiente se mantiene en GitHub Issues.

## [Unreleased]

### Añadido

- CI del core en Linux, Windows y macOS; pruebas y compilación de la UI en
  Linux. Paquetes Linux x86_64 con licencias y checksums; los tags de versión
  generan borradores de release tras superar la matriz de pruebas.

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

### Trabajo en curso

- Integración de exportación PDF en la interfaz: documento activo desde el
  menú File, botón y `Ctrl+E`. Sigue pendiente de completar su verificación,
  incluidos el diálogo, la escritura a disco y los errores de exportación.
  La composición de varios documentos sigue prevista para la Fase 6.

Las fases 0–3 están completadas. Markdown Intelligence, funciones académicas,
composición, controles contextuales y Windows siguen en el plan de desarrollo.
