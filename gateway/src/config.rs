use std::{env, fmt, net::SocketAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::Url;

/// Everything the gateway needs from its environment.
///
/// Read once at startup and never again. A config value that can change under a
/// running request handler is a bug waiting to happen. Missing or malformed
/// values fail here, loudly, before the process binds a port. A service that
/// starts successfully and then 500s on its first request because
/// `DATABASE_URL` was a typo is much harder to diagnose than one that refuses to
/// start.
///
/// This contains only configuration used by implemented features. RabbitMQ and
/// S3 settings will arrive with the code that needs them.
#[derive(Clone)]
pub struct Config {
    pub environment: Environment,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub database_max_connections: u32,
    pub database_connect_timeout: Duration,
    pub redis_url: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    pub github_redirect_uri: String,
    // The GitHub App itself, as opposed to its OAuth client credentials above.
    // The numeric App ID and the private key sign the App JWT that mints
    // installation access tokens; the slug builds the browser install URL.
    pub github_app_id: String,
    pub github_app_slug: String,
    pub github_app_private_key: String,
    pub frontend_url: Url,
    pub github_api_base_url: Url,
    pub github_webhook_secret: String,
    pub token_encryption_key: [u8; 32],
    pub session_ttl: Duration,
    pub rabbitmq_url: String,
    pub s3: S3Config,
}

/// Where build logs are stored. MinIO in development, real S3 in production;
/// the gateway does not care which, because both speak the S3 API. Path-style
/// addressing (`endpoint/bucket/key`) is what makes a single hostname work for
/// MinIO, where virtual-host-style (`bucket.endpoint`) does not resolve.
#[derive(Clone)]
pub struct S3Config {
    pub endpoint: Url,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub logs_bucket: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Environment {
    Development,
    Production,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{key} is invalid: {message}")]
    Invalid { key: &'static str, message: String },
    #[error("refusing to start in production:\n{}", render(.0))]
    Insecure(Vec<Weakness>),
}

/// A configuration value that parses but must not be trusted.
///
/// Kept apart from `ConfigError` because severity is a function of the
/// environment, not of the value: the all-zero encryption key is exactly what a
/// throwaway local stack should use, and exactly what must never reach
/// production. These are reported together rather than one at a time, so a
/// misconfigured deployment learns everything it has to fix in a single boot
/// instead of discovering it one restart at a time.
#[derive(Debug)]
pub struct Weakness {
    pub key: &'static str,
    pub problem: String,
    pub remedy: &'static str,
}

fn render(weaknesses: &[Weakness]) -> String {
    weaknesses
        .iter()
        .map(|weakness| {
            format!(
                "  {} {}\n    {}",
                weakness.key, weakness.problem, weakness.remedy
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let host = optional("GATEWAY_HOST").unwrap_or_else(|| "0.0.0.0".to_string());
        let port: u16 = parse_optional("GATEWAY_PORT")?.unwrap_or(8080);

        let bind_addr = format!("{host}:{port}")
            .parse()
            .map_err(|e| ConfigError::Invalid {
                key: "GATEWAY_HOST",
                message: format!("{host}:{port} is not a socket address: {e}"),
            })?;

        let database_max_connections = parse_optional("DATABASE_MAX_CONNECTIONS")?.unwrap_or(10);
        if database_max_connections == 0 {
            return Err(ConfigError::Invalid {
                key: "DATABASE_MAX_CONNECTIONS",
                message: "must be greater than zero".to_string(),
            });
        }

        let database_connect_timeout_seconds =
            parse_optional("DATABASE_CONNECT_TIMEOUT_SECONDS")?.unwrap_or(5);
        if database_connect_timeout_seconds == 0 {
            return Err(ConfigError::Invalid {
                key: "DATABASE_CONNECT_TIMEOUT_SECONDS",
                message: "must be greater than zero".to_string(),
            });
        }

        let session_ttl_seconds = parse_optional("SESSION_TTL_SECONDS")?.unwrap_or(604_800);
        if session_ttl_seconds == 0 {
            return Err(ConfigError::Invalid {
                key: "SESSION_TTL_SECONDS",
                message: "must be greater than zero".to_string(),
            });
        }

        let github_redirect_uri = required("GITHUB_REDIRECT_URI")?;
        reqwest::Url::parse(&github_redirect_uri).map_err(|e| ConfigError::Invalid {
            key: "GITHUB_REDIRECT_URI",
            message: e.to_string(),
        })?;
        let frontend_url = parse_url(
            "FRONTEND_URL",
            optional("FRONTEND_URL")
                .unwrap_or_else(|| "http://localhost:3000".to_string())
                .as_str(),
        )?;

        let mut github_api_base_url = optional("GITHUB_API_BASE_URL")
            .unwrap_or_else(|| "https://api.github.com/".to_string());
        if !github_api_base_url.ends_with('/') {
            github_api_base_url.push('/');
        }
        let github_api_base_url = parse_url("GITHUB_API_BASE_URL", &github_api_base_url)?;
        let github_webhook_secret = required("GITHUB_WEBHOOK_SECRET")?;
        if github_webhook_secret.len() < 32 {
            return Err(ConfigError::Invalid {
                key: "GITHUB_WEBHOOK_SECRET",
                message: "must be at least 32 characters".to_string(),
            });
        }

        let s3 = S3Config {
            endpoint: parse_url("S3_ENDPOINT", required("S3_ENDPOINT")?.as_str())?,
            region: optional("S3_REGION").unwrap_or_else(|| "us-east-1".to_string()),
            access_key: required("S3_ACCESS_KEY")?,
            secret_key: required("S3_SECRET_KEY")?,
            logs_bucket: required("S3_LOGS_BUCKET")?,
        };

        let config = Self {
            environment: parse_optional("ENVIRONMENT")?.unwrap_or(Environment::Development),
            bind_addr,
            database_url: required("DATABASE_URL")?,
            database_max_connections,
            database_connect_timeout: Duration::from_secs(database_connect_timeout_seconds),
            redis_url: required("REDIS_URL")?,
            github_client_id: required("GITHUB_CLIENT_ID")?,
            github_client_secret: required("GITHUB_CLIENT_SECRET")?,
            github_redirect_uri,
            github_app_id: required("GITHUB_APP_ID")?,
            github_app_slug: required("GITHUB_APP_SLUG")?,
            github_app_private_key: app_private_key()?,
            frontend_url,
            github_api_base_url,
            github_webhook_secret,
            token_encryption_key: encryption_key()?,
            session_ttl: Duration::from_secs(session_ttl_seconds),
            rabbitmq_url: required("RABBITMQ_URL")?,
            s3,
        };

        // Development is allowed to run with placeholders - that is what they
        // are for - but `main` says so on every boot rather than staying quiet.
        // Production gets no such latitude.
        if config.environment == Environment::Production {
            let weaknesses = config.weaknesses();
            if !weaknesses.is_empty() {
                return Err(ConfigError::Insecure(weaknesses));
            }
        }

        Ok(config)
    }

    /// Values that parse cleanly but leave the deployment insecure.
    ///
    /// Nothing above rejects them, which is what makes them dangerous: the
    /// all-zero encryption key works perfectly, so a self-hosted BuildLens can
    /// protect every stored GitHub token with a key published in
    /// `.env.example` and never see a single symptom.
    ///
    /// `main` calls this again after tracing is initialised, so the checks stay
    /// free of logging and the warnings reach the same subscriber as everything
    /// else.
    pub fn weaknesses(&self) -> Vec<Weakness> {
        let mut found = Vec::new();

        for (key, value) in [
            ("GITHUB_CLIENT_ID", &self.github_client_id),
            ("GITHUB_CLIENT_SECRET", &self.github_client_secret),
            ("GITHUB_WEBHOOK_SECRET", &self.github_webhook_secret),
            ("GITHUB_APP_ID", &self.github_app_id),
            ("GITHUB_APP_SLUG", &self.github_app_slug),
            ("GITHUB_APP_PRIVATE_KEY", &self.github_app_private_key),
        ] {
            // The webhook placeholder is 40 characters, so it satisfies the
            // length rule above and is otherwise indistinguishable from a real
            // secret.
            if value.starts_with(PLACEHOLDER_PREFIX) {
                found.push(Weakness {
                    key,
                    problem: "is still the .env.example placeholder".to_string(),
                    remedy: "Set the real value; GitHub sign-in cannot work until you do.",
                });
            }
        }

        if self.token_encryption_key == [0u8; 32] {
            found.push(Weakness {
                key: "TOKEN_ENCRYPTION_KEY",
                problem: "is the all-zero default from .env.example".to_string(),
                remedy: "Generate one: openssl rand -base64 32 - and keep it, because \
                         changing it makes already stored GitHub tokens unreadable.",
            });
        }

        // Only meaningful once the browser is somewhere other than localhost, and
        // in production it is also self-enforcing: the session cookie carries
        // `Secure` there, so an http origin cannot hold a session at all.
        if self.environment == Environment::Production {
            for (key, url) in [
                ("FRONTEND_URL", self.frontend_url.scheme()),
                ("GITHUB_REDIRECT_URI", scheme_of(&self.github_redirect_uri)),
            ] {
                if url != "https" {
                    found.push(Weakness {
                        key,
                        problem: "is not https".to_string(),
                        remedy: "Production sets Secure on the session cookie, so sign-in \
                                 silently fails over plain http.",
                    });
                }
            }
        }

        found
    }
}

const PLACEHOLDER_PREFIX: &str = "replace_with";

fn scheme_of(raw: &str) -> &str {
    match raw.split_once("://") {
        Some((scheme, _)) => scheme,
        None => "",
    }
}

fn parse_url(key: &'static str, value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|e| ConfigError::Invalid {
        key,
        message: e.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ConfigError::Invalid {
            key,
            message: "must be an absolute http or https URL".to_string(),
        });
    }
    Ok(url)
}

fn encryption_key() -> Result<[u8; 32], ConfigError> {
    let encoded = required("TOKEN_ENCRYPTION_KEY")?;
    let decoded = STANDARD.decode(encoded).map_err(|e| ConfigError::Invalid {
        key: "TOKEN_ENCRYPTION_KEY",
        message: format!("must be valid base64: {e}"),
    })?;

    decoded
        .try_into()
        .map_err(|bytes: Vec<u8>| ConfigError::Invalid {
            key: "TOKEN_ENCRYPTION_KEY",
            message: format!("must decode to exactly 32 bytes, got {}", bytes.len()),
        })
}

/// The GitHub App private key, accepted as either a raw PEM (newlines and all)
/// or a base64 blob of that PEM squeezed onto one line. The single-line form
/// exists because a multi-line secret is awkward in a `.env` file and outright
/// hostile in a Kubernetes Secret; either way the gateway hands a real PEM to
/// the JWT signer. The placeholder is left untouched so `weaknesses()` can still
/// recognise and report it rather than failing here on a base64 decode.
fn app_private_key() -> Result<String, ConfigError> {
    let raw = required("GITHUB_APP_PRIVATE_KEY")?;
    // Left as-is for weaknesses() to recognise and report; a placeholder is not
    // a key to validate.
    if raw.starts_with(PLACEHOLDER_PREFIX) {
        return Ok(raw);
    }
    let pem = if raw.contains("BEGIN") {
        raw
    } else {
        let decoded = STANDARD
            .decode(raw.trim())
            .map_err(|e| ConfigError::Invalid {
                key: "GITHUB_APP_PRIVATE_KEY",
                message: format!("is neither a PEM nor valid base64: {e}"),
            })?;
        String::from_utf8(decoded).map_err(|e| ConfigError::Invalid {
            key: "GITHUB_APP_PRIVATE_KEY",
            message: format!("base64 did not decode to text: {e}"),
        })?
    };
    // A real key is validated here so a mangled PEM fails at boot rather than on
    // the first installation-token mint, which happens deep inside a webhook.
    validate_rsa_pem(&pem)?;
    Ok(pem)
}

fn validate_rsa_pem(pem: &str) -> Result<(), ConfigError> {
    jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
        .map(|_| ())
        .map_err(|e| ConfigError::Invalid {
            key: "GITHUB_APP_PRIVATE_KEY",
            message: format!("is not a valid RSA private key PEM: {e}"),
        })
}

fn required(key: &'static str) -> Result<String, ConfigError> {
    optional(key).ok_or(ConfigError::Missing(key))
}

/// Treats an empty string as absent. Docker Compose interpolates an unset
/// variable to `""` rather than leaving it out, so without this an unset
/// `GATEWAY_PORT` would parse as an empty string instead of falling back.
fn optional(key: &'static str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn parse_optional<T>(key: &'static str) -> Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    optional(key)
        .map(|raw| {
            raw.parse::<T>().map_err(|e| ConfigError::Invalid {
                key,
                message: e.to_string(),
            })
        })
        .transpose()
}

impl std::str::FromStr for Environment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "development" | "dev" | "local" => Ok(Self::Development),
            "production" | "prod" => Ok(Self::Production),
            other => Err(format!(
                "expected 'development' or 'production', got '{other}'"
            )),
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Development => write!(f, "development"),
            Self::Production => write!(f, "production"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A production config with nothing wrong with it. Each test breaks exactly
    /// one thing, so a failure names the check that stopped working.
    fn secure_production() -> Config {
        Config {
            environment: Environment::Production,
            bind_addr: "0.0.0.0:8080".parse().unwrap(),
            database_url: "postgres://user:pass@db/buildlens".to_string(),
            database_max_connections: 10,
            database_connect_timeout: Duration::from_secs(5),
            redis_url: "redis://redis:6379".to_string(),
            github_client_id: "Iv1.a1b2c3d4e5f6".to_string(),
            github_client_secret: "0123456789abcdef0123456789abcdef01234567".to_string(),
            github_redirect_uri: "https://buildlens.example/auth/github/callback".to_string(),
            github_app_id: "123456".to_string(),
            github_app_slug: "buildlens".to_string(),
            github_app_private_key:
                "-----BEGIN RSA PRIVATE KEY-----\nfake\n-----END RSA PRIVATE KEY-----".to_string(),
            frontend_url: Url::parse("https://buildlens.example").unwrap(),
            github_api_base_url: Url::parse("https://api.github.com/").unwrap(),
            github_webhook_secret: "d1a0f3c8b7e6549201fedcba9876543210abcdef".to_string(),
            token_encryption_key: [7u8; 32],
            session_ttl: Duration::from_secs(604_800),
            rabbitmq_url: "amqp://user:pass@rabbit/buildlens".to_string(),
            s3: S3Config {
                endpoint: Url::parse("https://s3.example").unwrap(),
                region: "us-east-1".to_string(),
                access_key: "access".to_string(),
                secret_key: "secret".to_string(),
                logs_bucket: "buildlens-logs".to_string(),
            },
        }
    }

    fn keys(config: &Config) -> Vec<&'static str> {
        config.weaknesses().into_iter().map(|w| w.key).collect()
    }

    #[test]
    fn a_properly_configured_production_deployment_has_no_weaknesses() {
        assert!(secure_production().weaknesses().is_empty());
    }

    #[test]
    fn the_published_all_zero_encryption_key_is_rejected() {
        let mut config = secure_production();
        config.token_encryption_key = [0u8; 32];
        assert_eq!(keys(&config), ["TOKEN_ENCRYPTION_KEY"]);
    }

    /// The placeholder is 40 characters, so the length rule in `from_env` passes
    /// it. Nothing but this check stands between it and a production webhook.
    #[test]
    fn the_webhook_placeholder_is_caught_despite_being_long_enough() {
        let mut config = secure_production();
        config.github_webhook_secret = "replace_with_at_least_32_random_characters".to_string();
        assert!(config.github_webhook_secret.len() >= 32);
        assert_eq!(keys(&config), ["GITHUB_WEBHOOK_SECRET"]);
    }

    /// The App's private key is a root credential: it mints installation tokens
    /// for every repository the App can touch. Shipping the `.env.example`
    /// placeholder to production has to be caught for the same reason the
    /// webhook secret is.
    #[test]
    fn the_github_app_private_key_placeholder_is_rejected() {
        let mut config = secure_production();
        config.github_app_private_key = "replace_with_github_app_private_key_pem".to_string();
        assert_eq!(keys(&config), ["GITHUB_APP_PRIVATE_KEY"]);
    }

    #[test]
    fn plain_http_origins_are_only_a_problem_in_production() {
        let mut config = secure_production();
        config.frontend_url = Url::parse("http://localhost:3000").unwrap();
        config.github_redirect_uri = "http://localhost:8080/auth/github/callback".to_string();
        assert_eq!(keys(&config), ["FRONTEND_URL", "GITHUB_REDIRECT_URI"]);

        config.environment = Environment::Development;
        assert!(config.weaknesses().is_empty());
    }

    #[test]
    fn every_problem_is_reported_in_one_pass() {
        let mut config = secure_production();
        config.github_client_id = "replace_with_github_oauth_client_id".to_string();
        config.token_encryption_key = [0u8; 32];
        config.frontend_url = Url::parse("http://buildlens.example").unwrap();
        assert_eq!(
            keys(&config),
            ["GITHUB_CLIENT_ID", "TOKEN_ENCRYPTION_KEY", "FRONTEND_URL"]
        );
    }
}
