# Protocolo de benchmark — Fase 0

Normativo para la Fase 0 (§55) y ISSUE-007. Define máquina de referencia,
procedimiento de medición y formato de resultados. Los targets de §47 se calibran
y se reportan sobre esta máquina.

## 1. Máquina de referencia (v1)

Capturada el 2026-09-28:

| Componente | Valor |
|---|---|
| CPU | AMD Ryzen 5 8400F, 6 núcleos / 12 hilos, boost hasta 4.76 GHz |
| RAM | 30 GiB (+ 8 GiB swap) |
| Disco de pruebas | NVMe KINGSTON SNV3S1000G (931.5 GB, ROTA=0) |
| Placa | MSI MS-7E27 |
| SO | Ubuntu 24.04.5 LTS |
| Kernel | 7.0.0-34-generic (x86-64) |
| Sesiones gráficas | Wayland **y** X11 (§50.1): toda métrica de UI se mide en ambas |
| Toolchain | gcc 13.3.0; versión de Rust fijada en el repo al iniciar la Fase 0 |

Regla: toda medición anota fecha, commit, SO y sesión activa. Si cambia el
hardware de referencia, se añade una tabla nueva (la anterior no se borra).

## 2. Reglas generales

1. Máquina en reposo: cerrar aplicaciones de usuario y anotar la carga previa
   (load average y RAM usada) en la nota de la campaña.
2. 10 repeticiones por métrica. Se reportan mediana, mínimo y máximo. La primera
   repetición se descarta como warm-up en métricas calientes.
3. Un target de §47 se cumple si la **mediana** lo cumple.
4. Los CSV son hechos; las conclusiones se escriben en la issue correspondiente.

## 3. Métricas y procedimiento

### 3.1 Startup fría y caliente (NFR-003, NFR-004)

- Marca dentro del prototipo: de proceso lanzado a primera pintura del buffer.
- Fría: `sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches'` más vaciado de cachés
  propias del toolkit que cada prototipo documente.
- Caliente: relanzar inmediatamente sin vaciar cachés.
- Complemento: `hyperfine -r 10` para tiempo total de proceso.

### 3.2 RAM (§5.2)

- En reposo (ventana abierta, documento de 1 MB) y en edición (documento de
  20 MB, 10 s de escritura continua).
- Medir RSS y PSS (`smem` o `/proc/<pid>/status`) y pico (`VmHWM`).

### 3.3 Latencia por pulsación (NFR-005)

- Instrumentación del prototipo: timestamp del evento de tecla → frame commit.
- 2000 pulsaciones sobre un documento de 20 MB, incluyendo zonas densas (tabla
  grande, bloque de código largo).
- Reportar p50, p95 y p99. El objetivo (< 16 ms) se evalúa sobre p95.

### 3.4 Documentos grandes (NFR-006, NFR-007)

- Corpus sintético con semilla fija, compartido con ISSUE-010: 20 MB y 100 MB,
  mezcla representativa del dialecto (headings, listas, tablas, código,
  matemáticas, citas, callouts).
- Medir: tiempo de apertura, tiempo hasta el primer render de preview, reparse
  por pulsación y degradación declarada (§47.4).

### 3.5 Dependencias (§5.2, §4)

- `cargo tree -e normal` (número de crates) y `cargo tree -e normal --duplicates`.
- Tamaño del binario release en disco.
- Licencias de las dependencias de producción.

### 3.6 Integración de plataforma (§50.1)

En Wayland y X11: diálogos nativos, clipboard y pegado de imágenes (§18.2),
HiDPI y arrastre de archivos desde el explorador del sistema.

## 4. Formato de resultados

Un CSV por campaña y prototipo: `resultados-<prototipo>-<AAAA-MM-DD>.csv`.

Columnas:

```text
prototipo,commit,fecha,metrica,contexto,mediana,min,max,unidad,sesion
```

Ejemplo:

```text
ui-webview,a1b2c3d,2026-10-02,startup_fria,doc_1MB,210,180,260,ms,wayland
ui-webview,a1b2c3d,2026-10-02,latencia_tecla_p95,doc_20MB,11,7,24,ms,x11
```

`contexto` identifica el escenario (`doc_1MB`, `doc_20MB`, `doc_100MB`, `reposo`,
`edicion_10s`…). La comparativa A/B que cierra ISSUE-011 se construye uniendo
estos CSV.
