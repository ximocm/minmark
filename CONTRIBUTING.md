# Contribuir a minmark

Gracias por ayudar a mejorar minmark. El desarrollo y la documentación se
mantienen principalmente en español. Empieza por el [README](README.md), la
[guía de compilación](BUILDING.md). El seguimiento se realiza en la pestaña
[Issues de GitHub](https://github.com/ximocm/minmark/issues).

## Errores y propuestas

Para informar de un error, incluye los pasos para reproducirlo, el resultado
esperado y el observado, el commit o versión, la distribución, el entorno gráfico
(Wayland/X11) y las versiones de GTK/WebKitGTK. Adjunta un ejemplo Markdown
mínimo cuando sea relevante; evita incluir documentos o rutas con datos privados.

Para proponer una función, explica el problema que resuelve y revisa si aparece
en la [especificación](docs/custom_markdown_app_SRS_v1.3.md). La especificación
incluye trabajo futuro: no todo lo descrito está implementado.

Abre una GitHub Issue usando la plantilla de error o propuesta correspondiente.
Busca primero si ya existe una issue sobre el mismo tema. Las decisiones de
arquitectura duraderas se documentan también en `docs/adr/` y los cambios de
producto en la especificación.

## Preparar un cambio

Las contribuciones externas se envían desde un fork mediante pull request.
`@ximocm` es el propietario y revisor de todos los archivos, según
[CODEOWNERS](.github/CODEOWNERS). Los cambios en `main` requieren su revisión;
las nuevas modificaciones invalidan las aprobaciones anteriores. El propietario
conserva la capacidad de integrar sus propios cambios como administrador.

1. Crea una rama con un objetivo concreto.
2. Revisa la especificación y las decisiones de arquitectura que afecten al cambio.
3. Implementa el cambio y añade pruebas para comportamientos nuevos o errores
   corregidos cuando sean necesarias. Para cambios visuales, comprueba la app real.
4. Ejecuta los tests del workspace afectado siguiendo [BUILDING.md](BUILDING.md).
5. Actualiza la documentación y añade una entrada en `CHANGELOG.md` bajo
   `Unreleased` si el cambio afecta a usuarios o colaboradores.
6. Abre una pull request explicando el problema, el comportamiento resultante,
   cómo lo has verificado y las limitaciones pendientes.

Evita mezclar refactorizaciones o reformateos generales con un cambio puntual.
Conserva los lockfiles y revisa sus diferencias si cambias dependencias.

## Principios técnicos

- Markdown es la fuente de verdad; los modos de edición comparten el documento.
- Conserva la fidelidad del texto y los saltos de línea al guardar.
- Las operaciones de archivo deben manejar cambios externos y errores de E/S.
- El core Rust mantiene una responsabilidad por crate; GTK/WebKit pertenecen a la UI.
- Justifica las dependencias nuevas y comprueba su compatibilidad con el proyecto.
- Respeta el diseño local, sin cuentas, telemetría ni servicios de red automáticos.

## Seguimiento en GitHub

Para un cambio de alcance, abre primero una issue con el problema, el resultado
esperado y los criterios de aceptación. Una issue debe cubrir un solo tema.
Utiliza labels para clasificar el trabajo y milestones para agrupar las fases
de la especificación.

Relaciona las pull requests con sus issues mediante `#N` o su URL. Usa
`Closes #N` cuando la PR resuelva por completo la issue; si queda trabajo
pendiente, descríbelo antes de cerrarla. Registra la verificación en la PR.
Una corrección pequeña de documentación no necesita una propuesta de producto
nueva.

Los identificadores `ISSUE-NNN` que aparecen en comentarios de código y en el
[historial técnico](docs/DEVELOPMENT_HISTORY.md) pertenecen al seguimiento
anterior. Se conservan como contexto histórico y no equivalen a los números de
GitHub. No se mantienen archivos locales de issues.

## Licencia

Las contribuciones al código del proyecto se distribuyen bajo la misma
[Apache License 2.0](LICENSE). Aporta únicamente contenido que
puedas compartir bajo esos términos y conserva los avisos de terceros cuando
corresponda. No cambies la licencia del proyecto como parte de una modificación
no relacionada.
