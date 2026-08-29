//! cicdbar — a waybar widget for CI/CD spend and live job status.
//!
//! Stateless: waybar re-execs it on an interval, all state is the disk cache.
//! It must always print one line of valid waybar JSON and exit 0, whatever
//! went wrong, or the widget vanishes from the bar.

use cicdbar::cache::{Cache, Freshness};
use cicdbar::config::Config;
use cicdbar::cycle::Cycle;
use cicdbar::http::Http;
use cicdbar::money::Usd;
use cicdbar::providers::blacksmith;
use cicdbar::providers::github_billing::{self, Spend, UsageItem};
use cicdbar::providers::github_runs::{self, CiStatus, RunnerKind};
use cicdbar::render;
use cicdbar::snapshot::Snapshot;
use clap::Parser;
use jiff::Timestamp;

const DEFAULT_FORMAT: &str = "{total_usd} · {run_glyph}{running} · {proj_pct}%{stale}";

#[derive(Parser, Debug)]
#[command(version, about = "CI/CD spend and job status for waybar")]
struct Args {
    /// Bar text template. Placeholders: {total_usd} {gh_usd} {bs_usd}
    /// {gross_usd} {budget_usd} {proj_usd} {proj_pct} {cycle_reset}
    /// {running} {queued} {failed} {inflight_usd} {run_glyph} {stale}
    #[arg(long, default_value = DEFAULT_FORMAT)]
    format: String,

    /// Config file (default: $XDG_CONFIG_HOME/cicdbar/config.toml)
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Render fixed sample data without touching the network.
    #[arg(long)]
    demo: bool,

    /// Ignore cached values and refetch everything.
    #[arg(long)]
    no_cache: bool,

    /// Print the tooltip as plain text and exit (for eyeballing in a terminal).
    #[arg(long)]
    tooltip_only: bool,

    /// Report HTTP requests issued and 304s served, to stderr.
    #[arg(long)]
    stats: bool,
}

fn main() {
    let args = Args::parse();
    let out = run(&args).unwrap_or_else(|e| render::failure_json(&e.to_string()));
    if args.tooltip_only {
        // Strip Pango so the tooltip is readable in a terminal.
        let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
        let tip = v["tooltip"].as_str().unwrap_or_default();
        println!("{}", strip_markup(tip));
    } else {
        println!("{out}");
    }
}

fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'")
}

fn run(args: &Args) -> anyhow::Result<String> {
    if args.demo {
        return Ok(render::waybar_json(&Snapshot::demo(), &args.format));
    }

    let cfg_path = args.config.clone().unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path)?;
    let now = Timestamp::now();
    let cycle = Cycle::containing(now);
    let mut snap = Snapshot::new(now);
    snap.budget = cfg.budget_usd;

    let token = cfg.github.token_source.resolve()?;
    let cache = Cache::new(Cache::default_dir());
    // Conditional requests: a 304 does not count against the REST rate limit,
    // and most ticks find nothing changed.
    let http = Http::new(token)?.with_etag_store(cache.dir().join("etags"));
    let billing_ttl = if args.no_cache { 0 } else { cfg.cache.billing_ttl_secs };
    let runs_ttl = if args.no_cache { 0 } else { cfg.cache.runs_ttl_secs };

    let mut oldest_stale: Option<(u64, String)> = None;
    let note_stale = |f: &Freshness, oldest: &mut Option<(u64, String)>| {
        if let Freshness::Stale { age_secs, reason } = f {
            let worse = oldest.as_ref().map(|(a, _)| age_secs > a).unwrap_or(true);
            if worse {
                *oldest = Some((*age_secs, reason.clone()));
            }
        }
    };

    // ---- Billing, per org ---- (orgs fetched concurrently)
    let mut merged = Spend::default();
    let billing: Vec<_> = github_runs::fan_out(&cfg.github.orgs, |org| {
        let key = format!("billing-{org}-{}-{}", cycle.year, cycle.month);
        cache.get_or_refresh(&key, billing_ttl, || {
            github_billing::fetch(&http, org, cycle.year, cycle.month as u8)
                .map_err(|e| e.short())
        })
    });
    for (org, fetched) in cfg.github.orgs.iter().zip(billing) {
        match fetched {
            Ok((items, freshness)) => {
                note_stale(&freshness, &mut oldest_stale);
                let spend: Spend = github_billing::aggregate(&items);
                if spend.net > Usd::zero() || !items.is_empty() {
                    snap.per_org.push((org.clone(), spend.net));
                }
                merged.merge(&spend);
                check_storage_divergence(&http, org, &items, &mut snap, &cache, billing_ttl);
            }
            Err(reason) => snap.notes.push(format!("{org}: {reason}")),
        }
    }
    snap.per_org.sort_by_key(|(_, amount)| std::cmp::Reverse(*amount));
    snap.github = merged;

    // ---- Runs, per org ---- (orgs fetched concurrently)
    let mut ci = CiStatus::default();
    let statuses: Vec<_> = github_runs::fan_out(&cfg.github.orgs, |org| {
        let key = format!("runs-{org}");
        cache.get_or_refresh(&key, runs_ttl, || {
            github_runs::org_status(&http, org, cfg.runs.active_days, cfg.runs.max_repos)
                .map_err(|e| e.short())
        })
    });
    for (org, fetched) in cfg.github.orgs.iter().zip(statuses) {
        match fetched {
            Ok((st, freshness)) => {
                note_stale(&freshness, &mut oldest_stale);
                ci.running += st.running;
                ci.queued += st.queued;
                ci.repos_polled += st.repos_polled;
                ci.in_flight_estimate += st.in_flight_estimate;
                ci.in_flight.extend(st.in_flight);
                ci.failures.extend(st.failures);
                for e in st.errors {
                    snap.notes.push(e);
                }
            }
            Err(reason) => snap.notes.push(format!("{org} runs: {reason}")),
        }
    }
    snap.running = ci.running;
    snap.queued = ci.queued;
    snap.failures = ci.failures.len();
    snap.failure_runs = ci.failures;
    snap.repos_polled = ci.repos_polled;
    snap.in_flight_estimate = ci.in_flight_estimate;
    snap.in_flight = ci.in_flight;
    snap.in_flight
        .sort_by_key(|f| f.run.started_at.clone().unwrap_or_default());

    // ---- Blacksmith ----
    // Prefer the exact figure from the dashboard API; fall back to pricing the
    // jobs GitHub reports on blacksmith-* runners. The fallback is always
    // labelled, so a stale session never reads as "you spent nothing".
    snap.blacksmith_estimate = blacksmith_estimate(&snap);
    snap.blacksmith_is_estimate = true;
    if cfg.blacksmith.enabled {
        let session = cfg
            .blacksmith
            .session_file
            .clone()
            .unwrap_or_else(Config::default_blacksmith_session);
        let org = cfg
            .blacksmith
            .org
            .clone()
            .or_else(|| cfg.github.orgs.first().cloned())
            .unwrap_or_default();
        match blacksmith::Dashboard::from_cookie_file(&session) {
            Ok(dash) => {
                let key = format!("blacksmith-{org}");
                match cache.get_or_refresh(&key, billing_ttl, || {
                    dash.projected(&org).map(|p| p.amount).map_err(|e| e.to_string())
                }) {
                    Ok((amount, freshness)) => {
                        note_stale(&freshness, &mut oldest_stale);
                        snap.blacksmith = Some(amount);
                        snap.blacksmith_is_estimate = false;
                    }
                    Err(e) => snap.notes.push(format!("blacksmith: {e}")),
                }
                if let Ok((usage, _)) = cache.get_or_refresh(
                    &format!("blacksmith-live-{org}"),
                    runs_ttl,
                    || {
                        dash.core_usage(&org)
                            .map(|u| (u.total_jobs(), u.total_vcpus()))
                            .map_err(|e| e.to_string())
                    },
                ) {
                    if usage.0 > 0 || usage.1 > 0 {
                        snap.bs_live = Some(usage);
                    }
                }
            }
            Err(e) => snap.notes.push(format!("blacksmith: {e}")),
        }
    }

    if let Some((age, reason)) = oldest_stale {
        snap.age_secs = age;
        snap.stale_reason = Some(reason);
    }
    snap.recompute_projection();
    if args.stats {
        eprintln!(
            "requests={} not_modified={} repos_polled={}",
            http.request_count(),
            http.not_modified_count(),
            snap.repos_polled
        );
    }
    Ok(render::waybar_json(&snap, &args.format))
}

/// The monthly rollup reports Actions storage with no discount applied while
/// the per-day detail applies the included allowance. Surface the gap rather
/// than silently picking a side.
fn check_storage_divergence(
    http: &Http,
    org: &str,
    detail: &[UsageItem],
    snap: &mut Snapshot,
    cache: &Cache,
    ttl: u64,
) {
    let key = format!("rollup-{org}");
    let Ok((rollup, _)) = cache.get_or_refresh(&key, ttl.max(3600), || {
        github_billing::fetch_rollup(http, org).map_err(|e| e.short())
    }) else {
        return;
    };
    let month_prefix = {
        let c = Cycle::containing(Timestamp::now());
        format!("{:04}-{:02}", c.year, c.month)
    };
    let storage = |rows: &[UsageItem]| -> Usd {
        rows.iter()
            .filter(|r| r.is_storage() && r.date.starts_with(&month_prefix))
            .map(|r| Usd::from_f64(r.net_amount))
            .sum()
    };
    let (d, r) = (storage(detail), storage(&rollup));
    if r > d + Usd::from_f64(1.0) {
        snap.storage_divergence = Some(r - d);
        snap.notes.push(format!(
            "{org}: rollup bills storage at {r} vs {d} after allowance — reconcile with the invoice"
        ));
    }
}

/// Sum the Blacksmith minutes GitHub reports for this month's jobs.
/// Only in-flight work is visible without a full run sweep, so this is a
/// floor, and is always labelled as an estimate.
fn blacksmith_estimate(snap: &Snapshot) -> Usd {
    snap.in_flight
        .iter()
        .filter(|f| matches!(f.runner, RunnerKind::Blacksmith { .. }))
        .filter_map(|f| f.estimate)
        .sum()
}
