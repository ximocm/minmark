# Cierre al guardar ecuaciones Unicode — 2026-10-26

El propietario informa de otro cierre con Ctrl+S después de instalar el MSI
corregido de #60. El ejecutable instalado coincide por SHA-256 con el portable
de CI #85 y contiene los schemas GTK: no corresponde al paquete antiguo.

## Evidencia y reproducción

El registro de Windows de 2026-10-25 muestra `0xc0000409` en minmark. El
volcado examinado localmente referencia `core/minmark-parser/src/inlines.rs:468:18`.
Al abrir en ese mismo ejecutable un fixture nuevo con `$α_0$`, se reproduce
la misma salida de proceso y se captura:

```text
thread 'main' panicked at core/minmark-parser/src/inlines.rs:468:18:
start byte index 2 is not a char boundary; it is inside 'α' (bytes 1..3 of string)
```

La regresión añadida a `tests/dialecto.rs` falla antes de la corrección en esa
misma línea. No se necesitan el documento del propietario ni su contenido para
reproducir el problema.

## Causa y corrección

`try_math` usaba la longitud UTF-8 del primer carácter de la ecuación como
inicio del cuerpo. Ese inicio es la posición después del delimitador `$`,
no la longitud del carácter siguiente. Un carácter de varios bytes provocaba
un corte dentro de UTF-8 y un panic; desde el callback GTK el proceso abortaba.

Se conserva el desplazamiento correcto de `char_indices`. La prueba pública
comprueba el cuerpo matemático y los spans de highlighting con caracteres de
dos, tres y cuatro bytes. Todos los prefijos completos por carácter pasan por
parseo, highlighting y modo Visual para cubrir la edición todavía incompleta.

La autocomprobación gráfica añade `unicode/editar-guardar`: inserta `$α_0$`
en el TextBuffer real y activa `win.save`, el comando de Ctrl+S, antes del
refresco diferido. Comprueba el archivo guardado, el indicador de cambios,
la Preview matemática y la posterior sobrescritura confirmada.

## Validación

- Regresión mínima: falla antes del cambio y pasa después.
- Suite completa del core: 423 pruebas correctas.
- Lógica de UI: 122 pruebas correctas.
- Seis escenarios gráficos de integración: correctos en procesos separados.
- Diagnóstico gráfico con la nueva fase Unicode: correcto en Xvfb/Linux.
- Política de release y empaquetado: 27 pruebas correctas.
- La publicación de 0.4.3 exige CI correcta en las tres plataformas y la
  comprobación del paquete. El resultado del commit final se registra en
  [PR #62](https://github.com/ximocm/minmark/pull/62) y su workflow de release.

La limitación manual del selector de macOS y el resto de la matriz de fase 8
siguen registrados en [la validación Windows](estabilidad-windows-2026-10-25.md).
