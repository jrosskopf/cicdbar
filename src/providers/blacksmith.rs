//! Blacksmith spend.
//!
//! Blacksmith publishes no billing API -- the only public endpoint is their
//! Statuspage. Two routes therefore exist:
//!
//! * `dashboard_spend`, the exact figure from `app.blacksmith.sh`, which needs
//!   a session token captured from the dashboard. Until one is configured it
//!   returns an error rather than a zero, so the tooltip can say "unknown"
//!   instead of quietly lying.
//! * `org_month_estimate`, derived from GitHub's own job records: every job
//!   GitHub ran on a `blacksmith-*` runner, priced at Blacksmith's published
//!   per-minute rates. Always labelled an estimate.
//!
//! The estimate is scoped to repos that actually use Blacksmith runners, so
//! it costs a handful of requests rather than a sweep of every repo.

use crate::http::{ApiError, Http};
use crate::money::Usd;
use crate::providers::github_runs::{self, RunnerKind};
use std::collections::BTreeMap;

/// Blacksmith's included allowance.
pub const FREE_MINUTES_PER_MONTH: i64 = 3_000;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    /// Billable seconds observed on Blacksmith runners.
    pub seconds: i64,
    /// Cost before the free allowance.
    pub cost: Usd,
    pub by_runner: BTreeMap<String, i64>,
}

impl Usage {
    pub fn merge(&mut self, other: &Usage) {
        self.seconds += other.seconds;
        self.cost += other.cost;
        for (k, v) in &other.by_runner {
            *self.by_runner.entry(k.clone()).or_default() += v;
        }
    }
    pub fn minutes(&self) -> i64 {
        self.seconds / 60
    }
}

pub fn cost_for(kind: &RunnerKind, seconds: i64) -> Usd {
    github_runs::estimated_cost(kind, seconds).unwrap_or_else(Usd::zero)
}

/// Charge only the minutes beyond the included allowance.
pub fn apply_free_minutes(gross: Usd, used_seconds: i64, free_minutes: i64) -> Usd {
    let used_minutes = used_seconds / 60;
    if used_minutes <= free_minutes {
        return Usd::zero();
    }
    let billable = (used_minutes - free_minutes) as f64 / used_minutes as f64;
    gross * billable
}

/// Month-to-date Blacksmith usage for one repo, from GitHub's job records.
pub fn repo_month_usage(
    http: &Http,
    owner: &str,
    repo: &str,
    year: i16,
    month: u8,
) -> Result<Usage, ApiError> {
    let runs = github_runs::runs_in_month(http, owner, repo, year, month, 100)?;
    let mut usage = Usage::default();

    let per_run: Vec<Usage> = github_runs::fan_out(&runs, |run| {
        let mut u = Usage::default();
        let Ok(jobs) = github_runs::jobs_for_run(http, owner, repo, run.id) else {
            return u;
        };
        let now = jiff::Timestamp::now();
        for job in &jobs {
            let kind = job.runner();
            if !matches!(kind, RunnerKind::Blacksmith { .. }) {
                continue;
            }
            let secs = job.elapsed_secs(now);
            if secs <= 0 {
                continue;
            }
            u.seconds += secs;
            u.cost += cost_for(&kind, secs);
            *u.by_runner.entry(kind.short()).or_default() += secs;
        }
        u
    });
    for u in &per_run {
        usage.merge(u);
    }
    Ok(usage)
}

/// Which of an org's active repos actually run jobs on Blacksmith.
pub fn discover_repos(
    http: &Http,
    org: &str,
    active_days: i64,
    max_repos: usize,
) -> Result<Vec<String>, ApiError> {
    let repos = github_runs::active_repos(http, org, active_days, max_repos)?;
    let hits: Vec<Option<String>> = github_runs::fan_out(&repos, |repo| {
        let runs = github_runs::recent_runs(http, &repo.owner, &repo.name, 10).ok()?;
        let found = github_runs::fan_out(&runs, |run| {
            github_runs::jobs_for_run(http, &repo.owner, &repo.name, run.id)
                .map(|jobs| {
                    jobs.iter()
                        .any(|j| matches!(j.runner(), RunnerKind::Blacksmith { .. }))
                })
                .unwrap_or(false)
        });
        found.into_iter().any(|x| x).then(|| repo.name.clone())
    });
    Ok(hits.into_iter().flatten().collect())
}

/// Month-to-date estimate for an org, over the repos that use Blacksmith.
pub fn org_month_estimate(
    http: &Http,
    org: &str,
    repos: &[String],
    year: i16,
    month: u8,
) -> Usage {
    let per_repo: Vec<Usage> = github_runs::fan_out(repos, |repo| {
        repo_month_usage(http, org, repo, year, month).unwrap_or_default()
    });
    let mut total = Usage::default();
    for u in &per_repo {
        total.merge(u);
    }
    total
}

/// The exact figure from the Blacksmith dashboard.
///
/// Blacksmith exposes no documented billing API; this needs a session token
/// captured from `app.blacksmith.sh`. Absent one, it is an explicit error --
/// never a zero that would read as "you spent nothing".
pub fn dashboard_spend(cookie_file: Option<&std::path::Path>, org: &str) -> anyhow::Result<Usd> {
    let Some(file) = cookie_file else {
        anyhow::bail!(
            "blacksmith dashboard session not configured; \
             falling back to the estimate from GitHub job labels"
        );
    };
    Ok(Dashboard::from_cookie_file(file)?.projected(org)?.amount)
}

// ---------------------------------------------------------------------------
// Dashboard API
//
// `dashboardbackend.blacksmith.sh` is the JSON backend behind
// app.blacksmith.sh. It is undocumented and unversioned, so everything here
// is defensive: a shape change must degrade to the estimate, never panic.
//
// Auth is a Laravel cookie pair -- a long-lived `remember_web_*` and a
// `blacksmith_session` that the server ROTATES on every single response. A
// client that does not write the new value back authenticates exactly once,
// so the cookie jar is persisted after each call.
// ---------------------------------------------------------------------------

pub const DASHBOARD_API: &str = "https://dashboardbackend.blacksmith.sh";

pub struct Dashboard {
    client: reqwest::blocking::Client,
    cookies: std::sync::Mutex<BTreeMap<String, String>>,
    cookie_file: Option<std::path::PathBuf>,
    base: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Projected {
    #[serde(rename = "amount_cents", deserialize_with = "usd_from_cents")]
    pub amount: Usd,
    #[serde(default, deserialize_with = "null_as_default")]
    pub charges: BTreeMap<String, Charge>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Charge {
    #[serde(rename = "amount_cents", deserialize_with = "usd_from_cents")]
    pub amount: Usd,
    #[serde(default)]
    pub gb_hours: Option<f64>,
}

fn usd_from_cents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Usd, D::Error> {
    use serde::Deserialize;
    let cents = f64::deserialize(d)?;
    Ok(Usd::from_micros((cents * 10_000.0).round() as i64))
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CoreUsage {
    /// The API sends an explicit `null` here when nothing is running, which
    /// `#[serde(default)]` alone does not cover.
    #[serde(default, deserialize_with = "null_as_default")]
    pub current_usage: BTreeMap<String, Arch>,
}

fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de> + Default,
{
    use serde::Deserialize;
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Arch {
    #[serde(default)]
    pub vcpus: i64,
    #[serde(default)]
    pub jobs: i64,
    #[serde(default)]
    pub held: i64,
}

impl CoreUsage {
    pub fn total_vcpus(&self) -> i64 {
        self.current_usage.values().map(|a| a.vcpus).sum()
    }
    pub fn total_jobs(&self) -> i64 {
        self.current_usage.values().map(|a| a.jobs).sum()
    }
    /// Architectures with work on them right now.
    pub fn active(&self) -> Vec<(String, i64, i64)> {
        let mut v: Vec<_> = self
            .current_usage
            .iter()
            .filter(|(_, a)| a.jobs > 0 || a.vcpus > 0)
            .map(|(k, a)| (k.clone(), a.jobs, a.vcpus))
            .collect();
        v.sort();
        v
    }
}

fn parse_cookie_string(s: &str) -> BTreeMap<String, String> {
    s.split(';')
        .filter_map(|p| p.trim().split_once('='))
        .filter(|(k, _)| !k.is_empty())
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

impl Dashboard {
    pub fn with_cookies(cookies: String) -> Dashboard {
        Dashboard {
            client: reqwest::blocking::Client::builder()
                .user_agent("Mozilla/5.0 (X11; Linux x86_64) cicdbar")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("client"),
            cookies: std::sync::Mutex::new(parse_cookie_string(&cookies)),
            cookie_file: None,
            base: DASHBOARD_API.to_string(),
        }
    }

    pub fn from_cookie_file(path: &std::path::Path) -> anyhow::Result<Dashboard> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!("reading blacksmith session {}: {e}", path.display())
        })?;
        let mut d = Dashboard::with_cookies(raw.replace('\n', "; "));
        d.cookie_file = Some(path.to_path_buf());
        Ok(d)
    }

    pub fn with_base(mut self, base: String) -> Dashboard {
        self.base = base;
        self
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Merge Set-Cookie from a response and persist, so the rotated session
    /// survives to the next exec of the binary.
    fn absorb_cookies(&self, resp: &reqwest::blocking::Response) {
        let mut changed = false;
        {
            let mut jar = self.cookies.lock().unwrap();
            for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
                let Ok(s) = hv.to_str() else { continue };
                let Some(pair) = s.split(';').next() else { continue };
                let Some((k, v)) = pair.split_once('=') else { continue };
                let (k, v) = (k.trim().to_string(), v.trim().to_string());
                if jar.get(&k).map(|old| old != &v).unwrap_or(true) {
                    jar.insert(k, v);
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        let Some(path) = &self.cookie_file else { return };
        let body = self.cookie_header();
        // 0600: this is a live credential.
        let _ = std::fs::write(path, &body);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }

    /// The durable half of the credential. `blacksmith_session` rotates and
    /// can go stale (another process refreshed it, or it simply aged out);
    /// `remember_web_*` is what re-establishes a session.
    fn durable_cookies(&self) -> Option<String> {
        let jar = self.cookies.lock().unwrap();
        let kept: Vec<String> = jar
            .iter()
            .filter(|(k, _)| k.starts_with("remember_web"))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        (!kept.is_empty()).then(|| kept.join("; "))
    }

    fn send(&self, path: &str, cookies: &str) -> anyhow::Result<(reqwest::StatusCode, String)> {
        let resp = self
            .client
            .get(format!("{}{}", self.base, path))
            .header(reqwest::header::COOKIE, cookies)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ORIGIN, "https://app.blacksmith.sh")
            .header(reqwest::header::REFERER, "https://app.blacksmith.sh/")
            .send()?;
        self.absorb_cookies(&resp);
        let status = resp.status();
        Ok((status, resp.text()?))
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let (mut status, mut body) = self.send(path, &self.cookie_header())?;

        // A stale session is expected, not exceptional: the server rotates the
        // cookie on every response, so a concurrent caller (or a previous run
        // whose write we missed) can leave ours behind. Re-authenticate with
        // the durable cookie before giving up.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(durable) = self.durable_cookies() {
                let (s2, b2) = self.send(path, &durable)?;
                status = s2;
                body = b2;
            }
        }

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!(
                "blacksmith session expired or invalid (HTTP {}); re-capture it from app.blacksmith.sh",
                status.as_u16()
            );
        }
        if !status.is_success() {
            anyhow::bail!("blacksmith dashboard HTTP {}", status.as_u16());
        }
        serde_json::from_str(&body).map_err(|e| {
            anyhow::anyhow!("blacksmith dashboard returned an unexpected shape: {e}")
        })
    }

    /// Month-to-date charge for the current billing period.
    pub fn projected(&self, org: &str) -> anyhow::Result<Projected> {
        self.get(&format!("/api/user/github/orgs/{org}/billing/projected"))
    }

    /// Runner capacity in use right now.
    pub fn core_usage(&self, org: &str) -> anyhow::Result<CoreUsage> {
        self.get(&format!("/api/user/github/orgs/{org}/metrics/core-usage/current"))
    }
}
