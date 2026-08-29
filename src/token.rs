//! Where the GitHub token comes from. Default is the gh CLI's own store, so
//! cicdbar needs no credential of its own.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenSource {
    GhCli,
    Env(String),
    Literal(String),
}

impl TokenSource {
    pub fn parse(s: &str) -> Self {
        match s {
            "gh-cli" => TokenSource::GhCli,
            other => match other.split_once(':') {
                Some(("env", var)) => TokenSource::Env(var.to_string()),
                _ => TokenSource::Literal(other.to_string()),
            },
        }
    }

    pub fn resolve(&self) -> Result<String> {
        match self {
            TokenSource::Literal(t) => Ok(t.clone()),
            TokenSource::Env(var) => {
                std::env::var(var).with_context(|| format!("env var {var} is not set"))
            }
            TokenSource::GhCli => gh_cli_token(),
        }
    }
}

fn hosts_path() -> PathBuf {
    if let Ok(p) = std::env::var("GH_CONFIG_DIR") {
        return PathBuf::from(p).join("hosts.yml");
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
    base.join("gh").join("hosts.yml")
}

fn gh_cli_token() -> Result<String> {
    // GH_TOKEN wins, as it does for gh itself.
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return Ok(v);
            }
        }
    }
    let path = hosts_path();
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let hosts: serde_yaml::Value = serde_yaml::from_str(&raw)?;
    let host = hosts
        .get("github.com")
        .ok_or_else(|| anyhow!("no github.com entry in {}", path.display()))?;

    // Either a top-level oauth_token, or one under the active user.
    if let Some(t) = host.get("oauth_token").and_then(|v| v.as_str()) {
        return Ok(t.to_string());
    }
    let user = host.get("user").and_then(|v| v.as_str());
    if let Some(users) = host.get("users").and_then(|v| v.as_mapping()) {
        if let Some(u) = user {
            if let Some(t) = users
                .get(serde_yaml::Value::String(u.to_string()))
                .and_then(|v| v.get("oauth_token"))
                .and_then(|v| v.as_str())
            {
                return Ok(t.to_string());
            }
        }
        for (_, v) in users {
            if let Some(t) = v.get("oauth_token").and_then(|v| v.as_str()) {
                return Ok(t.to_string());
            }
        }
    }
    Err(anyhow!("no oauth_token found in {}", path.display()))
}
