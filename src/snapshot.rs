//! Everything the widget needs to draw itself, gathered from all providers.

use crate::config::Theme;
use crate::cycle::Cycle;
use crate::money::Usd;
use crate::providers::github_billing::Spend;
use crate::providers::github_runs::{InFlight, RunSummary, RunnerKind};
use crate::render::Severity;
use jiff::Timestamp;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub cycle: Cycle,
    pub now: Timestamp,
    pub github: Spend,
    /// Per-org net spend, for the tooltip breakdown.
    pub per_org: Vec<(String, Usd)>,
    /// Blacksmith spend: exact when the dashboard API answered, otherwise the
    /// label-derived estimate.
    pub blacksmith: Option<Usd>,
    pub blacksmith_estimate: Usd,
    pub blacksmith_is_estimate: bool,
    pub budget: Usd,
    pub projected: Usd,
    pub running: usize,
    pub queued: usize,
    pub failures: usize,
    pub failure_runs: Vec<RunSummary>,
    pub in_flight: Vec<InFlight>,
    pub in_flight_estimate: Usd,
    pub repos_polled: usize,
    /// Blacksmith runner capacity in use right now, from their dashboard.
    pub bs_live: Option<(i64, i64)>,
    pub notes: Vec<String>,
    pub stale_reason: Option<String>,
    pub age_secs: u64,
    pub theme: Theme,
}

impl Snapshot {
    pub fn new(now: Timestamp) -> Snapshot {
        Snapshot {
            cycle: Cycle::containing(now),
            now,
            github: Spend::default(),
            per_org: Vec::new(),
            blacksmith: None,
            blacksmith_estimate: Usd::zero(),
            blacksmith_is_estimate: true,
            budget: Usd::zero(),
            projected: Usd::zero(),
            running: 0,
            queued: 0,
            failures: 0,
            failure_runs: Vec::new(),
            in_flight: Vec::new(),
            in_flight_estimate: Usd::zero(),
            repos_polled: 0,
            bs_live: None,
            notes: Vec::new(),
            stale_reason: None,
            age_secs: 0,
            theme: Theme::default(),
        }
    }

    /// Blacksmith spend to display: the exact figure when we have one.
    pub fn blacksmith_usd(&self) -> Usd {
        self.blacksmith.unwrap_or(self.blacksmith_estimate)
    }

    pub fn total(&self) -> Usd {
        self.github.net + self.blacksmith_usd()
    }

    pub fn projected_pct(&self) -> Option<f64> {
        self.projected.pct_of(self.budget)
    }

    pub fn severity(&self) -> Severity {
        match self.projected_pct() {
            None => Severity::Unknown,
            Some(p) if p < 60.0 => Severity::Ok,
            Some(p) if p < 85.0 => Severity::Low,
            Some(p) if p <= 100.0 => Severity::Warning,
            Some(_) => Severity::Critical,
        }
    }

    pub fn recompute_projection(&mut self) {
        self.projected = self.cycle.project(self.total(), self.now);
    }

    /// A fixed snapshot for tests and `--demo`, so rendering can be exercised
    /// without the network.
    pub fn demo() -> Snapshot {
        let now: Timestamp = "2026-08-29T06:30:00Z".parse().unwrap();
        let mut s = Snapshot::new(now);
        s.github.net = Usd::from_f64(232.02);
        s.github.gross = Usd::from_f64(2746.06);
        s.github.discount = s.github.gross - s.github.net;
        s.github
            .per_repo
            .insert("R&D-platform".into(), Usd::from_f64(120.0));
        s.github
            .per_repo
            .insert("widget-service".into(), Usd::from_f64(80.0));
        s.github
            .per_sku
            .insert("Actions macOS 3-core".into(), Usd::from_f64(122.49));
        s.github
            .per_sku
            .insert("Actions Linux".into(), Usd::from_f64(79.98));
        s.per_org = vec![("acme".into(), Usd::from_f64(232.02))];
        s.budget = Usd::from_f64(400.0);
        s.running = 1;
        s.repos_polled = 6;
        s.bs_live = Some((4, 16));
        let run = RunSummary {
            id: 1,
            repo: "widget-service".into(),
            owner: "acme".into(),
            workflow: "CI".into(),
            branch: "main".into(),
            status: "in_progress".into(),
            conclusion: None,
            started_at: Some("2026-08-29T06:24:00Z".into()),
            updated_at: None,
            is_default_branch: true,
            url: "https://github.com/acme/widget-service/actions/runs/1".into(),
        };
        s.in_flight = vec![InFlight {
            run,
            runner: RunnerKind::Blacksmith {
                vcpu: 4,
                family: "ubuntu".into(),
            },
            estimate: Some(Usd::from_f64(0.05)),
        }];
        s.in_flight_estimate = Usd::from_f64(0.05);
        s.recompute_projection();
        s
    }
}
