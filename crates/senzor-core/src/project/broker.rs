use std::path::Path;
use std::time::Duration;

use super::error::ProjectError;
use super::file::{BrokerSpec, TlsSpec};
use super::model::{BrokerConfig, Credentials, TlsConfig};

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
        errors.push(ProjectError::broker("host must not be empty"));
    }
    if spec.port == 0 {
        errors.push(ProjectError::broker("port must not be 0"));
    }
    if spec.keep_alive < Duration::from_secs(1) {
        errors.push(ProjectError::broker("keep_alive must be at least 1s"));
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
    }
}

fn resolve_credentials(
    spec: &BrokerSpec,
    env: EnvLookup<'_>,
) -> Result<Option<Credentials>, ProjectError> {
    let password = match (&spec.password, &spec.password_env) {
        (Some(_), Some(_)) => {
            return Err(ProjectError::broker(
                "use either password or password_env, not both",
            ));
        }
        (Some(password), None) => Some(password.clone()),
        (None, Some(var)) => Some(env(var).ok_or_else(|| {
            ProjectError::broker(format!(
                "environment variable `{var}` (password_env) is not set"
            ))
        })?),
        (None, None) => None,
    };
    match (&spec.username, password) {
        (Some(username), password) => Ok(Some(Credentials {
            username: username.clone(),
            password: password.unwrap_or_default(),
        })),
        (None, Some(_)) => Err(ProjectError::broker("a password requires a username")),
        (None, None) => Ok(None),
    }
}

fn resolve_tls(spec: &TlsSpec, base_dir: &Path) -> Result<TlsConfig, Vec<ProjectError>> {
    let mut errors = Vec::new();
    let mut read = |name: &str, path: &Path| match std::fs::read(base_dir.join(path)) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            errors.push(ProjectError::broker(format!(
                "cannot read tls.{name} `{}`: {e}",
                path.display()
            )));
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
        _ => {
            errors.push(ProjectError::broker(
                "tls.client_cert and tls.client_key must be set together",
            ));
            None
        }
    };
    if spec.client_cert.is_some() && spec.ca_file.is_none() {
        errors.push(ProjectError::broker("tls.client_cert requires tls.ca_file"));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(TlsConfig { ca, client_auth })
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
    fn password_without_username_is_an_error() {
        let spec = BrokerSpec {
            password: Some("x".into()),
            ..BrokerSpec::default()
        };
        let errors = resolve_broker(&spec, Path::new("."), &no_env).unwrap_err();
        assert_eq!(messages(errors), ["a password requires a username"]);
    }

    #[test]
    fn tls_files_are_read_relative_to_base_dir() {
        let dir = std::env::temp_dir().join(format!("senzor-tls-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("certs")).unwrap();
        std::fs::write(dir.join("certs/ca.pem"), b"CA").unwrap();
        let spec = BrokerSpec {
            tls: Some(TlsSpec {
                ca_file: Some(PathBuf::from("certs/ca.pem")),
                ..TlsSpec::default()
            }),
            ..BrokerSpec::default()
        };
        let broker = resolve_broker(&spec, &dir, &no_env).unwrap();
        assert_eq!(
            broker.tls,
            Some(TlsConfig {
                ca: Some(b"CA".to_vec()),
                client_auth: None
            })
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tls_without_ca_file_uses_system_roots() {
        let spec = BrokerSpec {
            tls: Some(TlsSpec::default()),
            ..BrokerSpec::default()
        };
        let broker = resolve_broker(&spec, Path::new("."), &no_env).unwrap();
        assert_eq!(
            broker.tls,
            Some(TlsConfig {
                ca: None,
                client_auth: None
            })
        );
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
