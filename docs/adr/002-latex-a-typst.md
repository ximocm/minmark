# ADR-002: Traducción LaTeX→Typst para matemáticas

**Fecha:** 2026-09-28
**Estado:** Aceptada
**Contexto:** ISSUE-004 - Traducción de matemáticas LaTeX → Typst

## Contexto

SRS §24 fija LaTeX como sintaxis de entrada de matemáticas. SRS §5.3 fija Typst como único motor de composición (§4.3 dependencia estratégica). Typst no compila LaTeX nativamente: necesita traducción explícita.

Restricciones:
- §4 Lean Dependency Promise: mínimo de dependencias
- §24 usuario no escribe Typst math
- NFR-026: sin pérdida silenciosa ante comandos no soportados

## Decisiones

1. **Estrategia**: Traductor LaTeX→Typst propio en Rust (no dependencia externa)
2. **Alcance**: Subconjunto LaTeX Math estándar (90% uso académico)
3. **Comportamiento no soportado**: Marcador de error en Preview y PDF (no silenciar)

## Alcance soportado (subconjunto estándar)

### Operadores básicos
- Fracciones: `\frac{a}{b}` → `a/b`
- Raíces: `\sqrt{x}`, `\sqrt[n]{x}` → `sqrt(x)`, `root(n, x)`
- Superíndices/índices: `x^{2}`, `x_{i}` → `x^2`, `x_i`
- Paréntesis: `\left(`, `\right)` → `(`, `)`

### Símbolos y letras griegas
- `\alpha`, `\beta`, `\gamma`, etc. → `alpha`, `beta`, `gamma`
- `\sum`, `\prod`, `\int` → `sum`, `product`, `int`
- `\infty`, `\partial`, `\nabla` → `infinity`, `diff`, `nabla`

### Operadores grandes
- Sumatorios: `\sum_{i=1}^{n}` → `sum_(i=1)^n`
- Integrales: `\int_{a}^{b}` → `int_a^b`
- Productorios: `\prod_{i=1}^{n}` → `product_(i=1)^n`

### Matrices
- `\begin{pmatrix}...\end{pmatrix}` → `mat(...)` con delimitadores
- `\begin{bmatrix}`, `\begin{vmatrix}`, etc.

### Funciones
- `\sin`, `\cos`, `\tan`, `\log`, `\ln`, `\exp`, etc. → `sin`, `cos`, `tan`, `log`, `ln`, `exp`
- `\lim_{x \to 0}` → `lim_(x -> 0)`

### Acentos
- `\hat{x}`, `\bar{x}`, `\vec{x}` → `hat(x)`, `bar(x)`, `vec(x)`

## Comportamiento no soportado

Sintaxis fuera del subconjunto estándar (ej: macros personalizadas, AMS completo):

1. **Preview**: Marcador visible `⚠ [math no soportado: <comando>]`
2. **PDF export**: Marcador en el PDF (ecuación placeholder) + warning en log
3. **Nunca**: Silenciar, omitir, o deformar la ecuación

## Implementación

En `core/minmark-pdf/` o `core/minmark-math/`:

1. Parser LaTeX math → AST (tokens, comandos, estructura)
2. Traductor AST → código Typst
3. Tabla de mapeo comunes (comandos LaTeX → Typst)
4. Error handling: comandos no soportados → marcador + warning

## Justificación dependencias (§4.4)

- **Ninguna dependencia adicional**: traductor propio en Rust
- **std-only**: parsing y generación de texto
- **Control total**: alcance y errores definidos por nosotros
- **Alternativa descartada**: crate `latex-to-typst` (no encontrado, hipotético mantendría nuestro alcance)

## Cumplimiento SRS

- ✅ §24: usuario escribe LaTeX, no Typst
- ✅ §5.3: Typst único motor PDF
- ✅ §4.1: std-first, sin dependencias extra
- ✅ §4.2: un traductor LaTeX→Typst (no múltiples)
- ✅ NFR-026: comandos no soportados visibles, no silenciosos

## Consecuencias

1. ISSUE-018 (PDF básico) puede implementarse con math
2. Alcance garantizado: subconjunto estándar (documentado)
3. Comandos avanzados requieren extensión futura (Fase 5+)

## Referencias

- ISSUE-004 (identificador histórico; véase el [historial técnico](../DEVELOPMENT_HISTORY.md)).
- SRS §24 Matemáticas
- SRS §5.3 Motor PDF
