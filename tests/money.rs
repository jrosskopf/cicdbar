use cicdbar::money::Usd;

// The billing API prices Linux minutes at $0.006 and quotes amounts like
// 79.977071728. Summing many such rows as f64 drifts; Usd must not.
#[test]
fn summing_many_priced_rows_is_exact() {
    let row = Usd::from_f64(0.006);
    let total: Usd = (0..100_000).map(|_| row).sum();
    assert_eq!(total, Usd::from_f64(600.0));
    assert_eq!(total.to_string(), "$600.00");
}

#[test]
fn parses_the_real_amounts_the_api_returns() {
    let linux = Usd::from_f64(79.977071728);
    let windows = Usd::from_f64(25.339999999999975);
    let macos = Usd::from_f64(122.49199999999996);
    let arm = Usd::from_f64(0.8400000000000034);
    let storage = Usd::from_f64(58.586750011);
    let total = linux + windows + macos + arm + storage;
    // DataZooDE, August 2026 to date.
    assert_eq!(total.to_string(), "$287.24");
}

#[test]
fn formats_with_thousands_separators_and_rounds_half_up() {
    assert_eq!(Usd::from_f64(2746.06).to_string(), "$2,746.06");
    assert_eq!(Usd::from_f64(0.005).to_string(), "$0.01");
    assert_eq!(Usd::from_f64(0.004).to_string(), "$0.00");
    assert_eq!(Usd::zero().to_string(), "$0.00");
}

#[test]
fn compact_form_is_used_for_the_narrow_bar() {
    assert_eq!(Usd::from_f64(287.23).compact(), "$287");
    assert_eq!(Usd::from_f64(2746.06).compact(), "$2.7k");
    assert_eq!(Usd::from_f64(9.5).compact(), "$9.50");
}

#[test]
fn ratio_against_a_budget_survives_a_zero_budget() {
    assert_eq!(Usd::from_f64(200.0).pct_of(Usd::from_f64(400.0)), Some(50.0));
    assert_eq!(Usd::from_f64(200.0).pct_of(Usd::zero()), None);
}
