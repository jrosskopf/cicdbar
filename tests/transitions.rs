//! The start/finish diff. Pure logic, so it runs offline -- but the run
//! payloads are shaped exactly as GitHub returns them.

use cicdbar::providers::github_runs::RunSummary;
use cicdbar::transitions::{diff, Event, NotifyState};

fn run(id: u64, status: &str, conclusion: Option<&str>) -> RunSummary {
    RunSummary {
        id,
        repo: "heron".into(),
        owner: "DataZooDE".into(),
        workflow: "Build".into(),
        branch: "fix/worker-binary-race".into(),
        status: status.into(),
        conclusion: conclusion.map(str::to_string),
        started_at: Some("2026-08-29T06:24:00Z".into()),
        updated_at: None,
        is_default_branch: false,
        url: "https://github.com/DataZooDE/heron/actions/runs/1".into(),
    }
}

#[test]
fn a_cold_start_notifies_nothing() {
    // Otherwise first launch fires once per in-flight run.
    let state = NotifyState::default();
    let (events, next) = diff(
        &state,
        &[run(1, "in_progress", None), run(2, "queued", None)],
    );
    assert!(
        events.is_empty(),
        "cold start must be silent, got {events:?}"
    );
    assert_eq!(next.runs.len(), 2, "but it must still seed the state");
}

#[test]
fn a_new_run_appearing_is_a_start() {
    let (_, seeded) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let (events, _) = diff(
        &seeded,
        &[run(1, "in_progress", None), run(2, "in_progress", None)],
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], Event::Started(r) if r.id == 2));
}

#[test]
fn a_run_completing_is_a_finish_that_replaces_its_start() {
    let (_, seeded) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let mut seeded = seeded;
    seeded.runs.get_mut(&1).unwrap().notif_id = Some(4242);

    let (events, next) = diff(&seeded, &[run(1, "completed", Some("failure"))]);
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::Finished {
            run,
            previous_notif_id,
            ..
        } => {
            assert_eq!(run.id, 1);
            assert_eq!(*previous_notif_id, Some(4242), "must replace, not stack");
        }
        other => panic!("expected Finished, got {other:?}"),
    }
    assert_eq!(next.runs.get(&1).unwrap().status, "completed");
}

#[test]
fn a_run_that_starts_and_ends_inside_one_tick_still_notifies_once() {
    let (_, seeded) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let (events, _) = diff(
        &seeded,
        &[
            run(1, "in_progress", None),
            run(9, "completed", Some("success")),
        ],
    );
    assert_eq!(events.len(), 1, "one finish, not a start and a finish");
    match &events[0] {
        Event::Finished {
            run,
            previous_notif_id,
            ..
        } => {
            assert_eq!(run.id, 9);
            assert_eq!(*previous_notif_id, None);
        }
        other => panic!("expected Finished, got {other:?}"),
    }
}

#[test]
fn an_unchanged_run_notifies_nothing() {
    let (_, seeded) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let (events, _) = diff(&seeded, &[run(1, "in_progress", None)]);
    assert!(events.is_empty());
}

#[test]
fn a_completed_run_is_never_announced_twice() {
    let (_, s1) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let (e2, s2) = diff(&s1, &[run(1, "completed", Some("success"))]);
    assert_eq!(e2.len(), 1);
    let (e3, _) = diff(&s2, &[run(1, "completed", Some("success"))]);
    assert!(
        e3.is_empty(),
        "the same finish must not fire on every later tick"
    );
}

#[test]
fn old_completed_runs_are_pruned_so_the_state_file_cannot_grow_forever() {
    let mut state = NotifyState::default();
    for i in 0..500u64 {
        state.runs.insert(
            i,
            cicdbar::transitions::Seen {
                status: "completed".into(),
                conclusion: Some("success".into()),
                notif_id: None,
                runner: None,
                estimate: None,
                last_seen: 0, // epoch: ancient
            },
        );
    }
    let (_, next) = diff(&state, &[run(1000, "in_progress", None)]);
    assert!(
        next.runs.len() < 50,
        "ancient entries must be pruned, kept {}",
        next.runs.len()
    );
}

#[test]
fn events_are_ordered_oldest_first_so_notifications_arrive_in_sequence() {
    let (_, seeded) = diff(&NotifyState::default(), &[run(1, "in_progress", None)]);
    let (events, _) = diff(
        &seeded,
        &[
            run(1, "in_progress", None),
            run(5, "in_progress", None),
            run(3, "in_progress", None),
        ],
    );
    let ids: Vec<u64> = events.iter().map(|e| e.run().id).collect();
    assert_eq!(ids, vec![3, 5]);
}

// ---- filtering and rendering ----

use cicdbar::config::NotificationConfig;
use cicdbar::transitions::{render, should_notify};

fn cfg() -> NotificationConfig {
    NotificationConfig::default()
}

fn fin(r: RunSummary) -> Event {
    Event::Finished {
        run: r,
        previous_notif_id: None,
        runner: None,
        estimate: None,
    }
}

#[test]
fn on_finish_failures_suppresses_successes_but_not_failures() {
    let mut c = cfg();
    c.on_start = false;
    c.on_finish = "failures".into();
    let ok = Event::Finished {
        run: run(1, "completed", Some("success")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    let bad = Event::Finished {
        run: run(2, "completed", Some("failure")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    assert!(!should_notify(&c, &ok, false));
    assert!(should_notify(&c, &bad, false));
    assert!(!should_notify(
        &c,
        &Event::Started(run(3, "in_progress", None)),
        false
    ));
}

#[test]
fn failures_and_recoveries_adds_the_first_success_after_a_failure() {
    let mut c = cfg();
    c.on_finish = "failures-and-recoveries".into();
    let ok = Event::Finished {
        run: run(1, "completed", Some("success")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    assert!(
        !should_notify(&c, &ok, false),
        "a success after a success is not news"
    );
    assert!(
        should_notify(&c, &ok, true),
        "but a success after a failure is"
    );
}

#[test]
fn the_default_config_notifies_starts_and_every_finish() {
    let c = cfg();
    assert!(should_notify(
        &c,
        &Event::Started(run(1, "in_progress", None)),
        false
    ));
    let ok = Event::Finished {
        run: run(2, "completed", Some("success")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    assert!(should_notify(&c, &ok, false));
}

#[test]
fn a_repo_allowlist_excludes_everything_else() {
    let mut c = cfg();
    c.repos = vec!["something-else".into()];
    assert!(!should_notify(
        &c,
        &Event::Started(run(1, "in_progress", None)),
        false
    ));
    c.repos = vec!["heron".into()];
    assert!(should_notify(
        &c,
        &Event::Started(run(1, "in_progress", None)),
        false
    ));
}

#[test]
fn only_default_branch_filters_feature_branches() {
    let mut c = cfg();
    c.only_default_branch = true;
    assert!(!should_notify(
        &c,
        &Event::Started(run(1, "in_progress", None)),
        false
    ));
    let mut r = run(2, "in_progress", None);
    r.is_default_branch = true;
    assert!(should_notify(&c, &Event::Started(r), false));
}

#[test]
fn cancelled_runs_are_not_reported_as_failures() {
    let cancelled = Event::Finished {
        run: run(1, "completed", Some("cancelled")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    let (summary, _, urgency) = render(&cancelled);
    assert!(!summary.contains("failed"), "got {summary}");
    assert_ne!(urgency, cicdbar::notify::Urgency::Critical);
}

#[test]
fn the_body_carries_duration_runner_and_blacksmith_cost() {
    // The cost line is the thing a generic CI notifier cannot tell you.
    let mut r = run(1, "completed", Some("failure"));
    r.started_at = Some("2026-08-29T06:24:00Z".into());
    r.updated_at = Some("2026-08-29T06:28:12Z".into());
    let e = Event::Finished {
        run: r,
        previous_notif_id: None,
        runner: Some("blacksmith-4vcpu-ubuntu".into()),
        estimate: Some(cicdbar::money::Usd::from_f64(0.19)),
    };
    let (_, body, _) = render(&e);
    assert!(body.contains("fix/worker-binary-race"), "{body}");
    assert!(body.contains("4m12s"), "duration missing: {body}");
    assert!(
        body.contains("blacksmith-4vcpu-ubuntu"),
        "runner missing: {body}"
    );
    assert!(body.contains("$0.19"), "cost missing: {body}");
}

#[test]
fn a_github_hosted_run_shows_no_invented_cost() {
    // Only Blacksmith minutes are priced locally; GitHub spend comes from the
    // billing API and must never be guessed at per-run.
    let mut r = run(1, "completed", Some("success"));
    r.updated_at = Some("2026-08-29T06:25:00Z".into());
    let e = Event::Finished {
        run: r,
        previous_notif_id: None,
        runner: Some("github-hosted".into()),
        estimate: None,
    };
    let (_, body, _) = render(&e);
    assert!(body.contains("github-hosted"));
    assert!(!body.contains('$'), "no cost should be shown: {body}");
}

#[test]
fn a_body_degrades_gracefully_when_nothing_extra_is_known() {
    let mut r = run(1, "completed", Some("success"));
    r.updated_at = None;
    let e = Event::Finished {
        run: r,
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    let (_, body, _) = render(&e);
    assert_eq!(
        body, "fix/worker-binary-race",
        "just the branch, no stray separators"
    );
}

#[test]
fn a_failure_renders_as_critical_and_names_the_repo_and_workflow() {
    let e = Event::Finished {
        run: run(1, "completed", Some("failure")),
        previous_notif_id: None,
        runner: None,
        estimate: None,
    };
    let (summary, body, urgency) = render(&e);
    assert!(summary.contains("heron"));
    assert!(summary.contains("Build"));
    assert!(summary.to_lowercase().contains("fail"));
    assert!(
        body.contains("fix/worker-binary-race"),
        "branch belongs in the body: {body}"
    );
    assert_eq!(urgency, cicdbar::notify::Urgency::Critical);
}

#[test]
fn a_start_renders_as_low_urgency() {
    let (_, _, urgency) = render(&Event::Started(run(1, "in_progress", None)));
    assert_eq!(urgency, cicdbar::notify::Urgency::Low);
}

#[test]
fn markup_in_a_branch_name_cannot_break_the_notification_body() {
    let mut r = run(1, "completed", Some("success"));
    r.branch = "feature/a&b<c>".into();
    let (_, body, _) = render(&fin(r));
    assert!(
        body.contains("&amp;") && body.contains("&lt;"),
        "unescaped: {body}"
    );
}
