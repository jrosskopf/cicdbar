//! Workflow-run tests against the real GitHub API and real org data.

use cicdbar::http::Http;
use cicdbar::money::Usd;
use cicdbar::providers::github_runs::{self, RunnerKind};
use cicdbar::token::TokenSource;

fn http() -> Http {
    Http::new(TokenSource::GhCli.resolve().expect("token")).expect("client")
}

#[test]
fn discovers_repos_pushed_within_the_window() {
    let repos = github_runs::active_repos(&http(), "DataZooDE", 7, 100).expect("repos");
    assert!(!repos.is_empty(), "DataZooDE has had pushes in the last week");
    assert!(repos.len() < 100);
    // Ordered most-recently-pushed first, so max_repos truncates the least
    // interesting repos rather than an arbitrary set.
    assert!(repos.windows(2).all(|w| w[0].pushed_at >= w[1].pushed_at));
}

#[test]
fn respects_the_max_repos_cap() {
    let repos = github_runs::active_repos(&http(), "DataZooDE", 30, 3).expect("repos");
    assert_eq!(repos.len(), 3);
}

#[test]
fn a_zero_day_window_yields_nothing_rather_than_everything() {
    let repos = github_runs::active_repos(&http(), "DataZooDE", 0, 100).expect("repos");
    assert!(repos.is_empty());
}

#[test]
fn reads_real_workflow_runs_for_an_active_repo() {
    let runs = github_runs::recent_runs(&http(), "DataZooDE", "heron", 20).expect("runs");
    assert!(!runs.is_empty());
    for r in &runs {
        assert!(!r.workflow.is_empty());
        assert!(!r.branch.is_empty());
        assert!(r.started_at.is_some(), "every run has a start time");
    }
}

#[test]
fn classifies_real_runner_labels() {
    // These are labels observed in this org's actual jobs.
    assert_eq!(RunnerKind::from_labels(&["blacksmith-4vcpu-ubuntu-2404".into()]),
               RunnerKind::Blacksmith { vcpu: 4, family: "ubuntu".into() });
    assert_eq!(RunnerKind::from_labels(&["blacksmith-8vcpu-ubuntu-2404-arm".into()]),
               RunnerKind::Blacksmith { vcpu: 8, family: "arm".into() });
    assert_eq!(RunnerKind::from_labels(&["ubuntu-latest".into()]), RunnerKind::GitHubHosted);
    assert_eq!(RunnerKind::from_labels(&["macos-15-intel".into()]), RunnerKind::GitHubHosted);
    assert_eq!(RunnerKind::from_labels(&["self-hosted".into(), "linux".into()]),
               RunnerKind::SelfHosted);
    assert_eq!(RunnerKind::from_labels(&[]), RunnerKind::Unknown);
}

#[test]
fn blacksmith_minute_rate_scales_with_vcpu() {
    let four = RunnerKind::Blacksmith { vcpu: 4, family: "ubuntu".into() };
    let eight = RunnerKind::Blacksmith { vcpu: 8, family: "ubuntu".into() };
    let r4 = four.rate_per_minute().expect("rate");
    let r8 = eight.rate_per_minute().expect("rate");
    assert!((r8 - r4 * 2.0).abs() < 1e-9, "linear in vcpu");
    // Published base: ubuntu x64 $0.004/min at the 2-vcpu tier.
    assert!((four.rate_per_minute().unwrap() - 0.008).abs() < 1e-9);
    let arm = RunnerKind::Blacksmith { vcpu: 2, family: "arm".into() };
    assert!((arm.rate_per_minute().unwrap() - 0.0025).abs() < 1e-9);
    assert!(RunnerKind::GitHubHosted.rate_per_minute().is_none(),
            "github-hosted cost comes from the billing API, never estimated");
}

#[test]
fn fetches_real_jobs_with_labels_and_timings() {
    let h = http();
    let runs = github_runs::recent_runs(&h, "DataZooDE", "heron", 5).expect("runs");
    let run = runs.first().expect("at least one run");
    let jobs = github_runs::jobs_for_run(&h, "DataZooDE", "heron", run.id).expect("jobs");
    assert!(!jobs.is_empty());
    assert!(jobs.iter().any(|j| !j.labels.is_empty()));
}

#[test]
fn in_flight_cost_accrues_with_elapsed_minutes() {
    let kind = RunnerKind::Blacksmith { vcpu: 4, family: "ubuntu".into() };
    // 10 minutes at $0.008/min
    let c = github_runs::estimated_cost(&kind, 600);
    assert_eq!(c, Some(Usd::from_f64(0.08)));
    assert_eq!(github_runs::estimated_cost(&kind, 0), Some(Usd::zero()));
    assert_eq!(github_runs::estimated_cost(&RunnerKind::GitHubHosted, 600), None);
}

#[test]
fn summarises_ci_status_across_the_org_from_real_data() {
    let h = http();
    let status = github_runs::org_status(&h, "DataZooDE", 7, 6).expect("status");
    // Whatever CI is doing right now, the invariants hold.
    assert!(status.running <= status.in_flight.len() + status.queued);
    for r in &status.in_flight {
        assert!(r.started_at.is_some());
        assert!(!r.repo.is_empty());
    }
    for f in &status.failures {
        assert_eq!(f.conclusion.as_deref(), Some("failure"));
    }
    assert!(status.repos_polled > 0);
}
