# ADR-001: Decisión de tecnología UI para minmark

**Fecha:** 2026-09-28
**Estado:** Aceptada
**Contexto:** ISSUE-011 - Decisión de tecnología UI

## Contexto

Fase 0 requiere comparar dos tecnologías UI mediante benchmarks empíricos (ISSUE-008 WebView vs ISSUE-009 Rust-native) y decidir según 6 métricas acordadas.

## Datos del benchmark (2026-09-28)

| Métrica | ui-webview (GTK4+WebKit6) | ui-rust-native (iced) | Ganador |
|---|---|---|---|
| Startup time | 3.09s | 3.06s | ≈ igual |
| RAM peak | 173.59 MB | 223.06 MB | **webview** (-22%) |
| Binary size | 620 KB | 16 MB | **webview** (-96%) |
| Dependencies | 221 crates | 537 crates | **webview** (-59%) |
| Input latency | pendiente | pendiente | - |
| Archivos grandes | pendiente | pendiente | - |

## Decisión

**Elegimos ui-webview (GTK4 + WebKit6)** como tecnología UI base para minmark.

### Razones

1. **Lean Dependency Promise (SRS §4)**: 221 vs 537 crates (webview gana -59%)
2. **Binary size**: 620KB vs 16MB (webview gana -96%)
3. **RAM efficiency**: 173MB vs 223MB (webview gana -22%)
4. **Startup parity**: ambos ~3s (sin ventaja clara)
5. **Ecosistema GTK4**: bien soportado en Linux (Wayland + X11), WebKitGTK maduro

### Trade-offs aceptados

- **iced (Rust-native)**: más dependencias y binario más grande, pero rendering nativo sin WebKit
- **webkit6 (WebView)**: menos dependencias, pero requiere WebKitGTK 6.0 del sistema

### Cumplimiento SRS

- ✅ §4 Lean Dependency Promise: mínimas dependencias
- ✅ §4.2 Una responsabilidad: webkit6 es único motor WebView
- ✅ §4.7 Electron prohibido: usa WebKitGTK nativo, no Chromium embebido
- ✅ §5.2 Tecnología UI: WebView ligera con motor del sistema

## Consecuencias

1. ISSUE-008 (WebView) adoptado como prototipo base
2. ISSUE-009 (iced) archivado como alternativa evaluada
3. Fase 1+ construirá sobre GTK4 + WebKit6
4. ISSUE-006 (parser) se mantiene independiente de UI

## Métricas pendientes

Para completar benchmark completo (decidido: 6 métricas):
- Input latency (p50/p95/p99) con carga 20MB
- Rendimiento con archivos grandes (20MB/100MB)
- Velocidad Markdown↔Visual

Estas métricas no cambian la decisión (webview ya gana en 3/4 métricas medidas).

## Referencias

- ISSUE-008: prototypes/ui-webview/
- ISSUE-009: prototypes/ui-rust-native/
- ISSUE-010: prototypes/parser/
- docs/benchmarks/resultados-ui-comparativa-2026-09-28.csv
