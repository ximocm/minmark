# Preparar una publicación en GitHub

Esta guía se refiere a publicar el repositorio y, más adelante, una release.
El repositorio público es [ximocm/minmark](https://github.com/ximocm/minmark).
El seguimiento se realiza en [GitHub Issues](https://github.com/ximocm/minmark/issues).

## Historial inicial limpio

El 2026-10-13 se reinició el historial con un único commit inicial que contiene
el estado actual del proyecto. El historial anterior se conserva en una copia
local fuera del repositorio.

Los corpus históricos permanecen en disco, excluidos de Git mediante
`.gitignore`. No son necesarios para compilar ni ejecutar los tests:

| Archivo local | Bytes | Tamaño aproximado |
| --- | ---: | ---: |
| `docs/benchmarks/corpus-20MB.md` | 54 083 489 | 51,6 MiB |
| `docs/benchmarks/corpus-100MB.md` | 270 417 445 | 257,9 MiB |

GitHub advierte al subir archivos de más de 50 MiB y bloquea los de más de
100 MiB en Git normal. Véase la [documentación oficial sobre archivos grandes](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github).

Para compartir estos corpus, distribúyelos aparte o configura Git LFS antes de
incorporarlos. No los añadas con `git add -f`. Sus nombres no reflejan el tamaño
real. Las notas de desarrollo y resultados históricos siguen versionados.

Para revisar los blobs grandes alcanzables por las referencias locales:

```sh
git rev-list --objects --all |
  git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' |
  awk '$1 == "blob" && $2 > 50 * 1024 * 1024'
```

## Antes del primer push

- Comprobar que los corpus siguen excluidos y que el historial que se publicará
  no contiene blobs grandes.
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

El [workflow de releases](CI.md#crear-una-release) valida el tag, ejecuta CI y
crea un borrador con el paquete Linux y sus checksums.

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
