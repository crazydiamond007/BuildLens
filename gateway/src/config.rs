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
    pub github_api_base_url: Url,
    pub github_webhook_url: Url,
    pub github_webhook_secret: String,
    pub token_encryption_key: [u8; 32],
    pub session_ttl: Duration,
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

        let mut github_api_base_url = optional("GITHUB_API_BASE_URL")
            .unwrap_or_else(|| "https://api.github.com/".to_string());
        if !github_api_base_url.ends_with('/') {
            github_api_base_url.push('/');
        }
        let github_api_base_url = parse_url("GITHUB_API_BASE_URL", &github_api_base_url)?;
        let github_webhook_url = parse_url(
            "GITHUB_WEBHOOK_URL",
            required("GITHUB_WEBHOOK_URL")?.as_str(),
        )?;
        let github_webhook_secret = required("GITHUB_WEBHOOK_SECRET")?;
        if github_webhook_secret.len() < 32 {
            return Err(ConfigError::Invalid {
                key: "GITHUB_WEBHOOK_SECRET",
                message: "must be at least 32 characters".to_string(),
            });
        }

        Ok(Self {
            environment: parse_optional("ENVIRONMENT")?.unwrap_or(Environment::Development),
            bind_addr,
            database_url: required("DATABASE_URL")?,
            database_max_connections,
            database_connect_timeout: Duration::from_secs(database_connect_timeout_seconds),
            redis_url: required("REDIS_URL")?,
            github_client_id: required("GITHUB_CLIENT_ID")?,
            github_client_secret: required("GITHUB_CLIENT_SECRET")?,
            github_redirect_uri,
            github_api_base_url,
            github_webhook_url,
            github_webhook_secret,
            token_encryption_key: encryption_key()?,
            session_ttl: Duration::from_secs(session_ttl_seconds),
        })
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
