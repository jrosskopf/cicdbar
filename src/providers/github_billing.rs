//! GitHub Actions billing.
//!
//! Two endpoints, both real and both needed:
//!
//! * `?year=&month=` returns per-day, per-repo, per-SKU detail (~1600 rows for
//!   DataZooDE in a month). This is our source of truth: it carries the repo
//!   breakdown and it applies the included allowances.
//! * unfiltered returns a monthly rollup. It agrees with the detail exactly on
//!   compute SKUs, but reports storage with **no** discount applied.
//!
//! The two disagree on storage, and **the rollup is the one that matches the
//! invoice**. Checked against a real July 2026 bill: GitHub charged $210.63
//! with $45.99 of Actions storage, exactly the rollup's figure; the detail
//! rows claimed $43.08 of storage discount that was never applied. Sourcing
//! storage from the detail understated spend by ~$55 a month.
//!
//! So `month_spend` takes compute from the detail (which carries the per-repo
//! breakdown the rollup lacks) and storage from the rollup.

use crate::http::{ApiError, Http};
use crate::money::Usd;
use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageItem {
    pub date: String,
    pub product: String,
    pub sku: String,
    pub quantity: f64,
    pub unit_type: String,
    pub price_per_unit: f64,
    pub gross_amount: f64,
    pub discount_amount: f64,
    pub net_amount: f64,
    pub organization_name: Option<String>,
    pub repository_name: Option<String>,
}

impl UsageItem {
    pub fn is_storage(&self) -> bool {
        self.sku.to_ascii_lowercase().contains("storage")
    }
}

#[derive(serde::Deserialize)]
struct UsageResponse {
    #[serde(rename = "usageItems")]
    usage_items: Vec<UsageItem>,
}

pub fn fetch(http: &Http, org: &str, year: i16, month: u8) -> Result<Vec<UsageItem>, ApiError> {
    let path = format!("/organizations/{org}/settings/billing/usage?year={year}&month={month}");
    let r: UsageResponse = http.get_json(&path)?;
    Ok(r.usage_items)
}

pub fn fetch_rollup(http: &Http, org: &str) -> Result<Vec<UsageItem>, ApiError> {
    let path = format!("/organizations/{org}/settings/billing/usage");
    let r: UsageResponse = http.get_json(&path)?;
    Ok(r.usage_items)
}

#[derive(Debug, Clone, Default)]
pub struct Spend {
    pub net: Usd,
    pub gross: Usd,
    pub discount: Usd,
    pub per_repo: BTreeMap<String, Usd>,
    pub per_sku: BTreeMap<String, Usd>,
    pub per_org: BTreeMap<String, Usd>,
}

impl Spend {
    pub fn merge(&mut self, other: &Spend) {
        self.net += other.net;
        self.gross += other.gross;
        self.discount += other.discount;
        for (k, v) in &other.per_repo {
            *self.per_repo.entry(k.clone()).or_default() += *v;
        }
        for (k, v) in &other.per_sku {
            *self.per_sku.entry(k.clone()).or_default() += *v;
        }
        for (k, v) in &other.per_org {
            *self.per_org.entry(k.clone()).or_default() += *v;
        }
    }

    fn sorted(map: &BTreeMap<String, Usd>) -> Vec<(String, Usd)> {
        let mut v: Vec<_> = map.iter().map(|(k, x)| (k.clone(), *x)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    pub fn top_repos(&self, n: usize) -> Vec<(String, Usd)> {
        Self::sorted(&self.per_repo).into_iter().take(n).collect()
    }

    pub fn by_sku(&self) -> Vec<(String, Usd)> {
        Self::sorted(&self.per_sku)
    }

    pub fn by_org(&self) -> Vec<(String, Usd)> {
        Self::sorted(&self.per_org)
    }
}

/// One month's spend as GitHub will actually invoice it.
///
/// Compute comes from the per-day detail, which is the only source with repo
/// granularity and which agrees with the rollup to the cent. Storage comes
/// from the monthly rollup, because the detail's storage discount is not
/// honoured on the bill.
pub fn month_spend(http: &Http, org: &str, year: i16, month: u8) -> Result<Spend, ApiError> {
    let detail = fetch(http, org, year, month)?;
    let rollup = fetch_rollup(http, org)?;
    Ok(combine(&detail, &rollup, year, month))
}

/// Split out from `month_spend` so the combination rule can be exercised
/// without two network round-trips.
pub fn combine(detail: &[UsageItem], rollup: &[UsageItem], year: i16, month: u8) -> Spend {
    let prefix = format!("{year:04}-{month:02}");
    let compute: Vec<UsageItem> = detail.iter().filter(|r| !r.is_storage()).cloned().collect();
    let storage: Vec<UsageItem> = rollup
        .iter()
        .filter(|r| r.is_storage() && r.date.starts_with(&prefix))
        .cloned()
        .collect();

    let mut spend = aggregate(&compute);
    let storage_spend = aggregate(&storage);
    spend.net += storage_spend.net;
    spend.gross += storage_spend.gross;
    spend.discount += storage_spend.discount;
    for (k, v) in &storage_spend.per_sku {
        *spend.per_sku.entry(k.clone()).or_default() += *v;
    }
    for (k, v) in &storage_spend.per_org {
        *spend.per_org.entry(k.clone()).or_default() += *v;
    }
    // Deliberately NOT merged into per_repo: the rollup names a single
    // arbitrary repository per SKU, so attributing storage to it would be
    // inventing a breakdown that does not exist.
    spend
}

pub fn aggregate(items: &[UsageItem]) -> Spend {
    let mut s = Spend::default();
    for it in items {
        let net = Usd::from_f64(it.net_amount);
        let gross = Usd::from_f64(it.gross_amount);
        s.net += net;
        s.gross += gross;
        // Derived, not summed: the API's own per-row floats only satisfy
        // gross - discount = net approximately, and summing all three
        // independently leaves the aggregate self-inconsistent.
        s.discount += gross - net;
        if let Some(r) = &it.repository_name {
            *s.per_repo.entry(r.clone()).or_default() += net;
        }
        *s.per_sku.entry(it.sku.clone()).or_default() += net;
        if let Some(o) = &it.organization_name {
            *s.per_org.entry(o.clone()).or_default() += net;
        }
    }
    s
}
