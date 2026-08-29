//! Blocking HTTP against the GitHub API, with the error shapes the callers
//! actually need to distinguish (403 no-access, 404, 429 rate limit).

use std::time::Duration;

pub const API: &str = "https://api.github.com";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("access denied ({status}): {message}")]
    AccessDenied { status: u16, message: String },
    #[error("not found: {message}")]
    NotFound { message: String },
    #[error("rate limited: {message}")]
    RateLimited { message: String },
    #[error("http {status}: {message}")]
    Status { status: u16, message: String },
    #[error("transport: {0}")]
    Transport(String),
    #[error("decode: {0}")]
    Decode(String),
}

impl ApiError {
    pub fn is_access_denied(&self) -> bool {
        matches!(self, ApiError::AccessDenied { .. })
    }
    pub fn is_not_found(&self) -> bool {
        matches!(self, ApiError::NotFound { .. })
    }
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, ApiError::RateLimited { .. })
    }
    /// Short text for the tooltip's per-provider health line.
    pub fn short(&self) -> String {
        match self {
            ApiError::AccessDenied { .. } => "no billing access".into(),
            ApiError::NotFound { .. } => "not found".into(),
            ApiError::RateLimited { .. } => "rate limited".into(),
            ApiError::Status { status, .. } => format!("HTTP {status}"),
            ApiError::Transport(_) => "unreachable".into(),
            ApiError::Decode(_) => "bad response".into(),
        }
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::blocking::Client,
    base: String,
}

#[derive(serde::Deserialize)]
struct GhError {
    message: Option<String>,
}

impl Http {
    pub fn new(token: String) -> anyhow::Result<Self> {
        Self::with_base(token, API.to_string())
    }

    pub fn with_base(token: String, base: String) -> anyhow::Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse()?,
        );
        headers.insert("X-GitHub-Api-Version", "2022-11-28".parse()?);
        headers.insert(reqwest::header::ACCEPT, "application/vnd.github+json".parse()?);
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("cicdbar/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Http { client, base })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if (200..300).contains(&status) {
            return serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()));
        }
        let message = serde_json::from_str::<GhError>(&body)
            .ok()
            .and_then(|e| e.message)
            .unwrap_or_else(|| body.chars().take(120).collect());
        Err(match status {
            403 => ApiError::AccessDenied { status, message },
            404 => ApiError::NotFound { message },
            429 => ApiError::RateLimited { message },
            _ => ApiError::Status { status, message },
        })
    }
}
