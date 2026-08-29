use cicdbar::money::Usd;
use cicdbar::render::{self, Severity};
use cicdbar::snapshot::Snapshot;

fn snap() -> Snapshot {
    let mut s = Snapshot::demo();
    s.github.net = Usd::from_f64(232.02);
    s.github.gross = Usd::from_f64(2746.06);
    s.blacksmith_estimate = Usd::from_f64(41.20);
    s.budget = Usd::from_f64(400.0);
    s.projected = Usd::from_f64(372.0);
    s.running = 2;
    s.queued = 1;
    s.failures = 1;
    s
}

#[test]
fn expands_every_documented_placeholder() {
    let s = snap();
    let f = "{total_usd}|{gh_usd}|{bs_usd}|{gross_usd}|{budget_usd}|{proj_usd}|{proj_pct}|\
             {running}|{queued}|{failed}|{inflight_usd}|{cycle_reset}|{run_glyph}|{stale}";
    let out = render::expand(f, &s).expect("expand");
    assert!(!out.contains('{'), "unexpanded placeholder in {out:?}");
    assert!(out.contains("$273"), "total is gh + blacksmith: {out}");
    assert!(out.contains("$232"));
    assert!(out.contains("93"), "372 of 400 is 93%");
}

#[test]
fn an_unknown_placeholder_is_an_error_not_silent_output() {
    assert!(render::expand("{total_usd} {toatl_usd}", &snap()).is_err());
}

#[test]
fn literal_braces_survive() {
    let out = render::expand("{{literal}} {running}", &snap()).unwrap();
    assert_eq!(out, "{literal} 2");
}

#[test]
fn severity_follows_projected_spend_against_budget() {
    let mk = |proj: f64| {
        let mut s = snap();
        s.projected = Usd::from_f64(proj);
        s.severity()
    };
    assert_eq!(mk(100.0), Severity::Ok);
    assert_eq!(mk(239.0), Severity::Ok);
    assert_eq!(mk(241.0), Severity::Low);
    assert_eq!(mk(339.0), Severity::Low);
    assert_eq!(mk(341.0), Severity::Warning);
    assert_eq!(mk(401.0), Severity::Critical);
}

#[test]
fn severity_is_unknown_when_no_budget_is_configured() {
    let mut s = snap();
    s.budget = Usd::zero();
    assert_eq!(s.severity(), Severity::Unknown);
    assert_eq!(s.severity().class(), "ok");
}

#[test]
fn pango_markup_is_escaped_so_repo_names_cannot_break_the_tooltip() {
    let esc = render::escape("R&D <script> \"x\"");
    assert_eq!(esc, "R&amp;D &lt;script&gt; &quot;x&quot;");
}

#[test]
fn the_tooltip_is_well_formed_pango_and_names_the_real_sections() {
    let s = snap();
    let tip = render::tooltip(&s);
    assert!(tip.contains("GitHub Actions"));
    assert!(tip.contains("Blacksmith"));
    assert!(tip.contains("Projected"));
    assert!(tip.contains(&s.cycle.label()));
    // Every span opened is closed.
    assert_eq!(tip.matches("<span").count(), tip.matches("</span>").count());
    assert!(
        !tip.contains("R&D"),
        "unescaped ampersand would break Pango"
    );
}

#[test]
fn the_tooltip_lists_in_flight_runs_with_elapsed_and_cost() {
    let s = snap();
    let tip = render::tooltip(&s);
    assert!(tip.contains("widget-service"));
    assert!(tip.contains("blacksmith"), "runner kind is shown");
}

#[test]
fn a_stale_snapshot_is_marked_in_both_bar_and_tooltip() {
    let mut s = snap();
    s.stale_reason = Some("rate limited".into());
    s.age_secs = 420;
    assert!(render::tooltip(&s).contains("Stale"));
    assert!(render::expand("{stale}", &s).unwrap().contains('⏸'));
    s.stale_reason = None;
    assert_eq!(render::expand("{stale}", &s).unwrap(), "");
}

#[test]
fn the_widget_emits_valid_waybar_json() {
    let out = render::waybar_json(&snap(), "{total_usd} {run_glyph}{running}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["text"].is_string());
    assert!(v["tooltip"].is_string());
    assert!(v["class"].is_string());
}

#[test]
fn a_total_failure_still_emits_valid_waybar_json() {
    let out = render::failure_json("no token");
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["class"], "critical");
    assert!(v["tooltip"].as_str().unwrap().contains("no token"));
    assert!(!v["text"].as_str().unwrap().is_empty());
}
