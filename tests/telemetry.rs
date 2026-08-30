//! Telemetry tests. No mocks: a real HTTP server on loopback stands in for
//! PostHog, and every assertion is made on the actual JSON that arrived on
//! the socket.

use cicdbar::telemetry::{Telemetry, Value};
use std::sync::mpsc;

/// A real server that records the bodies it receives.
fn capture_server() -> (String, mpsc::Receiver<String>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
            let _ = tx.send(body);
            let _ = req.respond(tiny_http::Response::from_string("{\"status\":1}"));
        }
    });
    (url, rx)
}

fn recv(rx: &mpsc::Receiver<String>) -> serde_json::Value {
    let raw = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no telemetry arrived");
    serde_json::from_str(&raw).expect("payload must be json")
}

fn telemetry(url: &str) -> Telemetry {
    Telemetry::for_test("cicdbar", "9.9.9", url).with_features(&["spend_shown"])
}

#[test]
fn every_event_carries_the_schema_2_envelope() {
    let (url, rx) = capture_server();
    let t = telemetry(&url);
    t.capture(
        "feature_used",
        vec![("feature", Value::from("spend_shown"))],
    );
    t.flush();

    let body = recv(&rx);
    assert_eq!(
        body["api_key"].as_str().unwrap_or("")[..4].to_string(),
        "phc_"
    );
    let ev = &body["batch"][0];
    assert_eq!(ev["event"], "feature_used");
    let p = &ev["properties"];

    // The envelope the shared C++ library emits. If this list drifts from
    // docs/TELEMETRY-SCHEMA.md, the products stop being comparable.
    assert_eq!(p["product"], "cicdbar");
    assert_eq!(p["product_version"], "9.9.9");
    assert_eq!(p["product_edition"], "oss");
    assert_eq!(p["telemetry_schema"], 2);
    assert!(p["telemetry_schema"].is_number(), "schema must be a number");
    assert!(["linux", "macos", "windows"].contains(&p["os"].as_str().unwrap()));
    assert!(["amd64", "arm64"].contains(&p["arch"].as_str().unwrap()));
    assert!(p["platform"].is_string());
    assert!(p["is_ci"].is_boolean());
    assert!(p["is_container"].is_boolean());
    assert!(p["$session_id"].is_string());
    assert!(["machine_id", "mac", "ephemeral"].contains(&p["identity_source"].as_str().unwrap()));
    assert!(ev["distinct_id"].is_string());
    assert!(!ev["distinct_id"].as_str().unwrap().is_empty());
}

#[test]
fn an_ephemeral_identity_never_creates_a_person() {
    let (url, rx) = capture_server();
    let t = Telemetry::for_test_ephemeral("cicdbar", "9.9.9", &url);
    t.capture("cli_started", vec![]);
    t.flush();
    let p = recv(&rx)["batch"][0]["properties"].clone();
    assert_eq!(p["identity_source"], "ephemeral");
    assert_eq!(p["$process_person_profile"], false);
}

#[test]
fn the_machine_id_is_hashed_not_sent_raw() {
    let (url, rx) = capture_server();
    let t = telemetry(&url);
    t.capture("cli_started", vec![]);
    t.flush();
    let id = recv(&rx)["batch"][0]["distinct_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id.len(), 64, "expected a sha256 hex digest");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    if let Ok(raw) = std::fs::read_to_string("/etc/machine-id") {
        assert_ne!(id, raw.trim(), "the raw machine id must never be sent");
    }
}

// ---- opt-out: nothing may leave the machine ----

fn assert_silent(env_key: &str, env_val: &str) {
    let (url, rx) = capture_server();
    std::env::set_var(env_key, env_val);
    let t = Telemetry::for_test("cicdbar", "9.9.9", &url);
    t.capture("cli_started", vec![]);
    t.flush();
    std::env::remove_var(env_key);
    assert!(
        !t.is_enabled(),
        "{env_key}={env_val} must disable telemetry"
    );
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(400))
            .is_err(),
        "{env_key}={env_val} still sent something"
    );
}

#[test]
fn datazoo_kill_switch_is_honoured() {
    assert_silent("DATAZOO_DISABLE_TELEMETRY", "1");
}

#[test]
fn do_not_track_is_honoured() {
    assert_silent("DO_NOT_TRACK", "1");
}

#[test]
fn the_product_local_env_var_is_honoured() {
    assert_silent("CICDBAR_NO_TELEMETRY", "1");
}

#[test]
fn truthy_spellings_all_disable() {
    for v in ["1", "true", "yes", "TRUE", "Yes"] {
        assert_silent("DATAZOO_DISABLE_TELEMETRY", v);
    }
}

#[test]
fn a_disabled_telemetry_sends_nothing_even_when_asked_repeatedly() {
    let (url, rx) = capture_server();
    let t = Telemetry::disabled();
    for _ in 0..5 {
        t.capture("cli_started", vec![]);
    }
    t.flush();
    let _ = url;
    assert!(rx
        .recv_timeout(std::time::Duration::from_millis(400))
        .is_err());
}

// ---- privacy ----

#[test]
fn no_identifier_we_handle_can_reach_a_payload() {
    // Adversarial values drawn from what cicdbar actually touches.
    let secrets = [
        "DataZooDE",
        "heron",
        "fix/worker-binary-race",
        "gho_supersecrettoken",
        "remember_web_deadbeef",
        "/home/jr/.config/cicdbar/blacksmith-session",
        "jrosskopf",
    ];
    let (url, rx) = capture_server();
    let t = telemetry(&url);
    // Even if a caller tries, the enum-only API must not carry these through.
    for s in secrets {
        t.capture("feature_used", vec![("feature", Value::from(s))]);
    }
    t.flush();
    let mut seen = String::new();
    while let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(300)) {
        seen.push_str(&b);
    }
    for s in secrets {
        assert!(!seen.contains(s), "{s:?} reached a telemetry payload");
    }
}

#[test]
fn no_currency_amount_can_reach_a_payload() {
    // Spend is commercially sensitive; telemetry never learns amounts.
    let (url, rx) = capture_server();
    let t = telemetry(&url);
    t.capture(
        "feature_used",
        vec![
            ("feature", Value::from("spend_shown")),
            ("total_usd", Value::from("$291.47")),
            ("budget", Value::from(400.0)),
        ],
    );
    t.flush();
    let mut seen = String::new();
    while let Ok(b) = rx.recv_timeout(std::time::Duration::from_millis(300)) {
        seen.push_str(&b);
    }
    assert!(
        !seen.contains("291"),
        "a dollar amount reached telemetry: {seen}"
    );
    assert!(!seen.contains('$') || !seen.contains("291.47"));
    assert!(!seen.contains("400"), "a budget reached telemetry");
}

#[test]
fn strings_are_clamped_as_a_backstop() {
    let (url, rx) = capture_server();
    let t = telemetry(&url);
    t.capture(
        "feature_used",
        vec![("feature", Value::from("x".repeat(4096)))],
    );
    t.flush();
    let p = recv(&rx)["batch"][0]["properties"].clone();
    if let Some(f) = p["feature"].as_str() {
        assert!(f.len() <= 512, "expected a 512-byte clamp, got {}", f.len());
    }
}

// ---- never harm the host program ----

#[test]
fn an_unreachable_endpoint_neither_fails_nor_hangs() {
    let t = Telemetry::for_test("cicdbar", "9.9.9", "http://127.0.0.1:1");
    let start = std::time::Instant::now();
    t.capture("cli_started", vec![]);
    t.flush();
    assert!(
        start.elapsed().as_secs_f64() < 3.0,
        "telemetry blocked the host for {:?}",
        start.elapsed()
    );
}

#[test]
fn the_daily_rollup_actually_survives_the_allow_list() {
    // The allow-list is what makes the privacy promise true, but it must not
    // silently swallow the one event this product exists to send.
    use cicdbar::telemetry::rollup::{self, RollupState};
    let (url, rx) = capture_server();
    let t = telemetry(&url);

    let mut s = RollupState {
        orgs: 2,
        repos: 15,
        blacksmith_enabled: true,
        ..Default::default()
    };
    rollup::record_tick(&mut s, 1_800_000_000);
    let done = rollup::take_for_flush(&mut s, 1_800_090_000);

    t.capture("feature_used", rollup::properties(&done));
    t.flush();

    let p = recv(&rx)["batch"][0]["properties"].clone();
    assert_eq!(p["feature"], "spend_shown");
    assert_eq!(
        p["orgs_bucket"], "2-5",
        "bucket was dropped by the allow-list"
    );
    assert_eq!(p["repos_bucket"], "6-20");
    assert_eq!(p["ticks_bucket"], "1");
    assert_eq!(p["blacksmith_enabled"], true);
    assert_eq!(p["install_kind"], "waybar");
}
