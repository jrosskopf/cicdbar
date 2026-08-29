//! Notification tests against the REAL session D-Bus and the REAL daemon.
//! Assertions read the notification back out of the daemon with `makoctl`.
//!
//! Run serially (`--test-threads=1`): the daemon is global shared state, and
//! concurrent tests dismiss each other's notifications.

use cicdbar::notify::{Notifier, Urgency};

fn makoctl_list() -> String {
    std::process::Command::new("makoctl")
        .arg("list")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// mako holds the full notification, body included, which `makoctl list`
/// does not print. This is the only way to assert on what we actually sent.
fn mako_notifications() -> String {
    std::process::Command::new("busctl")
        .args([
            "--user",
            "--json=short",
            "call",
            "org.freedesktop.Notifications",
            "/fr/emersion/Mako",
            "fr.emersion.Mako",
            "ListNotifications",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn dismiss_all() {
    let _ = std::process::Command::new("makoctl")
        .args(["dismiss", "-a"])
        .output();
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn a_notification_actually_reaches_the_daemon() {
    dismiss_all();
    let n = Notifier::connect().expect("session bus");
    let id = n
        .send(
            None,
            "cicdbar-test alpha",
            "body of alpha",
            Urgency::Normal,
            "t1",
        )
        .expect("send");
    assert!(id > 0, "daemon must return a notification id");

    let listed = makoctl_list();
    assert!(
        listed.contains("cicdbar-test alpha"),
        "not in daemon: {listed}"
    );
    assert!(
        listed.contains("cicdbar"),
        "app name must be cicdbar: {listed}"
    );
    dismiss_all();
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn replacing_updates_in_place_instead_of_stacking() {
    // This is what keeps start+finish to one notification per run.
    dismiss_all();
    let n = Notifier::connect().expect("session bus");
    let id = n
        .send(
            None,
            "cicdbar-test running",
            "started",
            Urgency::Low,
            "run-42",
        )
        .expect("send");
    let before = makoctl_list().matches("Notification").count();

    let id2 = n
        .send(
            Some(id),
            "cicdbar-test finished",
            "succeeded",
            Urgency::Normal,
            "run-42",
        )
        .expect("replace");
    assert_eq!(id2, id, "replacing must reuse the id");

    let listed = makoctl_list();
    let after = listed.matches("Notification").count();
    assert_eq!(after, before, "replacement must not add a notification");
    assert!(
        listed.contains("cicdbar-test finished"),
        "content must update: {listed}"
    );
    assert!(
        !listed.contains("cicdbar-test running"),
        "old text must be gone"
    );
    dismiss_all();
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn a_failure_is_sent_as_critical_so_it_persists() {
    dismiss_all();
    let n = Notifier::connect().expect("session bus");
    n.send(
        None,
        "cicdbar-test broke",
        "a failure",
        Urgency::Critical,
        "t2",
    )
    .expect("send");
    // mako holds critical notifications until dismissed; a normal one with a
    // short timeout would be gone. Presence after a pause is the assertion.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        makoctl_list().contains("cicdbar-test broke"),
        "critical must persist"
    );
    dismiss_all();
}

#[test]
fn a_disabled_notifier_is_silent_and_never_fails() {
    // --no-notify, --demo, and `enabled = false` all take this path, which
    // must work with no bus at all.
    let n = Notifier::disabled();
    let id = n
        .send(None, "should not appear", "", Urgency::Normal, "x")
        .expect("no-op");
    assert_eq!(id, 0);
    assert!(!makoctl_list().contains("should not appear"));
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn replacing_repeatedly_does_not_kill_the_daemon() {
    // Regression guard for a real mako crash: replacing a notification that
    // carries x-dunst-stack-tag takes the daemon down, which would leave the
    // user with no notifications at all until they restart it.
    dismiss_all();
    let n = Notifier::connect().expect("session bus");
    let mut id = n
        .send(None, "cicdbar-test churn", "0", Urgency::Low, "run-99")
        .expect("first");
    for i in 1..6 {
        id = n
            .send(
                Some(id),
                "cicdbar-test churn",
                &i.to_string(),
                Urgency::Low,
                "run-99",
            )
            .unwrap_or_else(|e| panic!("replace {i} failed -- daemon may have crashed: {e}"));
    }
    assert!(
        makoctl_list().contains("cicdbar-test churn"),
        "daemon must still be serving notifications"
    );
    dismiss_all();
}

#[test]
#[ignore = "runs the real binary against the live API and the real daemon"]
fn the_real_binary_notifies_about_a_real_run_transition() {
    // End to end: prime state from the live API, doctor it so a real run
    // looks newly-finished, run the binary again, and read the notification
    // out of the actual daemon.
    dismiss_all();
    let cache = std::env::temp_dir().join(format!("cicdbar-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let bin = env!("CARGO_BIN_EXE_cicdbar");

    // Tick 1: seeds silently.
    let out = std::process::Command::new(bin)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .expect("first tick");
    assert!(out.status.success());
    assert!(
        !makoctl_list().contains("cicdbar"),
        "a cold start must not notify: {}",
        makoctl_list()
    );

    // Doctor the state: mark one known run as still in progress, so the next
    // tick sees it complete.
    let state_path = cache.join("cicdbar").join("notify-state.json");
    let raw = std::fs::read_to_string(&state_path).expect("state written");
    let mut v: serde_json::Value = serde_json::from_str(&raw).expect("state json");
    let runs = v["value"]["runs"].as_object_mut().expect("runs map");
    let key = runs
        .iter()
        .find(|(_, s)| s["status"] == "completed")
        .map(|(k, _)| k.clone())
        .expect("at least one completed run in the org");
    runs[&key]["status"] = serde_json::Value::String("in_progress".into());
    runs[&key]["conclusion"] = serde_json::Value::Null;
    std::fs::write(&state_path, serde_json::to_string(&v).unwrap()).unwrap();

    // Tick 2: that run now reads as finished.
    let out = std::process::Command::new(bin)
        .args(["--no-cache"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .expect("second tick");
    assert!(out.status.success());

    let listed = makoctl_list();
    assert!(
        listed.contains("App name: cicdbar"),
        "the binary should have raised a real notification; daemon shows: {listed}"
    );
    dismiss_all();
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn the_body_we_send_is_the_body_the_daemon_records() {
    let n = Notifier::connect().expect("session bus");
    let body = "main · 1m49s · blacksmith-4vcpu-ubuntu · ~$0.19";
    n.send(None, "cicdbar-test body check", body, Urgency::Normal, "b1")
        .expect("send");

    let history = mako_notifications();
    assert!(
        history.contains("cicdbar-test body check"),
        "summary missing from the daemon's record"
    );
    assert!(
        history.contains("blacksmith-4vcpu-ubuntu"),
        "the runner and cost must survive the round trip: {}",
        &history[..history.len().min(400)]
    );
    dismiss_all();
}
