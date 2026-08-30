//! The rollup. waybar re-execs cicdbar every 60s -- ~1,440 times a day, and
//! once per output, so ~2,880 on two monitors. Emitting per invocation would
//! be a firehose and would put an HTTPS round trip on a 4ms warm path.

use cicdbar::telemetry::rollup::{self, RollupState};

#[test]
fn a_tick_only_increments_a_counter() {
    let mut s = RollupState::default();
    let now = 1_800_000_000;
    for _ in 0..500 {
        assert!(!rollup::should_flush(&s, now), "no tick may trigger a send");
        rollup::record_tick(&mut s, now);
    }
    assert_eq!(s.ticks, 500);
}

#[test]
fn it_flushes_once_a_day_not_once_a_tick() {
    let mut s = RollupState::default();
    let t0 = 1_800_000_000;
    rollup::record_tick(&mut s, t0);
    assert!(!rollup::should_flush(&s, t0));
    assert!(
        !rollup::should_flush(&s, t0 + 23 * 3600),
        "23h is not yet a day"
    );
    assert!(rollup::should_flush(&s, t0 + 24 * 3600 + 1));

    // After flushing, the window restarts and counters reset.
    let flushed = rollup::take_for_flush(&mut s, t0 + 24 * 3600 + 1);
    assert_eq!(flushed.ticks, 1);
    assert_eq!(s.ticks, 0);
    assert!(!rollup::should_flush(&s, t0 + 24 * 3600 + 2));
}

#[test]
fn the_very_first_run_does_not_flush_immediately() {
    // Otherwise every fresh install sends an event describing one tick.
    let s = RollupState::default();
    assert!(!rollup::should_flush(&s, 1_800_000_000));
}

#[test]
fn counts_are_bucketed_never_exact() {
    // Exact counts are identifying; buckets are not.
    assert_eq!(rollup::bucket(0), "0");
    assert_eq!(rollup::bucket(1), "1");
    assert_eq!(rollup::bucket(3), "2-5");
    assert_eq!(rollup::bucket(5), "2-5");
    assert_eq!(rollup::bucket(12), "6-20");
    assert_eq!(rollup::bucket(21), "20+");
    assert_eq!(rollup::bucket(99999), "20+");
}

#[test]
fn the_rollup_carries_no_money_and_no_names() {
    let mut s = RollupState::default();
    let t0 = 1_800_000_000;
    s.orgs = 2;
    s.repos = 15;
    s.blacksmith_enabled = true;
    s.degraded = 3;
    rollup::record_tick(&mut s, t0);
    let props = rollup::properties(&rollup::take_for_flush(&mut s, t0 + 90_000));
    let keys: Vec<&str> = props.iter().map(|(k, _)| *k).collect();
    for forbidden in ["total_usd", "spend", "budget", "amount", "org", "repo_name"] {
        assert!(!keys.contains(&forbidden), "rollup carries {forbidden}");
    }
    assert!(keys.contains(&"orgs_bucket"));
    assert!(keys.contains(&"repos_bucket"));
}

#[test]
fn state_survives_a_round_trip_through_disk_format() {
    let s = RollupState {
        ticks: 42,
        orgs: 2,
        ..Default::default()
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: RollupState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.ticks, 42);
    assert_eq!(back.orgs, 2);
}

#[test]
fn an_error_class_is_recorded_at_most_once_per_window() {
    let mut s = RollupState::default();
    for _ in 0..50 {
        rollup::record_error(&mut s, "rate_limited");
    }
    rollup::record_error(&mut s, "unreachable");
    assert_eq!(s.errors.len(), 2, "one entry per class, not per occurrence");
}

// ---- against the real binary ----

#[test]
#[ignore = "runs the real binary against the live API"]
fn a_real_tick_sends_no_telemetry_and_stays_fast() {
    // The whole point of the rollup: the warm path must not gain a network
    // round trip. Point telemetry at a real closed port -- if the binary
    // tried to send, it would stall on connect.
    let cache = std::env::temp_dir().join(format!("cicdbar-tel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let bin = env!("CARGO_BIN_EXE_cicdbar");

    // Prime, so the second run is a warm tick.
    std::process::Command::new(bin)
        .env("XDG_CACHE_HOME", &cache)
        .env("DATAZOO_TELEMETRY_HOST", "http://127.0.0.1:1")
        .output()
        .expect("prime");

    let t = std::time::Instant::now();
    let out = std::process::Command::new(bin)
        .env("XDG_CACHE_HOME", &cache)
        .env("DATAZOO_TELEMETRY_HOST", "http://127.0.0.1:1")
        .output()
        .expect("warm tick");
    let elapsed = t.elapsed();
    assert!(out.status.success());
    assert!(
        elapsed.as_millis() < 400,
        "a warm tick took {elapsed:?}; telemetry must not be on that path"
    );

    // And the rollup state is being kept.
    let state = cache.join("cicdbar").join("telemetry-rollup.json");
    assert!(state.exists(), "rollup state should be written every tick");
    let raw = std::fs::read_to_string(&state).unwrap();
    assert!(raw.contains("ticks"), "state: {raw}");
    assert!(!raw.contains('$'), "no currency may be persisted either");
}

#[test]
#[ignore = "runs the real binary against the live API"]
fn no_telemetry_flag_writes_no_state_at_all() {
    let cache = std::env::temp_dir().join(format!("cicdbar-tel-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    std::process::Command::new(env!("CARGO_BIN_EXE_cicdbar"))
        .args(["--no-telemetry"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .expect("run");
    assert!(
        !cache.join("cicdbar").join("telemetry-rollup.json").exists(),
        "--no-telemetry must not even keep local state"
    );
}
