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
use cicdbar::notify::{Notifier, Urgency};
use cicdbar::providers::blacksmith;
use cicdbar::providers::github_billing::{self, Spend};
use cicdbar::providers::github_runs::{self, CiStatus, RunnerKind};
use cicdbar::render;
use cicdbar::snapshot::Snapshot;
use cicdbar::transitions::{self, NotifyState};
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

    /// Never send desktop notifications on this run.
    #[arg(long)]
    no_notify: bool,
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
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
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
    snap.theme = cfg.theme.clone();
    github_runs::set_rate_table(cfg.blacksmith.rates.clone(), cfg.blacksmith.base_vcpu);

    let token = cfg.github.token_source.resolve()?;
    let cache = Cache::new(Cache::default_dir());
    // Conditional requests: a 304 does not count against the REST rate limit,
    // and most ticks find nothing changed.
    let http = Http::new(token)?.with_etag_store(cache.dir().join("etags"));
    let billing_ttl = if args.no_cache {
        0
    } else {
        cfg.cache.billing_ttl_secs
    };
    let runs_ttl = if args.no_cache {
        0
    } else {
        cfg.cache.runs_ttl_secs
    };

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
            // Both endpoints: compute comes from the detail, storage from the
            // rollup, because only the rollup matches the invoice.
            let detail = github_billing::fetch(&http, org, cycle.year, cycle.month as u8)
                .map_err(|e| e.short())?;
            let rollup = github_billing::fetch_rollup(&http, org).map_err(|e| e.short())?;
            Ok::<_, String>((detail, rollup))
        })
    });
    for (org, fetched) in cfg.github.orgs.iter().zip(billing) {
        match fetched {
            Ok(((detail, rollup), freshness)) => {
                note_stale(&freshness, &mut oldest_stale);
                // Compute from the detail (per-repo), storage from the rollup
                // (what GitHub actually invoices).
                let spend: Spend =
                    github_billing::combine(&detail, &rollup, cycle.year, cycle.month as u8);
                if spend.net > Usd::zero() || !detail.is_empty() {
                    snap.per_org.push((org.clone(), spend.net));
                }
                merged.merge(&spend);
            }
            Err(reason) => snap.notes.push(format!("{org}: {reason}")),
        }
    }
    snap.per_org
        .sort_by_key(|(_, amount)| std::cmp::Reverse(*amount));
    snap.github = merged;

    // ---- Runs, per org ---- (orgs fetched concurrently)
    let mut ci = CiStatus::default();
    let mut runs_were_stale = false;
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
                if freshness.is_stale() {
                    runs_were_stale = true;
                }
                note_stale(&freshness, &mut oldest_stale);
                ci.all_runs.extend(st.all_runs);
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
                    dash.projected(&org)
                        .map(|p| p.amount)
                        .map_err(|e| e.to_string())
                }) {
                    Ok((amount, freshness)) => {
                        note_stale(&freshness, &mut oldest_stale);
                        snap.blacksmith = Some(amount);
                        snap.blacksmith_is_estimate = false;
                    }
                    Err(e) => snap.notes.push(format!("blacksmith: {e}")),
                }
                if let Ok((usage, _)) =
                    cache.get_or_refresh(&format!("blacksmith-live-{org}"), runs_ttl, || {
                        dash.core_usage(&org)
                            .map(|u| (u.total_jobs(), u.total_vcpus()))
                            .map_err(|e| e.to_string())
                    })
                {
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
    // ---- Notifications ----
    // Only when the run data is genuinely fresh: a stale cache would replay
    // transitions that did not happen.
    if cfg.notifications.enabled && !args.no_notify && !runs_were_stale {
        if let Err(e) = announce(&cfg, &cache, &ci.all_runs) {
            snap.notes.push(format!("notifications: {e}"));
        }
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

/// Diff this tick's runs against the last, and notify about what changed.
///
/// State lives in the cache because the binary is re-exec'd every 60s. The
/// first ever run seeds silently -- announcing every in-flight run on first
/// launch would be noise.
fn announce(
    cfg: &Config,
    cache: &Cache,
    runs: &[cicdbar::providers::github_runs::RunSummary],
) -> anyhow::Result<()> {
    const KEY: &str = "notify-state";

    let previous: NotifyState = cache.read_raw(KEY).unwrap_or_default();
    let (events, next) = transitions::diff(&previous, runs);
    // Persist before notifying: a crash mid-send must not replay the whole
    // batch on the next tick.
    cache.write_raw(KEY, &next);

    // Which workflows were failing before, so a success can count as a
    // recovery under "failures-and-recoveries".
    let was_failing = |run: &cicdbar::providers::github_runs::RunSummary| -> bool {
        previous
            .runs
            .values()
            .any(|s| s.conclusion.as_deref() == Some("failure"))
            && run.conclusion.as_deref() == Some("success")
    };

    let selected: Vec<_> = events
        .into_iter()
        .filter(|e| transitions::should_notify(&cfg.notifications, e, was_failing(e.run())))
        .collect();
    if selected.is_empty() {
        return Ok(());
    }

    let notifier = Notifier::connect()?;

    // A single push fans out to many workflows at once, so a tick can carry
    // a dozen events. Past the cap, one line beats twelve.
    if selected.len() > cfg.notifications.max_per_tick {
        let failures = selected.iter().filter(|e| e.is_failure()).count();
        let summary = format!("{} CI events", selected.len());
        let body = if failures > 0 {
            format!("{failures} failed")
        } else {
            "none failed".to_string()
        };
        let urgency = if failures > 0 {
            Urgency::Critical
        } else {
            Urgency::Normal
        };
        notifier.send(None, &summary, &body, urgency, "cicdbar-summary")?;
        return Ok(());
    }

    let mut state = next;
    for event in &selected {
        let (summary, body, urgency) = transitions::render(event);
        let replaces = match event {
            transitions::Event::Finished {
                previous_notif_id, ..
            } => *previous_notif_id,
            transitions::Event::Started(_) => None,
        };
        match notifier.send(replaces, &summary, &body, urgency, "cicdbar") {
            Ok(id) if id > 0 => {
                // Remember it so the finish can replace the start in place.
                if let Some(seen) = state.runs.get_mut(&event.run().id) {
                    seen.notif_id = Some(id);
                }
            }
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    }
    cache.write_raw(KEY, &state);
    Ok(())
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
