//! Config lives at ~/.config/cicdbar/config.toml. Absent is fine; a typo is not.

use crate::money::Usd;
use crate::token::TokenSource;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    #[serde(deserialize_with = "usd_from_f64")]
    pub budget_usd: Usd,
    pub github: GitHubConfig,
    pub runs: RunsConfig,
    pub blacksmith: BlacksmithConfig,
    pub cache: CacheConfig,
    pub theme: Theme,
}

/// Bar and tooltip colours. Defaults are One Dark, to sit alongside the
/// palette most waybar setups already use; override any subset.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Theme {
    pub ok: String,
    pub low: String,
    pub warning: String,
    pub critical: String,
    pub text: String,
    pub dim: String,
    pub accent: String,
    pub track: String,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            ok: "#98c379".into(),
            low: "#e5c07b".into(),
            warning: "#d19a66".into(),
            critical: "#e06c75".into(),
            text: "#abb2bf".into(),
            dim: "#5c6370".into(),
            accent: "#61afef".into(),
            track: "#3e4451".into(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GitHubConfig {
    pub orgs: Vec<String>,
    #[serde(deserialize_with = "token_source")]
    pub token_source: TokenSource,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RunsConfig {
    pub active_days: i64,
    pub max_repos: usize,
    pub max_tooltip_runs: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BlacksmithConfig {
    pub enabled: bool,
    pub org: Option<String>,
    /// File holding the app.blacksmith.sh cookie pair. Blacksmith publishes
    /// no billing API, so this is a captured dashboard session; the file is
    /// rewritten on each call because the server rotates the session cookie.
    pub session_file: Option<std::path::PathBuf>,
    pub api_base: Option<String>,
    /// Per-minute list price by runner family, at the `base_vcpu` tier, used
    /// for the computed estimate when the dashboard API is unavailable.
    /// Override these if you are on negotiated pricing.
    pub rates: std::collections::BTreeMap<String, f64>,
    /// vCPU count the rates above are quoted for; cost scales linearly.
    pub base_vcpu: f64,
    /// Minutes included free each month.
    pub free_minutes: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CacheConfig {
    pub billing_ttl_secs: u64,
    pub runs_ttl_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            budget_usd: Usd::zero(),
            github: GitHubConfig::default(),
            runs: RunsConfig::default(),
            blacksmith: BlacksmithConfig::default(),
            cache: CacheConfig::default(),
            theme: Theme::default(),
        }
    }
}

impl Default for GitHubConfig {
    fn default() -> Self {
        GitHubConfig {
            orgs: Vec::new(),
            token_source: TokenSource::GhCli,
        }
    }
}

impl Default for RunsConfig {
    fn default() -> Self {
        RunsConfig {
            active_days: 7,
            max_repos: 40,
            max_tooltip_runs: 8,
        }
    }
}

impl Default for BlacksmithConfig {
    fn default() -> Self {
        BlacksmithConfig {
            enabled: false,
            org: None,
            session_file: None,
            api_base: None,
            rates: default_rates(),
            base_vcpu: 2.0,
            free_minutes: 3_000,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            billing_ttl_secs: 900,
            runs_ttl_secs: 30,
        }
    }
}

/// Blacksmith's published per-minute list prices at the 2-vCPU tier, keyed by
/// runner family. Observed on their pricing page, 2026-08-29.
pub fn default_rates() -> std::collections::BTreeMap<String, f64> {
    [
        ("ubuntu", 0.004),
        ("arm", 0.0025),
        ("windows", 0.008),
        ("macos", 0.08),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn usd_from_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Usd, D::Error> {
    let v = f64::deserialize(d)?;
    Ok(Usd::from_f64(v))
}

fn token_source<'de, D: serde::Deserializer<'de>>(d: D) -> Result<TokenSource, D::Error> {
    let s = String::deserialize(d)?;
    Ok(TokenSource::parse(&s))
}

use serde::Deserialize;

impl Config {
    pub fn default_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
            });
        base.join("cicdbar").join("config.toml")
    }

    /// Default location of the blacksmith session file.
    pub fn default_blacksmith_session() -> PathBuf {
        Self::default_path().with_file_name("blacksmith-session")
    }

    pub fn load(path: &Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }
}
