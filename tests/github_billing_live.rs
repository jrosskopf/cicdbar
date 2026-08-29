//! Tests against the real GitHub billing API using the real gh CLI token.
//! No mocks, no fixtures: if these pass, the widget's numbers are real.

use cicdbar::money::Usd;
use cicdbar::providers::github_billing;
use cicdbar::token::TokenSource;
use cicdbar::http::Http;

fn http() -> Http {
    let token = TokenSource::GhCli.resolve().expect("gh cli token");
    Http::new(token).expect("http client")
}

#[test]
fn reads_the_real_gh_cli_token() {
    let t = TokenSource::GhCli.resolve().expect("token");
    assert!(t.starts_with("gho_") || t.starts_with("ghp_") || t.starts_with("github_pat_"),
            "unexpected token shape");
}

#[test]
fn fetches_current_month_usage_with_per_repo_granularity() {
    let usage = github_billing::fetch(&http(), "DataZooDE", 2026, 8).expect("fetch");
    assert!(usage.len() > 500, "expected per-day detail rows, got {}", usage.len());
    let repos: std::collections::BTreeSet<_> =
        usage.iter().filter_map(|r| r.repository_name.as_deref()).collect();
    assert!(repos.len() > 5, "expected many repos, got {:?}", repos);
    assert!(usage.iter().all(|r| r.product == "actions"));
}

#[test]
fn detail_rows_agree_with_the_monthly_rollup_on_compute_skus() {
    // The two endpoints agree on compute and disagree on storage, because
    // only the detail applies the included allowance. This is what justifies
    // sourcing every figure from the detail; if GitHub changes it, we want to
    // hear about it here.
    //
    // Note the two calls are not atomic: CI accrues spend between them, so
    // compute is compared with a tolerance rather than for exact equality.
    // The storage gap is ~$55 and structural, so it is asserted sharply.
    let h = http();
    let rollup = github_billing::fetch_rollup(&h, "DataZooDE").expect("rollup");
    let detail = github_billing::fetch(&h, "DataZooDE", 2026, 8).expect("detail");

    let sum = |rows: &[github_billing::UsageItem], storage: bool| -> Usd {
        rows.iter()
            .filter(|r| r.is_storage() == storage && r.date.starts_with("2026-08"))
            .map(|r| Usd::from_f64(r.net_amount))
            .sum()
    };

    let (d_compute, r_compute) = (sum(&detail, false), sum(&rollup, false));
    let drift = (d_compute.as_f64() - r_compute.as_f64()).abs();
    assert!(
        drift < 10.0,
        "compute should agree between endpoints; detail {d_compute} vs rollup {r_compute}"
    );

    let (d_storage, r_storage) = (sum(&detail, true), sum(&rollup, true));
    assert!(
        r_storage > d_storage + Usd::from_f64(10.0),
        "rollup should report storage without the allowance the detail applies; \
         detail {d_storage} vs rollup {r_storage}"
    );

    // Gross is the same on both sides -- the difference really is the discount.
    let gross = |rows: &[github_billing::UsageItem]| -> Usd {
        rows.iter()
            .filter(|r| r.is_storage() && r.date.starts_with("2026-08"))
            .map(|r| Usd::from_f64(r.gross_amount))
            .sum()
    };
    let gd = (gross(&detail).as_f64() - gross(&rollup).as_f64()).abs();
    assert!(gd < 1.0, "storage gross must match; only the discount differs");
}

#[test]
fn an_org_without_billing_access_is_degraded_not_fatal() {
    // octoopt really does return 403 "No access to billing usage data."
    let err = github_billing::fetch(&http(), "octoopt", 2026, 8).unwrap_err();
    assert!(err.is_access_denied(), "expected access-denied, got {err:?}");
}

#[test]
fn an_org_that_does_not_exist_is_degraded_not_fatal() {
    let err = github_billing::fetch(&http(), "zoitech-internal", 2026, 8).unwrap_err();
    assert!(err.is_not_found(), "expected not-found, got {err:?}");
}

#[test]
fn aggregates_real_usage_into_totals_and_breakdowns() {
    let usage = github_billing::fetch(&http(), "DataZooDE", 2026, 8).expect("fetch");
    let spend = github_billing::aggregate(&usage);

    assert!(spend.net > Usd::zero());
    assert!(spend.gross > spend.net, "gross must exceed net given discounts");
    assert_eq!(spend.gross - spend.discount, spend.net);

    // Breakdowns are ordered by spend, descending.
    let repos = spend.top_repos(5);
    assert!(!repos.is_empty());
    assert!(repos.windows(2).all(|w| w[0].1 >= w[1].1));

    let skus = spend.by_sku();
    assert!(skus.iter().any(|(s, _)| s.contains("macOS")));
}
