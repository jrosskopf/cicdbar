//! A waybar module that blocks for 17s is broken regardless of what it prints.
//! This runs the real binary against the real API with a cold cache.

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
    assert!(
        elapsed.as_secs_f64() < 6.0,
        "cold run took {:.1}s; waybar ticks every 60s and this blocks the bar",
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
