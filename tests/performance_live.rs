//! A waybar module that blocks for 17s is broken regardless of what it prints.
//! This runs the real binary against the real API with a cold cache.
//!
//! These tests measure latency, so they must not run alongside each other --
//! `run-tests.sh` invokes this suite with `--test-threads=1`.

use std::process::Command;
use std::time::Instant;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cicdbar")
}

#[test]
fn a_cold_run_against_the_real_api_finishes_fast_enough_for_a_60s_tick() {
    let cache = std::env::temp_dir().join(format!("cicdbar-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);

    let t = Instant::now();
    let out = Command::new(bin())
        .args(["--no-cache"])
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .expect("run");
    let elapsed = t.elapsed();

    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("valid waybar json");
    assert!(v["text"].as_str().unwrap().contains('$'));
    // Serial polling of 15 repos took 17s. Waybar spawns the module
    // asynchronously so the bar never freezes, but a tick must still finish
    // well inside the 60s interval, with room for a slow network.
    assert!(
        elapsed.as_secs_f64() < 12.0,
        "cold run took {:.1}s; 15 repos polled serially took 17s, so this is a \
         concurrency regression",
        elapsed.as_secs_f64()
    );
}

#[test]
fn a_warm_run_is_effectively_instant() {
    let cache = std::env::temp_dir().join(format!("cicdbar-perf-warm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    // Prime.
    Command::new(bin()).env("XDG_CACHE_HOME", &cache).output().expect("prime");

    let t = Instant::now();
    let out = Command::new(bin()).env("XDG_CACHE_HOME", &cache).output().expect("run");
    let elapsed = t.elapsed();
    assert!(out.status.success());
    assert!(
        elapsed.as_secs_f64() < 0.5,
        "warm run took {:.2}s; the cache is not being used",
        elapsed.as_secs_f64()
    );
}

#[test]
fn a_tick_stays_well_inside_the_rate_limit_budget() {
    // GitHub's own rate_limit counter is eventually consistent and resets
    // mid-measurement, so the request count is taken from inside the client.
    let cache = std::env::temp_dir().join(format!("cicdbar-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);

    let stats = |args: &[&str]| -> (usize, usize) {
        let out = Command::new(bin())
            .args(args)
            .arg("--stats")
            .env("XDG_CACHE_HOME", &cache)
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&out.stderr);
        let get = |k: &str| {
            err.split_whitespace()
                .find_map(|f| f.strip_prefix(k)?.parse::<usize>().ok())
                .unwrap_or_else(|| panic!("no {k} in {err:?}"))
        };
        (get("requests="), get("not_modified="))
    };

    let (cold, _) = stats(&[]);
    assert!(cold < 60, "a cold tick issued {cold} requests; that is a burst-limit risk");

    // A warm tick inside the cache TTL must touch the network at all.
    let (warm, _) = stats(&[]);
    assert_eq!(warm, 0, "the cache should absorb a tick entirely");

    // With the cache bypassed, ETags must turn almost everything into a 304,
    // and 304s do not count against the REST rate limit.
    let (reqs, not_modified) = stats(&["--no-cache"]);
    assert!(reqs > 0);
    assert!(
        not_modified * 4 >= reqs * 3,
        "expected most requests to be 304s, got {not_modified}/{reqs}"
    );
}
