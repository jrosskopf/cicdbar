//! Config is read from a real file on the real filesystem; cycle maths is
//! driven by real timestamps.

use cicdbar::config::Config;
use cicdbar::cycle::Cycle;
use cicdbar::money::Usd;
use cicdbar::token::TokenSource;
use jiff::Timestamp;

fn tmpdir() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("cicdbar-test-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn loads_a_real_config_file() {
    let dir = tmpdir();
    let path = dir.join("config.toml");
    std::fs::write(&path, r#"
budget_usd = 400.0

[github]
orgs = ["DataZooDE", "anofox"]
token_source = "gh-cli"

[runs]
active_days = 7
max_repos = 40

[blacksmith]
enabled = true
org = "DataZooDE"
"#).unwrap();

    let cfg = Config::load(&path).expect("load");
    assert_eq!(cfg.budget_usd, Usd::from_f64(400.0));
    assert_eq!(cfg.github.orgs, vec!["DataZooDE", "anofox"]);
    assert_eq!(cfg.github.token_source, TokenSource::GhCli);
    assert_eq!(cfg.runs.active_days, 7);
    assert_eq!(cfg.runs.max_repos, 40);
    assert!(cfg.blacksmith.enabled);
}

#[test]
fn a_missing_config_file_yields_usable_defaults() {
    let cfg = Config::load(&tmpdir().join("does-not-exist.toml")).expect("defaults");
    assert!(cfg.github.orgs.is_empty());
    assert_eq!(cfg.runs.active_days, 7);
    assert_eq!(cfg.cache.billing_ttl_secs, 900);
    assert_eq!(cfg.cache.runs_ttl_secs, 30);
}

#[test]
fn an_unknown_key_is_rejected_loudly() {
    let dir = tmpdir();
    let path = dir.join("bad.toml");
    std::fs::write(&path, "budget_usd = 1.0\nbudgte_usd = 2.0\n").unwrap();
    assert!(Config::load(&path).is_err(), "typos must not be silently ignored");
}

#[test]
fn cycle_covers_the_calendar_month_in_utc() {
    let now: Timestamp = "2026-08-29T06:30:00Z".parse().unwrap();
    let c = Cycle::containing(now);
    assert_eq!((c.year, c.month), (2026, 8));
    assert_eq!(c.label(), "August 2026");
    assert_eq!(c.resets_in_human(now), "2d 17h");
}

#[test]
fn elapsed_fraction_is_zero_at_the_start_and_one_at_the_end() {
    let start: Timestamp = "2026-08-01T00:00:00Z".parse().unwrap();
    let c = Cycle::containing(start);
    assert!(c.elapsed_fraction(start) < 0.001);
    let end: Timestamp = "2026-08-31T23:59:59Z".parse().unwrap();
    assert!(c.elapsed_fraction(end) > 0.999);
    let mid: Timestamp = "2026-08-16T12:00:00Z".parse().unwrap();
    assert!((c.elapsed_fraction(mid) - 0.5).abs() < 0.02);
}

#[test]
fn projection_extrapolates_spend_to_month_end() {
    let c = Cycle::containing("2026-08-01T00:00:00Z".parse().unwrap());
    // Exactly half the month gone, $100 spent -> $200 projected.
    let mid: Timestamp = "2026-08-16T12:00:00Z".parse().unwrap();
    let proj = c.project(Usd::from_f64(100.0), mid);
    assert!((proj.as_f64() - 200.0).abs() < 5.0, "got {proj}");
}

#[test]
fn projection_on_the_first_minute_does_not_explode() {
    let c = Cycle::containing("2026-08-01T00:00:00Z".parse().unwrap());
    let t: Timestamp = "2026-08-01T00:00:30Z".parse().unwrap();
    let proj = c.project(Usd::from_f64(5.0), t);
    assert!(proj.as_f64().is_finite());
    assert!(proj >= Usd::from_f64(5.0), "projection must not be below actual spend");
}

#[test]
fn february_and_month_lengths_are_handled() {
    let c = Cycle::containing("2028-02-10T00:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 29, "2028 is a leap year");
    let c = Cycle::containing("2026-02-10T00:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 28);
    let c = Cycle::containing("2026-12-31T23:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 31);
    assert_eq!(c.resets_in_human("2026-12-31T23:00:00Z".parse().unwrap()), "1h 0m");
}
