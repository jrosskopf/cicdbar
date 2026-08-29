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
    std::fs::write(
        &path,
        r#"
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
"#,
    )
    .unwrap();

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
    assert!(
        Config::load(&path).is_err(),
        "typos must not be silently ignored"
    );
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
    assert!(
        proj >= Usd::from_f64(5.0),
        "projection must not be below actual spend"
    );
}

#[test]
fn february_and_month_lengths_are_handled() {
    let c = Cycle::containing("2028-02-10T00:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 29, "2028 is a leap year");
    let c = Cycle::containing("2026-02-10T00:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 28);
    let c = Cycle::containing("2026-12-31T23:00:00Z".parse().unwrap());
    assert_eq!(c.days_in_month(), 31);
    assert_eq!(
        c.resets_in_human("2026-12-31T23:00:00Z".parse().unwrap()),
        "1h 0m"
    );
}

#[test]
fn the_theme_is_configurable_and_defaults_to_one_dark() {
    let cfg = Config::load(&tmpdir().join("nope.toml")).expect("defaults");
    assert_eq!(cfg.theme.ok, "#98c379", "One Dark green");
    assert_eq!(cfg.theme.critical, "#e06c75");

    let dir = tmpdir();
    let path = dir.join("theme.toml");
    std::fs::write(&path, "[theme]\nok = \"#00ff00\"\ncritical = \"#ff0000\"\n").unwrap();
    let cfg = Config::load(&path).expect("load");
    assert_eq!(cfg.theme.ok, "#00ff00");
    assert_eq!(cfg.theme.critical, "#ff0000");
    // Unspecified colours keep their defaults.
    assert_eq!(cfg.theme.warning, "#d19a66");
}

#[test]
fn blacksmith_rates_are_configurable_and_default_to_published_prices() {
    let cfg = Config::load(&tmpdir().join("nope.toml")).expect("defaults");
    assert_eq!(cfg.blacksmith.rates.get("ubuntu"), Some(&0.004));
    assert_eq!(cfg.blacksmith.rates.get("macos"), Some(&0.08));
    assert_eq!(cfg.blacksmith.base_vcpu, 2.0);

    let dir = tmpdir();
    let path = dir.join("rates.toml");
    std::fs::write(&path, "[blacksmith.rates]\nubuntu = 0.003\n").unwrap();
    let cfg = Config::load(&path).expect("load");
    assert_eq!(
        cfg.blacksmith.rates.get("ubuntu"),
        Some(&0.003),
        "a negotiated rate must override the list price"
    );
}

#[test]
fn the_example_config_in_the_repo_actually_parses() {
    // A shipped example that does not load is worse than none.
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
    let cfg = Config::load(&example).expect("config.example.toml must parse");
    assert!(!cfg.github.orgs.is_empty());
}

#[test]
fn the_example_config_names_no_real_private_org() {
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.toml");
    let raw = std::fs::read_to_string(example).unwrap();
    for private in ["DataZooDE", "anofox", "octoopt", "sparrowbi", "zoitech"] {
        assert!(
            !raw.contains(private),
            "example config still names {private}"
        );
    }
}

#[test]
fn notification_defaults_are_loud_but_configurable() {
    let cfg = Config::load(&tmpdir().join("nope.toml")).expect("defaults");
    assert!(cfg.notifications.enabled);
    assert!(
        cfg.notifications.on_start,
        "start notifications on by default"
    );
    assert_eq!(cfg.notifications.on_finish, "all");
    assert_eq!(cfg.notifications.max_per_tick, 8);

    let dir = tmpdir();
    let path = dir.join("quiet.toml");
    std::fs::write(
        &path,
        "[notifications]\non_start = false\non_finish = \"failures\"\n",
    )
    .unwrap();
    let cfg = Config::load(&path).expect("load");
    assert!(!cfg.notifications.on_start);
    assert_eq!(cfg.notifications.on_finish, "failures");
    // Untouched keys keep their defaults.
    assert_eq!(cfg.notifications.max_per_tick, 8);
    assert!(cfg.notifications.enabled);
}
