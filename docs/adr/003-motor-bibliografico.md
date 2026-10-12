# ADR-003: Motor bibliográfico con hayagriva

**Fecha:** 2026-09-28
**Estado:** Aceptada
**Contexto:** ISSUE-005 - Motor CSL frente al Lean Dependency Promise

## Contexto

SRS §25.4 exige soporte bibliográfico con estilo IEEE por defecto, configurable con CSL. Necesitamos procesar citas Pandoc `[@smith2025]` + bibliografía `.bib` + estilos CSL.

Restricciones:
- §4 Lean Dependency Promise: mínimas dependencias
- §4.3 dependencias estratégicas: solo si resuelven funcionalidad crítica
- §5.3 Typst único motor PDF

## Opciones evaluadas

1. **citeproc-rs**: no existe en crates.io
2. **Parser propio + IEEE hardcodeado**: std-only, sin deps, pero limitado a IEEE
3. **hayagriva**: motor citas oficial de Typst, CSL completo, MIT/Apache-2.0

## Decisión

**Usar hayagriva v0.10.1** como motor bibliográfico.

### Razones

1. **Integración natural con Typst** (§5.3): hayagriva es el motor que usa Typst.app oficial
2. **CSL completo** (§25.4): soporta todos los estilos CSL, incluido IEEE
3. **Dependencia estratégica** (§4.3): resuelve bibliografía completa (crítico para Fase 1 y Fase 5)
4. **Licencia limpia**: MIT OR Apache-2.0
5. **Repositorio activo**: github.com/typst/hayagriva (misma organización que Typst)

### Features a habilitar

```toml
[dependencies]
hayagriva = { version = "0.10.1", default-features = false, features = ["biblatex"] }
```

- `biblatex`: parsing de archivos `.bib` (§25.1)
- Sin `archive`, `cli`, `csl-json` (no necesarios, §4.5 features mínimas)

### Cumplimiento §4.4 (justificación dependencia)

- **Problema que resuelve**: bibliografía completa (citas, .bib, estilos CSL)
- **Por qué no std**: CSL es un lenguaje de estilos complejo, no viable en std
- **Dependencias transitivas**: ~15 crates (biblatex parsing + CSL engine)
- **Impacto binario**: moderado (~500KB estimado)
- **Impacto compilación**: aceptable (una vez, cacheado)
- **Impacto startup**: nulo (bibliografía se carga bajo demanda)
- **Licencia**: MIT OR Apache-2.0 ✅
- **Mantenimiento**: activo (Typst org) ✅
- **Seguridad**: sin red, sin FFI riesgoso ✅
- **Compatibilidad**: Linux + Windows ✅

## Alcance soportado

### Citas (§25.2)
- `[@smith2025]` → cita simple
- `[@smith2025; @jones2024]` → múltiples
- `[@smith2025, p. 42]` → con locator

### Bibliografía (§25.1)
- Archivos `.bib` estándar (BibTeX/BibLaTeX)
- Múltiples archivos `.bib`
- Entries: article, book, inproceedings, etc.

### Estilos (§25.4)
- IEEE por defecto (estilo CSL estándar)
- Configurable con cualquier estilo CSL
- Prioridad: documento > proyecto > global > IEEE (§58.1)

### Locators
- `p.`, `pp.`, `ch.`, `sec.`, `fig.`, `tbl.`, `eq.`, etc.

## Implementación

En `core/minmark-citations/` o integrado en `core/minmark-pdf/`:

1. Parsear `.bib` con hayagriva::biblatex
2. Resolver citas Pandoc `[@key]` → hayagriva Citation
3. Formatear con estilo CSL (IEEE default)
4. Generar bibliografía final
5. Integrar con Typst para PDF (nativo)

## Cumplimiento SRS

- ✅ §25.1: archivos .bib
- ✅ §25.2: citas Pandoc
- ✅ §25.4: IEEE default + CSL configurable
- ✅ §4.1: dependencia mínima (una para toda bibliografía)
- ✅ §4.3: dependencia estratégica justificada
- ✅ §5.3: integración natural con Typst

## Consecuencias

1. ISSUE-018 (PDF básico) puede implementar citas reales
2. Fase 5 (Academic) tiene base sólida
3. Estilos CSL completos disponibles sin trabajo adicional

## Estilo IEEE

Origen: estilo CSL estándar `ieee.csl` del repositorio oficial citation-style-language/styles (CC BY-SA 3.0). Incluido con hayagriva o descargado una vez.

## Referencias

- ISSUE-005 (identificador histórico; véase el [historial técnico](../DEVELOPMENT_HISTORY.md)).
- SRS §25 Bibliografía
- hayagriva: https://github.com/typst/hayagriva
- CSL styles: https://github.com/citation-style-language/styles
