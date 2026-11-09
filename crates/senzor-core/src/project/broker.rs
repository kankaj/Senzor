use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};

use super::error::ProjectError;
use super::file::{BrokerSpec, MqttVersion, TlsSpec};
use super::model::{BrokerConfig, Credentials, TlsConfig};

/// The longest keep-alive MQTT can carry: 16-bit seconds.
const MAX_KEEP_ALIVE: Duration = Duration::from_secs(65_535);

/// Looks up an environment variable; injected so tests do not touch the real environment.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Resolves `[broker]`: credentials, env var lookup, TLS files (relative to `base_dir`).
pub fn resolve_broker(
    spec: &BrokerSpec,
    base_dir: &Path,
    env: EnvLookup<'_>,
) -> Result<BrokerConfig, Vec<ProjectError>> {
    let mut errors = Vec::new();
    if spec.host.trim().is_empty() {
        errors.push(ProjectError::broker("host must not be empty").at("host"));
    }
    if spec.port == 0 {
        errors.push(ProjectError::broker("port must not be 0").at("port"));
    }
    let keep_alive = |message: String| ProjectError::broker(message).at("keep_alive");
    if spec.keep_alive < Duration::from_secs(1) {
        errors.push(keep_alive("keep_alive must be at least 1s".into()));
    } else if spec.keep_alive.subsec_nanos() != 0 {
        errors.push(keep_alive(format!(
            "keep_alive must be whole seconds, got {}",
            humantime::format_duration(spec.keep_alive)
        )));
    } else if spec.keep_alive > MAX_KEEP_ALIVE {
        // MQTT sends the keep-alive as 16-bit seconds.
        errors.push(keep_alive(
            "keep_alive must be at most 65535 seconds (18h 12m 15s)".into(),
        ));
    }
    if let Err(e) = whole_seconds("session_expiry", spec.session_expiry) {
        errors.push(ProjectError::broker(e).at("session_expiry"));
    }
    if spec.mqtt_version == MqttVersion::V311 && !spec.session_expiry.is_zero() {
        errors.push(ProjectError::broker(needs_mqtt5("session_expiry")).at("session_expiry"));
    }
    let credentials = resolve_credentials(spec, env).unwrap_or_else(|e| {
        errors.push(e);
        None
    });
    let tls = match &spec.tls {
        Some(tls) => resolve_tls(tls, base_dir)
            .map(Some)
            .unwrap_or_else(|mut e| {
                errors.append(&mut e);
                None
            }),
        None => None,
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(broker_config(spec, credentials, tls))
}

/// `spec`'s connection settings with already resolved credentials and TLS.
pub fn broker_config(
    spec: &BrokerSpec,
    credentials: Option<Credentials>,
    tls: Option<TlsConfig>,
) -> BrokerConfig {
    BrokerConfig {
        host: spec.host.clone(),
        port: spec.port,
        client_id_prefix: spec.client_id_prefix.clone(),
        keep_alive: spec.keep_alive,
        credentials,
        tls,
        mqtt_version: spec.mqtt_version,
        session_expiry: spec.session_expiry,
    }
}

/// `duration` as MQTT's whole seconds (a `u32`), or the problem with it, naming `key`.
pub(crate) fn whole_seconds(key: &str, duration: Duration) -> Result<u32, String> {
    if duration.subsec_nanos() != 0 {
        return Err(format!(
            "{key} must be whole seconds, got {}",
            humantime::format_duration(duration)
        ));
    }
    u32::try_from(duration.as_secs())
        .map_err(|_| format!("{key} must be at most {} seconds", u32::MAX))
}

/// Why an MQTT 5 setting cannot be used: the end of every such validation problem, and the note
/// the GUI shows next to disabled MQTT 5 settings.
pub const NEEDS_MQTT5: &str = "needs MQTT 5; the broker settings use MQTT 3.1.1";

/// The problem for an MQTT 5 setting `key` in a project that uses MQTT 3.1.1.
pub(crate) fn needs_mqtt5(key: &str) -> String {
    format!("{key} {NEEDS_MQTT5}")
}

/// `duration` in MQTT's whole seconds. Validation ([`whole_seconds`]) rejects durations that do
/// not fit, so the clamp to `u32::MAX` only guards projects that skipped it.
pub(crate) fn mqtt_seconds(duration: Duration) -> u32 {
    u32::try_from(duration.as_secs()).unwrap_or(u32::MAX)
}

fn resolve_credentials(
    spec: &BrokerSpec,
    env: EnvLookup<'_>,
) -> Result<Option<Credentials>, ProjectError> {
    let password = match (&spec.password, &spec.password_env) {
        (Some(_), Some(_)) => {
            return Err(
                ProjectError::broker("use either password or password_env, not both")
                    .at("password_env"),
            );
        }
        (Some(password), None) => Some(password.clone()),
        (None, Some(var)) => Some(env(var).ok_or_else(|| {
            ProjectError::broker(format!(
                "environment variable `{var}` (password_env) is not set"
            ))
            .at("password_env")
        })?),
        (None, None) => None,
    };
    match (&spec.username, password) {
        (Some(username), password) => Ok(Some(Credentials {
            username: username.clone(),
            password: password.unwrap_or_default(),
        })),
        (None, Some(_)) => {
            Err(ProjectError::broker("a password requires a username").at("username"))
        }
        (None, None) => Ok(None),
    }
}

fn resolve_tls(spec: &TlsSpec, base_dir: &Path) -> Result<TlsConfig, Vec<ProjectError>> {
    let mut errors = Vec::new();
    let mut read = |name: &str, path: &Path| match std::fs::read(base_dir.join(path)) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            errors.push(
                ProjectError::broker(format!("cannot read tls.{name} `{}`: {e}", path.display()))
                    .at(format!("tls.{name}")),
            );
            None
        }
    };
    let ca = spec.ca_file.as_deref().and_then(|p| read("ca_file", p));
    let client_auth = match (&spec.client_cert, &spec.client_key) {
        (Some(cert), Some(key)) => {
            let cert = read("client_cert", cert);
            let key = read("client_key", key);
            cert.zip(key)
        }
        (None, None) => None,
        (cert, _) => {
            // Blame the one that is missing.
            let missing = if cert.is_some() {
                "tls.client_key"
            } else {
                "tls.client_cert"
            };
            errors.push(
                ProjectError::broker("tls.client_cert and tls.client_key must be set together")
                    .at(missing),
            );
            None
        }
    };
    if spec.client_cert.is_some() && spec.ca_file.is_none() {
        errors.push(ProjectError::broker("tls.client_cert requires tls.ca_file").at("tls.ca_file"));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let client = match &ca {
        Some(ca) => client_config(spec, ca, client_auth.as_ref()),
        // A client certificate requires a CA file, so this is the plain system-roots setup.
        None => system_client_config(),
    }
    .map_err(|e| vec![e])?;
    Ok(TlsConfig {
        ca,
        client_auth,
        client,
    })
}

/// The TLS setup with `spec`'s CA file and client certificate, already read into `ca` and
/// `client_auth`. A file that holds no PEM certificate or key is a problem here rather than
/// a failed connect in every instance.
fn client_config(
    spec: &TlsSpec,
    ca: &[u8],
    client_auth: Option<&(Vec<u8>, Vec<u8>)>,
) -> Result<Arc<ClientConfig>, ProjectError> {
    let shown = |path: &Option<std::path::PathBuf>| {
        path.as_deref()
            .map(Path::display)
            .map(|p| p.to_string())
            .unwrap_or_default()
    };
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(certificates(ca));
    if roots.is_empty() {
        return Err(no_certificate("ca_file", &shown(&spec.ca_file)));
    }
    let builder = builder()?.with_root_certificates(roots);
    let config = match client_auth {
        None => builder.with_no_client_auth(),
        Some((cert, key)) => {
            let chain = certificates(cert);
            if chain.is_empty() {
                return Err(no_certificate("client_cert", &shown(&spec.client_cert)));
            }
            let key = PrivateKeyDer::from_pem_slice(key).map_err(|_| {
                ProjectError::broker(format!(
                    "tls.client_key `{}` holds no PEM private key",
                    shown(&spec.client_key)
                ))
                .at("tls.client_key")
            })?;
            builder.with_client_auth_cert(chain, key).map_err(|e| {
                ProjectError::broker(format!("tls.client_key: {e}")).at("tls.client_key")
            })?
        }
    };
    Ok(Arc::new(config))
}

/// The TLS setup that trusts the system's certificates, built once per process: loading them
/// takes milliseconds, and validation runs on every edit in the window.
fn system_client_config() -> Result<Arc<ClientConfig>, ProjectError> {
    static SYSTEM: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    SYSTEM
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            if roots.is_empty() {
                return Err("no trusted certificates found on this system; set tls.ca_file".into());
            }
            let builder = builder().map_err(|e| e.message)?;
            Ok(Arc::new(
                builder.with_root_certificates(roots).with_no_client_auth(),
            ))
        })
        .clone()
        .map_err(|e| ProjectError::broker(e).at("tls.ca_file"))
}

fn builder() -> Result<rustls::ConfigBuilder<ClientConfig, rustls::WantsVerifier>, ProjectError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProjectError::broker(format!("TLS: {e}")))
}

/// The PEM certificates in `pem`, skipping what does not parse.
fn certificates(pem: &[u8]) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem)
        .filter_map(Result::ok)
        .collect()
}

fn no_certificate(key: &str, path: &str) -> ProjectError {
    ProjectError::broker(format!("tls.{key} `{path}` holds no PEM certificate"))
        .at(format!("tls.{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn messages(errors: Vec<ProjectError>) -> Vec<String> {
        errors.into_iter().map(|e| e.message).collect()
    }

    fn fields(errors: Vec<ProjectError>) -> Vec<Option<String>> {
        errors.into_iter().map(|e| e.field).collect()
    }

    fn broker_fields(spec: &BrokerSpec, base_dir: &Path) -> Vec<Option<String>> {
        fields(resolve_broker(spec, base_dir, &no_env).unwrap_err())
    }

    fn some(names: &[&str]) -> Vec<Option<String>> {
        names.iter().map(|n| Some(n.to_string())).collect()
    }

    #[test]
    fn broker_problems_name_their_field() {
        let spec = BrokerSpec {
            host: " ".into(),
            port: 0,
            keep_alive: Duration::from_millis(500),
            password: Some("x".into()),
            password_env: Some("Y".into()),
            ..BrokerSpec::default()
        };
        assert_eq!(
            broker_fields(&spec, Path::new(".")),
            some(&["host", "port", "keep_alive", "password_env"])
        );
        for keep_alive in [Duration::from_millis(1500), Duration::from_secs(65_536)] {
            let spec = BrokerSpec {
                keep_alive,
                ..BrokerSpec::default()
            };
            assert_eq!(broker_fields(&spec, Path::new(".")), some(&["keep_alive"]));
        }
        let spec = BrokerSpec {
            password: Some("x".into()),
            ..BrokerSpec::default()
        };
        assert_eq!(broker_fields(&spec, Path::new(".")), some(&["username"]));
        let spec = BrokerSpec {
            username: Some("dev".into()),
            password_env: Some("MQTT_PASSWORD".into()),
            ..BrokerSpec::default()
        };
        assert_eq!(
            broker_fields(&spec, Path::new(".")),
            some(&["password_env"])
        );
        let spec = BrokerSpec {
            session_expiry: Duration::from_millis(1500),
            ..BrokerSpec::default()
        };
        assert_eq!(
            broker_fields(&spec, Path::new(".")),
            some(&["session_expiry"])
        );
        let spec = BrokerSpec {
            mqtt_version: MqttVersion::V311,
            session_expiry: Duration::from_secs(60),
            ..BrokerSpec::default()
        };
        assert_eq!(
            broker_fields(&spec, Path::new(".")),
            some(&["session_expiry"])
        );
    }

    #[test]
    fn tls_problems_name_their_field() {
        let tls = |ca: Option<&str>, cert: Option<&str>, key: Option<&str>| BrokerSpec {
            tls: Some(TlsSpec {
                ca_file: ca.map(PathBuf::from),
                client_cert: cert.map(PathBuf::from),
                client_key: key.map(PathBuf::from),
            }),
            ..BrokerSpec::default()
        };
        let nowhere = Path::new("/nonexistent");
        assert_eq!(
            broker_fields(&tls(None, Some("c.pem"), None), nowhere),
            some(&["tls.client_key", "tls.ca_file"])
        );
        assert_eq!(
            broker_fields(&tls(None, None, Some("k.pem")), nowhere),
            some(&["tls.client_cert"])
        );
        assert_eq!(
            broker_fields(&tls(Some("ca.pem"), Some("c.pem"), Some("k.pem")), nowhere),
            some(&["tls.ca_file", "tls.client_cert", "tls.client_key"])
        );
    }

    #[test]
    fn defaults_resolve() {
        let broker = resolve_broker(&BrokerSpec::default(), Path::new("."), &no_env).unwrap();
        assert_eq!(broker.host, "localhost");
        assert_eq!(broker.port, 1883);
        assert_eq!(broker.client_id_prefix, "senzor-");
        assert_eq!(broker.credentials, None);
        assert_eq!(broker.tls, None);
    }

    #[test]
    fn password_comes_from_env() {
        let spec = BrokerSpec {
            username: Some("dev".into()),
            password_env: Some("MQTT_PASSWORD".into()),
            ..BrokerSpec::default()
        };
        let env = |name: &str| (name == "MQTT_PASSWORD").then(|| "secret".to_string());
        let broker = resolve_broker(&spec, Path::new("."), &env).unwrap();
        assert_eq!(
            broker.credentials,
            Some(Credentials {
                username: "dev".into(),
                password: "secret".into()
            })
        );
    }

    #[test]
    fn missing_env_var_is_an_error() {
        let spec = BrokerSpec {
            username: Some("dev".into()),
            password_env: Some("MQTT_PASSWORD".into()),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
        assert_eq!(
            messages(errors),
            ["environment variable `MQTT_PASSWORD` (password_env) is not set"]
        );
    }

    #[test]
    fn collects_all_broker_errors() {
        let spec = BrokerSpec {
            host: " ".into(),
            port: 0,
            keep_alive: Duration::from_millis(500),
            password: Some("x".into()),
            password_env: Some("Y".into()),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
        assert_eq!(
            messages(errors),
            [
                "host must not be empty",
                "port must not be 0",
                "keep_alive must be at least 1s",
                "use either password or password_env, not both",
            ]
        );
    }

    #[test]
    fn keep_alive_must_fit_mqtt_whole_seconds() {
        for (keep_alive, message) in [
            (
                Duration::from_millis(1500),
                "keep_alive must be whole seconds, got 1s 500ms",
            ),
            (
                Duration::from_secs(65_536),
                "keep_alive must be at most 65535 seconds (18h 12m 15s)",
            ),
        ] {
            let spec = BrokerSpec {
                keep_alive,
                ..BrokerSpec::default()
            };
            let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
            assert_eq!(messages(errors), [message]);
        }
        let spec = BrokerSpec {
            keep_alive: Duration::from_secs(65_535),
            ..BrokerSpec::default()
        };
        assert!(resolve_broker(&spec, Path::new("."), &no_env).is_ok());
    }

    #[test]
    fn password_without_username_is_an_error() {
        let spec = BrokerSpec {
            password: Some("x".into()),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
        assert_eq!(messages(errors), ["a password requires a username"]);
    }

    /// The folder of the test-only TLS files: `ca.pem`, `client.pem` and `client.key`.
    fn tls_testdata() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/tls")
    }

    fn tls_spec(ca: Option<&str>, cert: Option<&str>, key: Option<&str>) -> BrokerSpec {
        BrokerSpec {
            tls: Some(TlsSpec {
                ca_file: ca.map(PathBuf::from),
                client_cert: cert.map(PathBuf::from),
                client_key: key.map(PathBuf::from),
            }),
            ..BrokerSpec::default()
        }
    }

    #[test]
    fn tls_files_are_read_relative_to_base_dir() {
        let spec = tls_spec(Some("ca.pem"), Some("client.pem"), Some("client.key"));
        let tls = resolve_broker(&spec, &tls_testdata(), &no_env)
            .unwrap()
            .tls
            .unwrap();
        let ca = std::fs::read(tls_testdata().join("ca.pem")).unwrap();
        assert_eq!(tls.ca, Some(ca));
        assert!(tls.client_auth.is_some());
        assert!(tls.client.client_auth_cert_resolver.has_certs());
    }

    #[test]
    fn tls_without_ca_file_uses_system_roots() {
        let broker = resolve_broker(&tls_spec(None, None, None), Path::new("."), &no_env).unwrap();
        let tls = broker.tls.unwrap();
        assert_eq!((tls.ca, tls.client_auth), (None, None));
    }

    #[test]
    fn the_tls_setup_is_built_once_for_every_instance() {
        // Without a CA file it is built once per process; validation runs on every edit.
        let first = resolve_broker(&tls_spec(None, None, None), Path::new("."), &no_env).unwrap();
        let again = resolve_broker(&tls_spec(None, None, None), Path::new("."), &no_env).unwrap();
        assert!(Arc::ptr_eq(
            &first.tls.unwrap().client,
            &again.tls.unwrap().client
        ));
    }

    #[test]
    fn tls_files_that_are_not_pem_are_problems_at_validation() {
        let dir = std::env::temp_dir().join(format!("senzor-tls-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["ca.pem", "client.pem", "client.key"] {
            std::fs::copy(tls_testdata().join(name), dir.join(name)).unwrap();
        }
        std::fs::write(dir.join("bad.pem"), "not a certificate").unwrap();
        let problems = |spec: BrokerSpec| {
            let errors = resolve_broker(&spec, &dir, &no_env).unwrap_err();
            (fields(errors.clone()), messages(errors))
        };
        assert_eq!(
            problems(tls_spec(Some("bad.pem"), None, None)),
            (
                some(&["tls.ca_file"]),
                vec!["tls.ca_file `bad.pem` holds no PEM certificate".to_string()]
            )
        );
        assert_eq!(
            problems(tls_spec(
                Some("ca.pem"),
                Some("bad.pem"),
                Some("client.key")
            )),
            (
                some(&["tls.client_cert"]),
                vec!["tls.client_cert `bad.pem` holds no PEM certificate".to_string()]
            )
        );
        assert_eq!(
            problems(tls_spec(
                Some("ca.pem"),
                Some("client.pem"),
                Some("bad.pem")
            )),
            (
                some(&["tls.client_key"]),
                vec!["tls.client_key `bad.pem` holds no PEM private key".to_string()]
            )
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tls_errors_are_reported() {
        let spec = BrokerSpec {
            tls: Some(TlsSpec {
                ca_file: None,
                client_cert: Some(PathBuf::from("missing.pem")),
                client_key: None,
            }),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
        assert_eq!(
            messages(errors),
            [
                "tls.client_cert and tls.client_key must be set together",
                "tls.client_cert requires tls.ca_file",
            ]
        );
    }

    #[test]
    fn unreadable_ca_file_is_an_error() {
        let spec = BrokerSpec {
            tls: Some(TlsSpec {
                ca_file: Some(PathBuf::from("nope.pem")),
                ..TlsSpec::default()
            }),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("/nonexistent"), &no_env).unwrap_err();
        assert!(
            errors[0]
                .message
                .starts_with("cannot read tls.ca_file `nope.pem`"),
            "{}",
            errors[0].message
        );
    }
}
