//! Blocking HTTP against the GitHub API, with the error shapes the callers
//! actually need to distinguish (403 no-access, 404, 429 rate limit).

use std::time::Duration;

pub const API: &str = "https://api.github.com";

const RATE_LIMIT_RETRIES: usize = 3;
/// Never block waybar's tick longer than this on any single request.
const MAX_RETRY_WAIT: std::time::Duration = std::time::Duration::from_secs(4);

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("access denied ({status}): {message}")]
    AccessDenied { status: u16, message: String },
    #[error("not found: {message}")]
    NotFound { message: String },
    #[error("rate limited: {message}")]
    RateLimited { message: String, retry_after: Option<u64> },
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
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            ApiError::RateLimited { retry_after, .. } => {
                retry_after.map(std::time::Duration::from_secs)
            }
            _ => None,
        }
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

/// A 403 from GitHub is either "you may not read this" or "you asked too
/// fast". Only the message distinguishes them, and confusing the two makes
/// the widget report "no billing access" during a burst.
pub fn classify(status: u16, message: String) -> ApiError {
    let looks_throttled = {
        let m = message.to_ascii_lowercase();
        m.contains("rate limit")
            || m.contains("secondary rate")
            || m.contains("abuse detection")
            || m.contains("try again later")
    };
    match status {
        403 | 429 if looks_throttled => ApiError::RateLimited { message, retry_after: None },
        429 => ApiError::RateLimited { message, retry_after: None },
        403 => ApiError::AccessDenied { status, message },
        404 => ApiError::NotFound { message },
        _ => ApiError::Status { status, message },
    }
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

    /// GET with retries for rate limiting.
    ///
    /// GitHub enforces a *secondary* limit on burst concurrency that is
    /// separate from the 5,000/hr quota, and reports it as a 403 whose body
    /// says "rate limit exceeded" -- indistinguishable from a permissions 403
    /// unless you read the message. Retrying after the advertised delay is
    /// the documented remedy.
    pub fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        // Secondary limits can persist for minutes, so the widget does not
        // try to outwait them: it retries briefly, then gives up and lets the
        // caller serve cached data flagged stale. Blocking waybar's tick for
        // 30s would be worse than showing a slightly old number.
        let mut delay = std::time::Duration::from_millis(800);
        for attempt in 0..RATE_LIMIT_RETRIES {
            match self.get_json_once::<T>(path) {
                Err(e) if e.is_rate_limited() && attempt + 1 < RATE_LIMIT_RETRIES => {
                    let wait = e.retry_after().unwrap_or(delay).min(MAX_RETRY_WAIT);
                    std::thread::sleep(wait);
                    delay *= 3;
                }
                other => return other,
            }
        }
        self.get_json_once(path)
    }

    fn get_json_once<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let body = resp
            .text()
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        if (200..300).contains(&status) {
            return serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()));
        }
        let message = serde_json::from_str::<GhError>(&body)
            .ok()
            .and_then(|e| e.message)
            .unwrap_or_else(|| body.chars().take(160).collect());
        Err(match classify(status, message) {
            ApiError::RateLimited { message, .. } => ApiError::RateLimited { message, retry_after },
            other => other,
        })
    }
}
