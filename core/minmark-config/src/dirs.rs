//! Descubrimiento centralizado de los directorios del usuario por sistema
//! operativo (§45.1, §14.2).
//!
//! Un único módulo decide las bases de configuración, datos y estado en los
//! tres sistemas soportados, para que la UI, el índice, los snapshots y la
//! detección de tema lean siempre el mismo sitio:
//!
//! | Base | Linux/Unix (XDG) | Windows | macOS |
//! | --- | --- | --- | --- |
//! | configuración | `$XDG_CONFIG_HOME` o `~/.config` | `%APPDATA%` | `~/Library/Application Support` |
//! | datos | `$XDG_DATA_HOME` o `~/.local/share` | `%LOCALAPPDATA%` | `~/Library/Application Support` |
//! | estado | `$XDG_STATE_HOME` o `~/.local/state` | `%APPDATA%` | `~/Library/Application Support` |
//!
//! En Linux las ubicaciones existentes son byte-idénticas a las del primer
//! release (§50.1: Linux primero): `$XDG_DATA_HOME`/`$XDG_STATE_HOME` solo se
//! aceptan absolutos (§45.1) y cada rama conserva su filtro histórico
//! exacto. No se introducen variables nuevas: solo `std` (§4.1).

use std::ffi::OsString;
use std::path::PathBuf;

/// Estructura de directorios del usuario que aplica minmark en cada SO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// XDG: Linux y los demás Unix no contemplados (§50.1).
    Xdg,
    /// `%APPDATA%`/`%LOCALAPPDATA%` en Windows.
    Windows,
    /// `~/Library/Application Support` en macOS.
    MacOs,
}

/// Variables de entorno que alimentan el descubrimiento.
///
/// Se inyectan en los tests para validar las tres ramas sin mutar el entorno
/// del proceso (compartido entre los tests en paralelo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub xdg_config_home: Option<OsString>,
    pub xdg_data_home: Option<OsString>,
    pub xdg_state_home: Option<OsString>,
    pub app_data: Option<OsString>,
    pub local_app_data: Option<OsString>,
    pub home: Option<OsString>,
}

impl Env {
    /// Variables del proceso en ejecución; las no definidas cuentan como `None`.
    pub fn from_process() -> Self {
        Self {
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
            xdg_state_home: std::env::var_os("XDG_STATE_HOME"),
            app_data: std::env::var_os("APPDATA"),
            local_app_data: std::env::var_os("LOCALAPPDATA"),
            home: std::env::var_os("HOME"),
        }
    }
}

/// Layout del binario en ejecución.
pub fn host_layout() -> Layout {
    if cfg!(windows) {
        Layout::Windows
    } else if cfg!(target_os = "macos") {
        Layout::MacOs
    } else {
        Layout::Xdg
    }
}

/// Base de configuración del usuario: `config.toml` (§58) y `kdeglobals` (§11).
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(host_layout(), &Env::from_process())
}

/// Base de datos del usuario: `minmark/index` e `minmark/snapshots` (§45.1).
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(host_layout(), &Env::from_process())
}

/// Base de estado del usuario: `minmark/ui.ini` (§14.2).
pub fn state_dir() -> Option<PathBuf> {
    state_dir_from(host_layout(), &Env::from_process())
}

/// [`config_dir`] con layout y entorno inyectados.
///
/// La rama XDG no filtra valores no absolutos: es el comportamiento
/// histórico byte-idéntico de `config.toml` y `kdeglobals`.
pub fn config_dir_from(layout: Layout, env: &Env) -> Option<PathBuf> {
    match layout {
        Layout::Xdg => env.xdg_config_home.clone().map(PathBuf::from).or_else(|| {
            env.home
                .clone()
                .map(|home| PathBuf::from(home).join(".config"))
        }),
        Layout::Windows => absolute(env.app_data.clone()),
        Layout::MacOs => application_support(&env.home),
    }
}

/// [`data_dir`] con layout y entorno inyectados (§45.1: solo absolutos).
pub fn data_dir_from(layout: Layout, env: &Env) -> Option<PathBuf> {
    match layout {
        Layout::Xdg => xdg_absolute(env.xdg_data_home.clone()).or_else(|| {
            xdg_absolute(env.home.clone()).map(|home| home.join(".local").join("share"))
        }),
        Layout::Windows => absolute(env.local_app_data.clone()),
        Layout::MacOs => application_support(&env.home),
    }
}

/// [`state_dir`] con layout y entorno inyectados.
///
/// La rama XDG acepta `$HOME/.local/state` aunque `HOME` no sea absoluto: es
/// el comportamiento histórico byte-idéntico de `ui.ini`.
pub fn state_dir_from(layout: Layout, env: &Env) -> Option<PathBuf> {
    match layout {
        Layout::Xdg => xdg_absolute(env.xdg_state_home.clone()).or_else(|| {
            env.home
                .clone()
                .map(|home| PathBuf::from(home).join(".local").join("state"))
        }),
        Layout::Windows => absolute(env.app_data.clone()),
        Layout::MacOs => application_support(&env.home),
    }
}

/// Valor de variable convertido a ruta solo si es absoluta (§45.1: una base
/// relativa caería dentro del proyecto). Absolutitud según el host: la usan
/// las ramas Windows y macOS, que sí corren en el host.
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Absolutitud con semántica XDG (§45.1) para las ramas `Layout::Xdg`: la
/// lógica XDG es POSIX, donde absoluta es «empieza por `/`», así que el
/// comportamiento no depende del host (`Path::is_absolute` de Windows
/// rechazaría `/home/ana`). `is_absolute` cubre además las rutas nativas del
/// host con que los tests ejercitan la rama (`C:\tmp\…` en Windows); en
/// Linux y macOS `/…` ya es absoluto, de modo que la rama sigue siendo
/// byte-idéntica al comportamiento histórico.
fn xdg_absolute(value: Option<OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() || path.starts_with("/"))
}

/// `~/Library/Application Support` en macOS, con `HOME` absoluto (§45.1).
fn application_support(home: &Option<OsString>) -> Option<PathBuf> {
    absolute(home.clone()).map(|home| home.join("Library").join("Application Support"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ruta absoluta válida en el SO que ejecuta el test.
    fn abs(nombre: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\tmp").join(nombre)
        } else {
            PathBuf::from("/tmp").join(nombre)
        }
    }

    fn env(xdg: Option<OsString>, home: Option<OsString>) -> Env {
        Env {
            xdg_data_home: xdg.clone(),
            xdg_state_home: xdg.clone(),
            xdg_config_home: xdg,
            home,
            ..Env::default()
        }
    }

    #[test]
    fn xdg_config_sin_filtros_como_config_toml_y_kdeglobals() {
        // Byte-idéntico a las ramas históricas: XDG manda si está definido,
        // aunque no sea absoluto; si no, ~/.config.
        assert_eq!(
            config_dir_from(Layout::Xdg, &env(Some(abs("cfg").into_os_string()), None)),
            Some(abs("cfg"))
        );
        assert_eq!(
            config_dir_from(
                Layout::Xdg,
                &env(
                    Some(OsString::from("cfg-relativo")),
                    Some(OsString::from("/home/ana"))
                )
            ),
            Some(PathBuf::from("cfg-relativo"))
        );
        assert_eq!(
            config_dir_from(Layout::Xdg, &env(None, Some(OsString::from("/home/ana")))),
            Some(PathBuf::from("/home/ana").join(".config"))
        );
        assert_eq!(config_dir_from(Layout::Xdg, &env(None, None)), None);
    }

    #[test]
    fn xdg_data_rechaza_relativos_y_usa_home() {
        // §45.1: un XDG_DATA_HOME no absoluto se ignora (byte-idéntico a
        // index/snapshots).
        assert_eq!(
            data_dir_from(Layout::Xdg, &env(Some(abs("data").into_os_string()), None)),
            Some(abs("data"))
        );
        assert_eq!(
            data_dir_from(
                Layout::Xdg,
                &env(
                    Some(OsString::from("data-relativa")),
                    Some(OsString::from("/home/ana"))
                )
            ),
            Some(PathBuf::from("/home/ana").join(".local").join("share"))
        );
        assert_eq!(
            data_dir_from(Layout::Xdg, &env(None, Some(OsString::from("/home/ana")))),
            Some(PathBuf::from("/home/ana").join(".local").join("share"))
        );
        for (xdg, home) in [
            (None, None),
            (Some(OsString::from("rel")), None),
            (None, Some(OsString::from("rel"))),
            (Some(OsString::from("rel")), Some(OsString::from("rel"))),
        ] {
            assert_eq!(
                data_dir_from(Layout::Xdg, &env(xdg, home)),
                None,
                "sin valores absolutos no hay base de datos fuera del proyecto (§45.1)"
            );
        }
    }

    #[test]
    fn xdg_state_filtro_historico_de_ui_ini() {
        // Byte-idéntico a `ui.ini`: XDG_STATE_HOME filtrado por absolutitud,
        // el fallback HOME sin filtrar.
        assert_eq!(
            state_dir_from(Layout::Xdg, &env(Some(abs("state").into_os_string()), None)),
            Some(abs("state"))
        );
        assert_eq!(
            state_dir_from(
                Layout::Xdg,
                &env(
                    Some(OsString::from("state-relativa")),
                    Some(OsString::from("/home/ana"))
                )
            ),
            Some(PathBuf::from("/home/ana").join(".local").join("state"))
        );
        assert_eq!(
            state_dir_from(
                Layout::Xdg,
                &env(None, Some(OsString::from("home-relativo")))
            ),
            Some(PathBuf::from("home-relativo").join(".local").join("state"))
        );
        assert_eq!(state_dir_from(Layout::Xdg, &env(None, None)), None);
    }

    #[test]
    fn windows_usa_appdata_y_localappdata() {
        let e = Env {
            app_data: Some(abs("Roaming").into_os_string()),
            local_app_data: Some(abs("Local").into_os_string()),
            // En Windows el entorno XDG no participa.
            xdg_data_home: Some(abs("xdg").into_os_string()),
            ..Env::default()
        };
        assert_eq!(config_dir_from(Layout::Windows, &e), Some(abs("Roaming")));
        assert_eq!(state_dir_from(Layout::Windows, &e), Some(abs("Roaming")));
        assert_eq!(data_dir_from(Layout::Windows, &e), Some(abs("Local")));
        let sin_appdata = Env::default();
        assert_eq!(config_dir_from(Layout::Windows, &sin_appdata), None);
        assert_eq!(state_dir_from(Layout::Windows, &sin_appdata), None);
        assert_eq!(data_dir_from(Layout::Windows, &sin_appdata), None);
    }

    #[test]
    fn macos_usa_application_support() {
        let e = Env {
            home: Some(abs("Usuarios/ana").into_os_string()),
            xdg_data_home: Some(abs("xdg").into_os_string()),
            ..Env::default()
        };
        let esperado = Some(
            abs("Usuarios/ana")
                .join("Library")
                .join("Application Support"),
        );
        assert_eq!(config_dir_from(Layout::MacOs, &e), esperado);
        assert_eq!(data_dir_from(Layout::MacOs, &e), esperado);
        assert_eq!(state_dir_from(Layout::MacOs, &e), esperado);
        // Sin HOME (o con uno no absoluto) no hay base utilizable.
        assert_eq!(config_dir_from(Layout::MacOs, &Env::default()), None);
        let relativo = Env {
            home: Some(OsString::from("relativo")),
            ..Env::default()
        };
        assert_eq!(data_dir_from(Layout::MacOs, &relativo), None);
    }

    #[test]
    fn layout_del_host() {
        if cfg!(windows) {
            assert_eq!(host_layout(), Layout::Windows);
        } else if cfg!(target_os = "macos") {
            assert_eq!(host_layout(), Layout::MacOs);
        } else {
            assert_eq!(host_layout(), Layout::Xdg);
        }
    }

    #[test]
    fn entorno_del_proceso_con_ruotas_completas() {
        // En los tres SO de CI alguna base existe; el resultado, si lo hay,
        // es una ruta absoluta.
        for base in [config_dir(), data_dir(), state_dir()]
            .into_iter()
            .flatten()
        {
            assert!(base.is_absolute(), "{base:?} debe ser absoluta");
        }
        // La lectura del entorno no muta el proceso.
        assert_eq!(Env::from_process(), Env::from_process());
    }
}
