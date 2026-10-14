# Fase 4: comprobaciones de cierre

Ejecución del 14 de octubre de 2026 por Codex. Alcance: los tres pendientes
del [plan #3](https://github.com/ximocm/minmark/issues/3): lector de pantalla,
Wayland y scroll de Preview. Se operaron ventanas reales con teclado y ratón
mediante XTest y AT-SPI; las capturas se inspeccionaron visualmente.

## Entorno y versiones

| Campo | Valor |
| --- | --- |
| Sistema de ejecución | Ubuntu 24.04.5 LTS, Linux 7.0.0-34-generic, x86_64 |
| GTK / WebKitGTK | 4.14.5 / 2.52.6 |
| Lector de pantalla | Orca 46.1, AT-SPI 2.52.0 |
| Voz | Speech Dispatcher 0.12.0-rc2, espeak-ng, servidor dedicado de QA |
| Wayland | Weston 13.0.0 con renderer Pixman, anidado sobre Xvfb |
| Aplicación | v0.2.0, commit `12c7a0f369c77e54c8d1d0abd8f1dfd9fc21c866` |
| Repetición tras corrección | Mismo código de v0.2.0 y únicamente el parche de `ui/src/editor/completion.rs` |
| SHA-256 del binario corregido | `dc9d30a8f1d371e8f1400dac8c2605e307afce60431ffd04bd3737b95fc82ae2` |
| Configuración y datos | Directorios temporales bajo `/tmp/minmark-fase4-qa` |

El paquete publicado se descargó de la release oficial. Su SHA-256 coincide
con `SHA256SUMS`: `17e6ffd3eb2991c47b30b66ad5fa0cf1814c0a7f141d55a9e19fe09e68cbdbb3`.
La compilación de la corrección utiliza una copia aislada del tag y los
lockfiles de v0.2.0; no incorpora los cambios locales de fase 5.

Minmark se ejecutó con `GDK_BACKEND=wayland`, socket de Weston y sin XWayland.
Esta comprobación cubre un cliente Wayland real en un compositor anidado;
no acredita una sesión de escritorio nativa de GNOME, KDE o Cinnamon.
Orca accedió a la aplicación mediante el bus AT-SPI privado de la ejecución.

## Datos y recorrido

Proyecto desechable: `main.md` con 100 párrafos, `beta.md` con headings
«Destino» y «Segundo», `bravo.md`, y `objects.md` con enlaces Markdown/wiki,
un enlace inexistente, referencias a heading/figura/tabla/ecuación e imagen SVG
local. Los documentos reales y la configuración del usuario se conservaron.

1. Abrir `main.md` mediante Quick Open; editar sin guardar.
2. Recorrer enlaces en Markdown, Visual y Preview; consultar backlinks y outline.
3. Revisar scroll de Preview con el documento largo.
4. Revisar referencias y exportar `objects.md` a un PDF temporal.
5. Reproducir el fallo de anuncios de autocompletado con Orca; repetir tras corregirlo.

## Resultados

| Caso | Prioridad | Versión / estado | Resultado y evidencia |
| --- | --- | --- | --- |
| F4-A11Y-01: nombres de las sugerencias con flechas | P0 | v0.2.0: **Fallido** | Orca lee el editor y las teclas, pero no anuncia los cambios entre `beta.md` y `bravo.md`. Registro de voz anterior al parche. |
| F4-A11Y-01: repetición | P0 | v0.2.0 + parche: **Pasado** | Orca anuncia «Sugerencia: beta.md», «Sugerencia: bravo.md» y vuelve a «beta.md» con Arriba. También anuncia «Destino» y «Segundo» en las sugerencias de headings. |
| F4-A11Y-02: aceptar, deshacer y cancelar | P0 | v0.2.0 + parche: **Pasado** | Tab acepta `[qa](bravo.md)`; Ctrl+Z restaura `[qa](b)`. Enter acepta `beta.md#segundo`. Escape cierra un popup activo conservando `[qa](b)` y el cursor. Capturas antes/después. |
| F4-WL-01: navegación y consultas | P1 | v0.2.0: **Pasado** | Navegación a `beta.md#destino` desde Markdown, Visual, Preview y wiki links. La pestaña modificada se conserva. Backlinks muestra el origen; outline muestra H1/H2. Escape cierra las vistas. |
| F4-WL-02: diagnósticos y referencias/PDF | P1 | v0.2.0: **Pasado en el alcance de fase 4** | El destino inexistente muestra un error. Preview y PDF resuelven «Objetos, Figura 1, Tabla 1 y Ecuación 1»; el PDF incluye imagen local y pie de figura. PDF reabierto, texto extraído y página renderizada e inspeccionada. |
| F4-SCROLL-01: refrescos generales | P1 | v0.2.0: **Pasado** | Preview permanece en los párrafos 058–070 al alternar Markdown/Visual con el cursor en otra ubicación. Capturas `wayland-scroll-before-refresh` y `wayland-scroll-after-refresh`. |
| F4-SCROLL-02: seguir el bloque editado | P1 | v0.2.0: **Pasado** | Desde el inicio, editar al final del documento desplaza Preview hasta `QA-scroll-A`. |
| F4-SCROLL-03: continuar escribiendo | P1 | v0.2.0: **Pasado** | Añadir `QA-scroll-B` al mismo bloque actualiza el contenido y mantiene visibles los mismos párrafos, sin volver al inicio. |

## Corrección y repetición

El popup mantiene el foco en el editor. La opción seleccionada tiene nombre
accesible y se vincula como descendiente activo. Durante el popup, el nombre
accesible del editor expone la sugerencia actual; al cerrarlo se restablece.
Este cambio produce los anuncios de selección en Orca 46.1/GTK 4.14.5 sin
nuevas dependencias ni elevar el mínimo de GTK 4.10.

Extracto de la repetición registrada por Orca:

```text
11:11:48.578319 SPEECH OUTPUT: Sugerencia: beta.md
11:11:49.120152 SPEECH OUTPUT: Sugerencia: bravo.md
11:11:51.150175 SPEECH OUTPUT: Sugerencia: beta.md
11:38:23.842900 SPEECH OUTPUT: Sugerencia: Destino
11:38:24.381249 SPEECH OUTPUT: Sugerencia: Segundo
```

Los primeros intentos con el servicio de voz automático de la sesión aislada
no permitieron completar la lectura. Se resolvió usando un Speech Dispatcher
dedicado; se contrastó con un control GTK independiente antes de reproducir
el fallo de Minmark. Esos intentos no se cuentan como pruebas pasadas.

## Validación ejecutable

Desde la copia aislada de v0.2.0, con el parche final:

```sh
cargo build --offline --locked --release --manifest-path ui/Cargo.toml \
  --target-dir /tmp/minmark-fase4-qa/target
cargo test --offline --locked --release --manifest-path ui/Cargo.toml \
  --target-dir /tmp/minmark-fase4-qa/target
cargo test --offline --locked --release --manifest-path ui/Cargo.toml \
  --target-dir /tmp/minmark-fase4-qa/target \
  gtk_navigation_outline_completion_and_dirty_buffers \
  -- --ignored --test-threads=1 --nocapture
```

Build correcto; **89 pruebas de UI pasan**, con la prueba gráfica ignorada en
la ejecución ordinaria. Ejecutada explícitamente en Wayland, **1 integración
gráfica pasa**, incluidos navegación, buffers modificados, aceptación/undo,
scroll y listas de resultados. `rustfmt --check` del archivo modificado y
`git diff --check` pasan. No se repitió la suite completa del core: el parche
solo afecta al popup de UI. La prueba gráfica no comprueba los anuncios de
Orca; estos se verificaron en la interacción y el registro de voz descritos.

## Evidencias y límites

Capturas, datos, extractos de voz, parche y log gráfico se conservan en el
paquete local `/tmp/minmark-fase4-qa/evidence.zip`. No está publicado en GitHub.

En el PDF publicado v0.2.0 también se observó que la tabla de dos columnas
se presenta como una columna con cuatro filas. Las referencias y el contenido
se conservan; el layout de tablas queda fuera del parche de autocompletado.
Se deja esta observación para el trabajo de PDF de fase 6.

Las tres comprobaciones de cierre están ejecutadas. La corrección de
accesibilidad está validada localmente y queda pendiente de integración y
publicación; v0.2.0 conserva el fallo inicial. El plan #3 debe conservar ese
pendiente hasta integrar la corrección.
