//! Workflow runs: what CI is doing right now, and what it is costing while it
//! does it.
//!
//! GitHub has no org-wide "runs in flight" endpoint, so this discovers
//! recently-pushed repos and asks each one. Cost is capped by `active_days`
//! and `max_repos`, and by caching at a shorter TTL than billing.

use crate::http::{ApiError, Http};
use crate::money::Usd;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
    pub pushed_at: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: u64,
    pub repo: String,
    pub owner: String,
    pub workflow: String,
    pub branch: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    pub is_default_branch: bool,
    pub url: String,
}

impl RunSummary {
    pub fn is_running(&self) -> bool {
        matches!(self.status.as_str(), "in_progress")
    }
    pub fn is_queued(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "waiting" | "pending" | "requested")
    }
    pub fn started(&self) -> Option<Timestamp> {
        self.started_at.as_ref().and_then(|s| s.parse().ok())
    }
    pub fn elapsed_secs(&self, now: Timestamp) -> i64 {
        self.started().map(|s| (now.as_second() - s.as_second()).max(0)).unwrap_or(0)
    }
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub name: String,
    pub labels: Vec<String>,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl JobInfo {
    pub fn runner(&self) -> RunnerKind {
        RunnerKind::from_labels(&self.labels)
    }
    pub fn elapsed_secs(&self, now: Timestamp) -> i64 {
        let Some(start) = self.started_at.as_ref().and_then(|s| s.parse::<Timestamp>().ok())
        else {
            return 0;
        };
        let end = self
            .completed_at
            .as_ref()
            .and_then(|s| s.parse::<Timestamp>().ok())
            .unwrap_or(now);
        (end.as_second() - start.as_second()).max(0)
    }
}

/// Blacksmith's published list price for the 2-vCPU tier, per minute.
const BS_BASE_PER_MIN: [(&str, f64); 4] = [
    ("ubuntu", 0.004),
    ("arm", 0.0025),
    ("windows", 0.008),
    ("macos", 0.08),
];
const BS_BASE_VCPU: f64 = 2.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunnerKind {
    /// A Blacksmith runner, e.g. `blacksmith-4vcpu-ubuntu-2404`.
    Blacksmith { vcpu: u32, family: String },
    GitHubHosted,
    SelfHosted,
    Unknown,
}

impl RunnerKind {
    pub fn from_labels(labels: &[String]) -> RunnerKind {
        if labels.is_empty() {
            return RunnerKind::Unknown;
        }
        for l in labels {
            let l = l.to_ascii_lowercase();
            if l.starts_with("blacksmith") {
                let vcpu = l
                    .split('-')
                    .find_map(|p| p.strip_suffix("vcpu").and_then(|n| n.parse::<u32>().ok()))
                    .unwrap_or(2);
                // Order matters: `-arm` is a suffix on an ubuntu image name.
                let family = if l.contains("arm") {
                    "arm"
                } else if l.contains("windows") {
                    "windows"
                } else if l.contains("macos") {
                    "macos"
                } else {
                    "ubuntu"
                };
                return RunnerKind::Blacksmith { vcpu, family: family.to_string() };
            }
        }
        if labels.iter().any(|l| l.eq_ignore_ascii_case("self-hosted")) {
            return RunnerKind::SelfHosted;
        }
        RunnerKind::GitHubHosted
    }

    /// Per-minute list price, for runners we have to price ourselves.
    /// GitHub-hosted returns None: that spend comes from the billing API and
    /// must never be double-counted by estimation.
    pub fn rate_per_minute(&self) -> Option<f64> {
        match self {
            RunnerKind::Blacksmith { vcpu, family } => {
                let base = BS_BASE_PER_MIN
                    .iter()
                    .find(|(f, _)| f == family)
                    .map(|(_, r)| *r)
                    .unwrap_or(0.004);
                Some(base * (*vcpu as f64 / BS_BASE_VCPU))
            }
            _ => None,
        }
    }

    pub fn short(&self) -> String {
        match self {
            RunnerKind::Blacksmith { vcpu, family } => format!("blacksmith-{vcpu}vcpu-{family}"),
            RunnerKind::GitHubHosted => "github-hosted".into(),
            RunnerKind::SelfHosted => "self-hosted".into(),
            RunnerKind::Unknown => "unknown".into(),
        }
    }
}

pub fn estimated_cost(kind: &RunnerKind, elapsed_secs: i64) -> Option<Usd> {
    let rate = kind.rate_per_minute()?;
    Some(Usd::from_f64(rate * (elapsed_secs as f64 / 60.0)))
}

#[derive(Deserialize)]
struct ApiRepo {
    name: String,
    pushed_at: Option<String>,
    default_branch: Option<String>,
    owner: ApiOwner,
    archived: bool,
}

#[derive(Deserialize)]
struct ApiOwner {
    login: String,
}

pub fn active_repos(
    http: &Http,
    org: &str,
    active_days: i64,
    max_repos: usize,
) -> Result<Vec<RepoRef>, ApiError> {
    let repos: Vec<ApiRepo> = http.get_json(&format!(
        "/orgs/{org}/repos?sort=pushed&direction=desc&per_page=100"
    ))?;
    let cutoff = Timestamp::now().as_second() - active_days * 86_400;
    let mut out = Vec::new();
    for r in repos {
        if r.archived {
            continue;
        }
        let Some(pushed) = r.pushed_at.clone() else { continue };
        let Ok(ts) = pushed.parse::<Timestamp>() else { continue };
        if ts.as_second() < cutoff {
            continue;
        }
        out.push(RepoRef {
            owner: r.owner.login,
            name: r.name,
            pushed_at: pushed,
            default_branch: r.default_branch.unwrap_or_else(|| "main".into()),
        });
        if out.len() >= max_repos {
            break;
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct RunsResponse {
    workflow_runs: Vec<ApiRun>,
}

#[derive(Deserialize)]
struct ApiRun {
    id: u64,
    name: Option<String>,
    head_branch: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    run_started_at: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    html_url: String,
    repository: Option<ApiRunRepo>,
}

#[derive(Deserialize)]
struct ApiRunRepo {
    name: String,
    default_branch: Option<String>,
    owner: ApiOwner,
}

pub fn recent_runs(
    http: &Http,
    owner: &str,
    repo: &str,
    per_page: u32,
) -> Result<Vec<RunSummary>, ApiError> {
    let r: RunsResponse = http.get_json(&format!(
        "/repos/{owner}/{repo}/actions/runs?per_page={per_page}"
    ))?;
    Ok(r.workflow_runs.into_iter().map(|run| to_summary(run, owner, repo)).collect())
}

fn to_summary(run: ApiRun, owner: &str, repo: &str) -> RunSummary {
    {
        {
            let branch = run.head_branch.unwrap_or_default();
            let default_branch = run
                .repository
                .as_ref()
                .and_then(|x| x.default_branch.clone())
                .unwrap_or_else(|| "main".into());
            RunSummary {
                id: run.id,
                repo: run.repository.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| repo.into()),
                owner: run
                    .repository
                    .as_ref()
                    .map(|x| x.owner.login.clone())
                    .unwrap_or_else(|| owner.into()),
                workflow: run.name.unwrap_or_else(|| "workflow".into()),
                is_default_branch: branch == default_branch,
                branch,
                status: run.status.unwrap_or_default(),
                conclusion: run.conclusion,
                started_at: run.run_started_at.or(run.created_at),
                updated_at: run.updated_at,
                url: run.html_url,
            }
        }
    }
}

#[derive(Deserialize)]
struct JobsResponse {
    jobs: Vec<ApiJob>,
}

#[derive(Deserialize)]
struct ApiJob {
    name: String,
    #[serde(default)]
    labels: Vec<String>,
    status: Option<String>,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

/// Runs created within one calendar month, newest first.
///
/// The range must be bounded at BOTH ends: `created=>=2025-01-01` also matches
/// every later month, which silently turned a historical query into a
/// month-to-date one.
pub fn runs_in_month(
    http: &Http,
    owner: &str,
    repo: &str,
    year: i16,
    month: u8,
    per_page: u32,
) -> Result<Vec<RunSummary>, ApiError> {
    let last = days_in_month(year, month);
    let range = format!("{year:04}-{month:02}-01..{year:04}-{month:02}-{last:02}");
    let r: RunsResponse = http.get_json(&format!(
        "/repos/{owner}/{repo}/actions/runs?created={range}&per_page={per_page}"
    ))?;
    Ok(r.workflow_runs.into_iter().map(|run| to_summary(run, owner, repo)).collect())
}

fn days_in_month(year: i16, month: u8) -> u8 {
    jiff::civil::date(year, month as i8, 1).days_in_month() as u8
}

pub fn jobs_for_run(
    http: &Http,
    owner: &str,
    repo: &str,
    run_id: u64,
) -> Result<Vec<JobInfo>, ApiError> {
    let r: JobsResponse =
        http.get_json(&format!("/repos/{owner}/{repo}/actions/runs/{run_id}/jobs?per_page=100"))?;
    Ok(r.jobs
        .into_iter()
        .map(|j| JobInfo {
            name: j.name,
            labels: j.labels,
            status: j.status.unwrap_or_default(),
            conclusion: j.conclusion,
            started_at: j.started_at,
            completed_at: j.completed_at,
        })
        .collect())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CiStatus {
    pub running: usize,
    pub queued: usize,
    pub in_flight: Vec<InFlight>,
    pub failures: Vec<RunSummary>,
    pub repos_polled: usize,
    /// Estimated cost of Blacksmith work currently in flight.
    pub in_flight_estimate: Usd,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InFlight {
    #[serde(flatten)]
    pub run: RunSummary,
    pub runner: RunnerKind,
    pub estimate: Option<Usd>,
}

impl std::ops::Deref for InFlight {
    type Target = RunSummary;
    fn deref(&self) -> &RunSummary {
        &self.run
    }
}

/// One org's CI picture. Per-repo failures are tolerated: a repo we cannot
/// read degrades to an error line, never to a missing widget.
///
/// Repos are polled concurrently -- serially this takes ~15s for 15 repos,
/// which would block waybar's tick.
pub fn org_status(
    http: &Http,
    org: &str,
    active_days: i64,
    max_repos: usize,
) -> Result<CiStatus, ApiError> {
    let repos = active_repos(http, org, active_days, max_repos)?;
    let now = Timestamp::now();
    let mut st = CiStatus { repos_polled: repos.len(), ..Default::default() };

    let per_repo: Vec<RepoOutcome> = fan_out(&repos, |repo| {
        let runs = match recent_runs(http, &repo.owner, &repo.name, 20) {
            Ok(r) => r,
            Err(e) => {
                return RepoOutcome {
                    error: Some(format!("{}/{}: {}", repo.owner, repo.name, e.short())),
                    ..Default::default()
                }
            }
        };
        let mut out = RepoOutcome::default();
        let in_progress: Vec<&RunSummary> = runs.iter().filter(|r| r.is_running()).collect();
        // Job detail is another request each, so only for runs actually running.
        let job_sets: Vec<Vec<JobInfo>> = fan_out(&in_progress, |run| {
            jobs_for_run(http, &run.owner, &run.repo, run.id).unwrap_or_default()
        });
        for (run, jobs) in in_progress.iter().zip(job_sets) {
            let running_job = jobs
                .iter()
                .find(|j| j.status == "in_progress")
                .or_else(|| jobs.first());
            let kind = running_job.map(|j| j.runner()).unwrap_or(RunnerKind::Unknown);
            let secs: i64 = jobs.iter().map(|j| j.elapsed_secs(now)).sum();
            let estimate = estimated_cost(&kind, secs);
            out.in_flight.push(InFlight { run: (*run).clone(), runner: kind, estimate });
        }
        out.running = in_progress.len();
        out.queued = runs.iter().filter(|r| r.is_queued()).count();

        // A failure counts when it is the newest run of its workflow on the
        // default branch -- i.e. still broken, not merely broken once.
        let mut seen = std::collections::BTreeSet::new();
        for run in runs.iter().filter(|r| r.is_default_branch && r.status == "completed") {
            if !seen.insert(run.workflow.clone()) {
                continue;
            }
            if run.conclusion.as_deref() == Some("failure") {
                out.failures.push(run.clone());
            }
        }
        out
    });

    for r in per_repo {
        st.running += r.running;
        st.queued += r.queued;
        for f in &r.in_flight {
            if let Some(e) = f.estimate {
                st.in_flight_estimate += e;
            }
        }
        st.in_flight.extend(r.in_flight);
        st.failures.extend(r.failures);
        if let Some(e) = r.error {
            st.errors.push(e);
        }
    }
    Ok(st)
}

#[derive(Default)]
struct RepoOutcome {
    running: usize,
    queued: usize,
    in_flight: Vec<InFlight>,
    failures: Vec<RunSummary>,
    error: Option<String>,
}

/// Run `f` over `items` on a bounded set of threads, preserving order.
/// Bounded because GitHub throttles bursts, and because the point is
/// latency, not throughput.
pub fn fan_out<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync + Send,
{
    // GitHub enforces a secondary limit on burst concurrency, separate from
    // the 5,000/hr quota. 8 tripped it repeatedly; 6 has not, and keeps the
    // steady-state tick (mostly 304s) comfortably fast.
    const MAX_CONCURRENCY: usize = 6;
    if items.len() <= 1 {
        return items.iter().map(&f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<R>>> =
        (0..items.len()).map(|_| std::sync::Mutex::new(None)).collect();
    let threads = MAX_CONCURRENCY.min(items.len());

    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                *slots[i].lock().unwrap() = Some(r);
            });
        }
    });

    slots.into_iter().map(|s| s.into_inner().unwrap().unwrap()).collect()
}
