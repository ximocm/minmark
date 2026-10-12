# Documentación de minmark

## Empezar

- [README](../README.md): funciones disponibles, estado y primeros pasos.
- [BUILDING](../BUILDING.md): requisitos, compilación, tests e instalación local.
- [CONTRIBUTING](../CONTRIBUTING.md): errores, propuestas y flujo de contribución.
- [CHANGELOG](../CHANGELOG.md): cambios pendientes de publicación.
- [Publicación en GitHub](PUBLISHING.md): comprobaciones y archivos grandes.
- [Licencia](../LICENSE): Apache License 2.0.

## Producto y arquitectura

La [SRS v1.3](custom_markdown_app_SRS_v1.3.md) es la especificación de producto y
software. Las referencias `§N` en el código y las issues remiten a sus secciones.
Describe tanto funciones existentes como futuras: para saber qué está terminado,
consulta el [changelog](../CHANGELOG.md). El trabajo pendiente se sigue en
GitHub Issues, con milestones para las fases del desarrollo.

- [Arquitectura del core](../core/README.md): crates, interfaces y dependencias.
- [ADR-001: tecnología de UI](adr/001-tecnologia-ui.md).
- [ADR-002: LaTeX a Typst](adr/002-latex-a-typst.md).
- [ADR-003: motor bibliográfico](adr/003-motor-bibliografico.md).

## Seguimiento y mediciones

- GitHub Issues: seguimiento de errores y propuestas, según la
  [guía de contribución](../CONTRIBUTING.md#seguimiento-en-github).
- [Protocolo de benchmarks](benchmarks/PROTOCOLO.md).
- [Resultados del parser](benchmarks/resultados-parser-2026-09-28.csv) y
  [medición actualizada](benchmarks/resultados-parser-2026-09-28-actualizado.csv).
- [Comparativa de UI](benchmarks/resultados-ui-comparativa-2026-09-28.csv).
- [Prototipos de la Fase 0](../prototypes/README.md).

Los benchmarks son mediciones históricas con un entorno concreto; no garantizan
el rendimiento de la aplicación actual ni de otros equipos. Los nombres de los
corpus históricos no reflejan su tamaño real; véase [publicación](PUBLISHING.md).
