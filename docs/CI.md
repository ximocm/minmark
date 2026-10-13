# CI y releases

Los workflows se ejecutan en [GitHub Actions](https://github.com/ximocm/minmark/actions).
Las contribuciones se revisan mediante pull request; `CODEOWNERS` asigna todos
los archivos a `@ximocm`.

## Cobertura por plataforma

| Plataforma | Pruebas | Aplicación y paquete |
| --- | --- | --- |
| Ubuntu 24.04 x86_64 | Los siete crates del core y lógica de la UI | Binario release y `.tar.gz` |
| Windows Server 2022 | Seis crates portables del core | Pendiente de portabilidad de la UI y filesystem |
| macOS 14 | Seis crates portables del core | Pendiente de portabilidad de la UI y filesystem |

Los seis crates portables son `minmark-document`, `minmark-config`,
`minmark-math`, `minmark-parser`, `minmark-renderer` y `minmark-pdf`.
`minmark-fs` queda fuera de Windows/macOS en esta matriz: todavía contiene
operaciones Unix y un watcher de Linux. La UI utiliza WebKitGTK y el watcher
Linux directamente. Superar los tests del core no significa que haya una app
Windows o macOS disponible.

## Integración continua

[ci.yml](../.github/workflows/ci.yml) se ejecuta en cada PR y push a `main`.
Usa Rust estable, dependencias bloqueadas por los `Cargo.lock` y dos procesos
de compilación para limitar el uso de memoria de Typst. Las acciones están
fijadas a commits y el token del workflow solo tiene permiso de lectura.

El job **CI success** solo pasa cuando toda la matriz termina correctamente.
El job Linux prueba y compila en modo release, comprueba las bibliotecas
compartidas y empaqueta el mismo binario. El artefacto `linux-x86_64` se conserva
14 días y permite revisar el paquete desde la PR sin publicar una release.

El `.tar.gz` incluye el ejecutable, documentación de instalación, `LICENSE`,
`NOTICE`, los archivos de licencia suministrados por las dependencias y los
avisos de las fuentes embebidas de Typst. `SHA256SUMS` permite comprobar la
integridad del archivo. El binario requiere GTK ≥ 4.10 y WebKitGTK 6.0; se
compila en Ubuntu 24.04 y no es un binario estático para cualquier distribución.

## Crear una release

1. Actualiza de forma coherente la versión del core, UI y referencias internas,
   así como los lockfiles afectados. Prepara las notas del changelog.
2. Revisa y fusiona la PR en `main` tras superar CI.
3. Desde ese commit, crea y sube una etiqueta que coincida con los manifiestos:

   ```sh
   git switch main
   git pull --ff-only
   git tag -a v0.1.0 -m 'minmark 0.1.0'
   git push origin v0.1.0
   ```

4. [release.yml](../.github/workflows/release.yml) valida que el tag identifique
   un commit de `main` y que las versiones coincidan. Vuelve a ejecutar la matriz
   de CI y utiliza su paquete Linux.
5. Si todo pasa, crea un **borrador** en
   [Releases](https://github.com/ximocm/minmark/releases), con el `.tar.gz`,
   `SHA256SUMS` y notas generadas. Revisa las notas, prueba la aplicación en una
   sesión gráfica y publica el borrador cuando corresponda. Marca como
   prerelease las versiones de prueba.

El workflow solo da permiso de escritura al job que crea el borrador, después
de validar y compilar. No hace push de código ni modifica las protecciones de
`main`. Un borrador conserva la publicación final bajo control del propietario.

Para repetir una ejecución fallida, usa **Re-run jobs**, o **Run workflow**
seleccionando el mismo tag como referencia y como entrada `tag`. Si ya existe
una release para esa etiqueta, el workflow se detiene: revisa el borrador antes
de decidir si procede eliminarlo y repetir. Nunca sobrescribe artefactos de una
release existente.

## Reproducir localmente

```sh
cargo test --locked --release --manifest-path core/Cargo.toml --workspace
cargo test --locked --release --manifest-path ui/Cargo.toml
cargo build --locked --release --manifest-path ui/Cargo.toml
python3 scripts/package-linux.py
(cd dist && sha256sum --check SHA256SUMS)
```

El empaquetado necesita Python ≥ 3.11, Linux x86_64 y las dependencias de
[BUILDING.md](../BUILDING.md). Respeta `CARGO_TARGET_DIR` si está definido.
Windows/macOS pueden ejecutar las pruebas del core con
`--workspace --exclude minmark-fs`. Los tests de lógica no sustituyen la
comprobación manual de ventanas, diálogos o Preview antes de publicar.
