//! Blacksmith spend. Until the dashboard API is authorised, month-to-date
//! spend is derived from the jobs GitHub reports on blacksmith-* runners.
//! Tested against the real repo that actually uses them.

use cicdbar::http::Http;
use cicdbar::money::Usd;
use cicdbar::providers::blacksmith;
use cicdbar::providers::github_runs::RunnerKind;
use cicdbar::token::TokenSource;

fn http() -> Http {
    Http::new(TokenSource::GhCli.resolve().expect("token")).expect("client")
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn finds_the_real_repo_using_blacksmith_runners() {
    // DataZooDE/datazoo-agent-template runs blacksmith-4vcpu-ubuntu-2404.
    let usage =
        blacksmith::repo_month_usage(&http(), "DataZooDE", "datazoo-agent-template", 2026, 8)
            .expect("usage");
    assert!(usage.seconds > 0, "expected blacksmith minutes in August");
    assert!(usage.cost > Usd::zero());
    assert!(usage.by_runner.keys().any(|k| k.starts_with("blacksmith")));
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn a_month_before_blacksmith_was_adopted_costs_nothing() {
    // Which repos use Blacksmith changes over time (erpl-proto adopted it
    // during August 2026), so this pins a period instead of a repo.
    let usage =
        blacksmith::repo_month_usage(&http(), "DataZooDE", "erpl-proto", 2025, 1).expect("usage");
    assert_eq!(usage.cost, Usd::zero());
    assert_eq!(usage.seconds, 0);
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn only_blacksmith_jobs_are_ever_priced() {
    // erpl-proto runs a mix of blacksmith, ubuntu-latest, windows and macos
    // jobs. Only the blacksmith ones may contribute.
    let usage =
        blacksmith::repo_month_usage(&http(), "DataZooDE", "erpl-proto", 2026, 8).expect("usage");
    assert!(usage.seconds > 0, "erpl-proto uses blacksmith runners");
    assert!(
        usage.by_runner.keys().all(|k| k.starts_with("blacksmith")),
        "github-hosted runners must never be priced here: {:?}",
        usage.by_runner
    );
    assert_eq!(usage.cost > Usd::zero(), usage.seconds > 0);
}

#[test]
fn cost_is_minutes_times_the_published_rate() {
    let kind = RunnerKind::Blacksmith {
        vcpu: 4,
        family: "ubuntu".into(),
    };
    assert_eq!(blacksmith::cost_for(&kind, 3600), Usd::from_f64(0.48));
}

#[test]
fn free_minutes_are_deducted_before_charging() {
    let charged = blacksmith::apply_free_minutes(Usd::from_f64(10.0), 1_000 * 60, 3_000);
    assert_eq!(charged, Usd::zero(), "still inside the free allowance");
    let charged = blacksmith::apply_free_minutes(Usd::from_f64(10.0), 6_000 * 60, 3_000);
    assert_eq!(charged, Usd::from_f64(5.0), "half the minutes were free");
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn discovers_blacksmith_repos_across_the_org_from_real_data() {
    // Asserts the property, not which repos happen to be top of the
    // recently-pushed list: that ordering changes by the hour, and an
    // earlier version of this test pinned a repo that simply moved down.
    let h = http();
    let repos = blacksmith::discover_repos(&h, "DataZooDE", 7, 8).expect("discover");
    assert!(
        !repos.is_empty(),
        "this org does use blacksmith runners somewhere"
    );

    // Everything returned must genuinely have blacksmith-priced minutes.
    for repo in &repos {
        let usage = blacksmith::repo_month_usage(&h, "DataZooDE", repo, 2026, 8)
            .unwrap_or_else(|e| panic!("usage for {repo}: {e:?}"));
        assert!(
            usage.by_runner.keys().all(|k| k.starts_with("blacksmith")),
            "{repo} was reported as a blacksmith repo but priced {:?}",
            usage.by_runner
        );
    }
}

#[test]
fn the_dashboard_provider_reports_unauthorised_rather_than_guessing() {
    let err = blacksmith::dashboard_spend(None, "DataZooDE").unwrap_err();
    assert!(err.to_string().contains("not configured"));
}

// ---- Dashboard API, against the real dashboardbackend.blacksmith.sh ----

fn cookie_file() -> std::path::PathBuf {
    dirs_config().join("cicdbar").join("blacksmith-session")
}

fn dirs_config() -> std::path::PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".config")
        })
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn reads_projected_spend_from_the_real_dashboard_api() {
    let file = cookie_file();
    if !file.exists() {
        panic!(
            "no blacksmith session at {}; capture one first",
            file.display()
        );
    }
    let client = blacksmith::Dashboard::from_cookie_file(&file).expect("client");
    let p = client.projected("DataZooDE").expect("projected");
    // The org is actively running blacksmith jobs, so this is a real figure.
    assert!(p.amount >= Usd::zero());
    assert!(p.amount < Usd::from_f64(100_000.0), "sanity");
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn reads_live_runner_concurrency_from_the_real_dashboard_api() {
    let client = blacksmith::Dashboard::from_cookie_file(&cookie_file()).expect("client");
    let u = client.core_usage("DataZooDE").expect("core usage");
    assert!(u.total_vcpus() >= 0);
    assert!(u.total_jobs() >= 0);
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn a_rotated_session_cookie_is_persisted_so_the_next_run_still_works() {
    // Laravel rotates blacksmith_session on every response. If we did not
    // write the new value back, the widget would authenticate exactly once.
    let file = cookie_file();
    let before = std::fs::read_to_string(&file).unwrap();
    let client = blacksmith::Dashboard::from_cookie_file(&file).expect("client");
    client.projected("DataZooDE").expect("first call");
    let after = std::fs::read_to_string(&file).unwrap();
    assert!(
        after.contains("blacksmith_session="),
        "session cookie retained"
    );
    assert_ne!(before, after, "rotated cookie must be written back");

    // And a second, independent client must still authenticate.
    let client2 = blacksmith::Dashboard::from_cookie_file(&file).expect("client2");
    client2
        .projected("DataZooDE")
        .expect("second call with the rolled cookie");
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn an_expired_session_is_reported_as_such_not_as_zero_spend() {
    let client =
        blacksmith::Dashboard::with_cookies("blacksmith_session=definitely-not-valid".into());
    let err = client.projected("DataZooDE").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("session"),
        "expected a session error, got {err}"
    );
}

#[test]
#[ignore = "hits the live GitHub and Blacksmith APIs; needs a gh token and a captured Blacksmith session"]
fn a_stale_session_cookie_is_recovered_via_the_durable_one() {
    // The server rotates blacksmith_session on every response, so a
    // concurrent caller can leave ours behind. That must self-heal against
    // the real API rather than surfacing as an error.
    let raw = std::fs::read_to_string(cookie_file()).unwrap();
    let durable: Vec<&str> = raw
        .split(';')
        .map(|c| c.trim())
        .filter(|c| c.starts_with("remember_web"))
        .collect();
    assert!(
        !durable.is_empty(),
        "session file must carry remember_web_*"
    );

    let cookies = format!("{}; blacksmith_session=stale-and-wrong", durable.join("; "));
    let client = blacksmith::Dashboard::with_cookies(cookies);
    let p = client
        .projected("DataZooDE")
        .expect("must recover via remember_web");
    assert!(p.amount >= Usd::zero());
}

#[test]
fn an_idle_org_reporting_null_usage_is_not_an_error() {
    // The dashboard sends {"current_usage":null,...} when nothing is running.
    // That is the common case at night, and must not break the widget.
    let parsed: blacksmith::CoreUsage =
        serde_json::from_str(r#"{"current_usage":null,"timestamp":"2026-08-29T06:35:57+00:00"}"#)
            .expect("null usage must parse");
    assert_eq!(parsed.total_jobs(), 0);
    assert_eq!(parsed.total_vcpus(), 0);
    assert!(parsed.active().is_empty());

    let p: blacksmith::Projected =
        serde_json::from_str(r#"{"amount_cents":140,"charges":null}"#).expect("null charges");
    assert_eq!(p.amount, Usd::from_f64(1.40));
}

#[test]
fn a_failed_auth_response_must_not_destroy_a_working_credential() {
    // The durable remember_web cookie is what re-establishes a session. If a
    // 401 response is allowed to rewrite the jar, one bad response can leave
    // the install permanently unable to authenticate -- which is exactly what
    // happened in practice on 2026-08-30.
    let dir = std::env::temp_dir().join(format!("cicdbar-jar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("session");
    std::fs::write(
        &file,
        "remember_web_abc=durable-value; blacksmith_session=stale-value",
    )
    .unwrap();

    // A real server that rejects and tries to clear the cookies, as a logout
    // or an invalidated session would.
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.server_addr());
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let resp = tiny_http::Response::from_string("{\"message\":\"Unauthenticated.\"}")
                .with_status_code(401)
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"Set-Cookie"[..],
                        &b"remember_web_abc=; Max-Age=0; expires=Thu, 01 Jan 1970 00:00:00 GMT"[..],
                    )
                    .unwrap(),
                );
            let _ = req.respond(resp);
        }
    });

    let client = blacksmith::Dashboard::from_cookie_file(&file)
        .expect("client")
        .with_base(base);
    let _ = client.projected("acme");

    let after = std::fs::read_to_string(&file).unwrap();
    assert!(
        after.contains("remember_web_abc=durable-value"),
        "a 401 destroyed the durable credential; jar is now: {after}"
    );
}

// ---- what you actually pay, not what you used ----

#[test]
fn credits_are_subtracted_from_projected_usage() {
    use cicdbar::providers::blacksmith::{net_due, Credits};
    let gross = Usd::from_f64(10.98);

    // A coupon larger than usage means nothing is due.
    let c = Credits {
        wallet_credit: Usd::zero(),
        fixed_coupon: Usd::from_f64(12.00),
        discount_percent: None,
    };
    assert_eq!(net_due(gross, &c), Usd::zero());

    // A partial coupon leaves the remainder.
    let c = Credits {
        wallet_credit: Usd::zero(),
        fixed_coupon: Usd::from_f64(4.00),
        discount_percent: None,
    };
    assert_eq!(net_due(gross, &c), Usd::from_f64(6.98));

    // Wallet credit stacks with the coupon.
    let c = Credits {
        wallet_credit: Usd::from_f64(2.00),
        fixed_coupon: Usd::from_f64(4.00),
        discount_percent: None,
    };
    assert_eq!(net_due(gross, &c), Usd::from_f64(4.98));

    // A percentage discount applies to usage before credits.
    let c = Credits {
        wallet_credit: Usd::zero(),
        fixed_coupon: Usd::zero(),
        discount_percent: Some(50.0),
    };
    assert_eq!(net_due(gross, &c), Usd::from_f64(5.49));

    // Never negative, however generous the credits.
    let c = Credits {
        wallet_credit: Usd::from_f64(999.0),
        fixed_coupon: Usd::from_f64(999.0),
        discount_percent: Some(100.0),
    };
    assert_eq!(net_due(gross, &c), Usd::zero());

    // No credits at all means you pay the usage.
    let c = Credits::default();
    assert_eq!(net_due(gross, &c), gross);
}

#[test]
#[ignore = "talks to the real session D-Bus and notification daemon"]
fn the_real_dashboard_reports_credits() {
    let client = blacksmith::Dashboard::from_cookie_file(&cookie_file()).expect("client");
    let credits = client.credits("DataZooDE").expect("credits");
    let gross = client.projected("DataZooDE").expect("projected").amount;
    let net = blacksmith::net_due(gross, &credits);

    // The invariants, rather than figures that move by the hour.
    assert!(net <= gross, "net {net} must not exceed gross {gross}");
    assert!(net >= Usd::zero(), "net must never be negative");
}
