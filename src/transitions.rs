//! Which runs started and which finished, since the last tick.
//!
//! Pure logic over (previous state, runs seen now). cicdbar is re-exec'd by
//! waybar every 60s, so "since last tick" means "since the state file was
//! last written".

use crate::providers::github_runs::RunSummary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Completed runs are forgotten after this long. Without pruning the state
/// file grows without bound; with too short a window a finish could be
/// announced twice.
const RETAIN_SECS: u64 = 6 * 3600;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyState {
    pub runs: BTreeMap<u64, Seen>,
    /// False until the first diff has run, so a cold start stays silent.
    #[serde(default)]
    pub seeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seen {
    pub status: String,
    pub conclusion: Option<String>,
    /// The notification this run currently occupies, so a finish can replace
    /// the start rather than stacking a second one.
    pub notif_id: Option<u32>,
    pub last_seen: u64,
}

#[derive(Debug, Clone)]
pub enum Event {
    Started(RunSummary),
    Finished {
        run: RunSummary,
        previous_notif_id: Option<u32>,
    },
}

impl Event {
    pub fn run(&self) -> &RunSummary {
        match self {
            Event::Started(r) => r,
            Event::Finished { run, .. } => run,
        }
    }
    pub fn is_failure(&self) -> bool {
        matches!(self, Event::Finished { run, .. } if run.conclusion.as_deref() == Some("failure"))
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_complete(status: &str) -> bool {
    status == "completed"
}

/// Diff `runs` against `state`, returning the events to announce and the
/// state to persist. Events are ordered by run id, which is monotonic at
/// GitHub, so notifications arrive oldest-first.
pub fn diff(state: &NotifyState, runs: &[RunSummary]) -> (Vec<Event>, NotifyState) {
    diff_at(state, runs, now_secs())
}

pub fn diff_at(state: &NotifyState, runs: &[RunSummary], now: u64) -> (Vec<Event>, NotifyState) {
    let mut next = state.clone();
    let mut events = Vec::new();

    let mut sorted: Vec<&RunSummary> = runs.iter().collect();
    sorted.sort_by_key(|r| r.id);

    for run in sorted {
        let previous = state.runs.get(&run.id);
        let complete = is_complete(&run.status);

        // A cold start seeds silently: announcing every in-flight run the
        // first time the widget ever runs is noise, not news.
        if state.seeded {
            match previous {
                None if !complete => events.push(Event::Started(run.clone())),
                None if complete => events.push(Event::Finished {
                    run: run.clone(),
                    previous_notif_id: None,
                }),
                Some(p) if !is_complete(&p.status) && complete => events.push(Event::Finished {
                    run: run.clone(),
                    previous_notif_id: p.notif_id,
                }),
                _ => {}
            }
        }

        next.runs.insert(
            run.id,
            Seen {
                status: run.status.clone(),
                conclusion: run.conclusion.clone(),
                notif_id: previous.and_then(|p| p.notif_id),
                last_seen: now,
            },
        );
    }

    next.runs
        .retain(|_, s| !is_complete(&s.status) || now.saturating_sub(s.last_seen) < RETAIN_SECS);
    next.seeded = true;
    (events, next)
}

// ---------------------------------------------------------------------------
// Filtering and rendering
// ---------------------------------------------------------------------------

use crate::config::NotificationConfig;
use crate::notify::Urgency;

/// Does this event pass the user's filters?
///
/// `after_failure` says whether the previous run of this workflow failed,
/// which is what makes a success newsworthy under "failures-and-recoveries".
pub fn should_notify(cfg: &NotificationConfig, event: &Event, after_failure: bool) -> bool {
    if !cfg.enabled {
        return false;
    }
    let run = event.run();

    if !cfg.repos.is_empty() && !cfg.repos.iter().any(|r| r == &run.repo) {
        return false;
    }
    if cfg.only_default_branch && !run.is_default_branch {
        return false;
    }

    match event {
        Event::Started(_) => cfg.on_start,
        Event::Finished { run, .. } => {
            let failed = run.conclusion.as_deref() == Some("failure");
            match cfg.on_finish.as_str() {
                "all" => true,
                "failures" => failed,
                "failures-and-recoveries" => failed || after_failure,
                _ => false,
            }
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Summary, body and urgency for a notification. The body carries Pango
/// markup, so anything interpolated into it is escaped.
pub fn render(event: &Event) -> (String, String, Urgency) {
    let run = event.run();
    let where_ = format!("{} · {}", run.repo, run.workflow);

    match event {
        Event::Started(_) => (
            format!("▶ {where_} started"),
            escape(&run.branch),
            Urgency::Low,
        ),
        Event::Finished { run, .. } => {
            let (glyph, word, urgency) = match run.conclusion.as_deref() {
                Some("success") => ("✔", "succeeded", Urgency::Normal),
                Some("failure") => ("✖", "failed", Urgency::Critical),
                Some("cancelled") => ("⊘", "cancelled", Urgency::Low),
                Some("timed_out") => ("⏱", "timed out", Urgency::Normal),
                Some(other) => ("•", other, Urgency::Normal),
                None => ("•", "finished", Urgency::Normal),
            };
            (
                format!("{glyph} {where_} {word}"),
                escape(&run.branch),
                urgency,
            )
        }
    }
}
