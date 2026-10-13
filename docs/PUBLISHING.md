# Preparar una publicación en GitHub

Esta guía se refiere a publicar el repositorio y, más adelante, una release.
El repositorio público es [ximocm/minmark](https://github.com/ximocm/minmark).
El seguimiento se realiza en [GitHub Issues](https://github.com/ximocm/minmark/issues).

## Historial inicial limpio

El 2026-10-13 se reinició el historial con un único commit inicial que contiene
el estado actual del proyecto.

Los prototipos y benchmarks históricos, incluidos sus corpus, se retiraron del
disco y del árbol de trabajo. Las decisiones de arquitectura y el historial
técnico se conservan. `target/` y `dist/` siguen excluidos de Git.

Para revisar los blobs grandes alcanzables por las referencias locales:

```sh
git rev-list --objects --all |
  git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' |
  awk '$1 == "blob" && $2 > 50 * 1024 * 1024'
```

## Antes del primer push

- Comprobar que los archivos de compilación no se versionan y que el historial
  que se publicará no contiene blobs grandes.
- Revisar `git status` y el diff: incorporar conscientemente los cambios locales
  en curso. La integración PDF debe conservar su estado real.
- Ejecutar los comandos de [BUILDING.md](../BUILDING.md), incluyendo build
  release y una comprobación manual de la app si se va a distribuir un binario.
- Mantener ambos `Cargo.lock`, `LICENSE`, `NOTICE` y la documentación en el commit.
- Revisar `git remote -v` y configurar la URL del repositorio de destino.
- Habilitar GitHub Issues para el seguimiento y usar las plantillas de `.github/`.
  Crear una issue para la verificación pendiente de exportación PDF descrita en
  el changelog y agrupar el trabajo por milestones cuando corresponda.

## Preparar una release

1. Elegir una versión y mantener coherentes los manifiestos del core, UI y
   referencias internas entre crates; actualizar los lockfiles afectados.
2. Pasar las entradas terminadas de `Unreleased` a una sección con versión y
   fecha reales. Dejar el trabajo pendiente fuera de las notas de la release.
3. Compilar y probar el commit exacto que se etiquetará.
4. Si se distribuyen binarios, incluir `LICENSE`, `NOTICE`, los avisos necesarios de
   dependencias y fuentes, y especificar arquitectura, distribución y
   bibliotecas compartidas requeridas.
5. Crear la etiqueta y la release con notas de cambios y limitaciones conocidas.

La licencia del proyecto es Apache-2.0. La descripción del repositorio debe
identificar minmark como software open source bajo Apache License 2.0.
