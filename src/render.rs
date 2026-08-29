//! Waybar output: a one-line `text`, a Pango-markup `tooltip`, and a `class`.
//!
//! Colour follows the *projected* month-end spend against the budget, so a
//! runaway shows on day 8 rather than at the invoice.

use crate::cycle::{elapsed_short, human_duration};
use crate::money::Usd;
use crate::snapshot::Snapshot;

// One Dark, matching the existing claudebar widget.
const FG_DIM: &str = "#5c6370";
const FG_TEXT: &str = "#abb2bf";
const BLUE: &str = "#61afef";
const GREEN: &str = "#98c379";
const YELLOW: &str = "#e5c07b";
const ORANGE: &str = "#d19a66";
const RED: &str = "#e06c75";
const TRACK: &str = "#3e4451";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Unknown,
    Ok,
    Low,
    Warning,
    Critical,
}

impl Severity {
    pub fn class(&self) -> &'static str {
        match self {
            Severity::Unknown | Severity::Ok => "ok",
            Severity::Low => "low",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
    pub fn colour(&self) -> &'static str {
        match self {
            Severity::Unknown => FG_TEXT,
            Severity::Ok => GREEN,
            Severity::Low => YELLOW,
            Severity::Warning => ORANGE,
            Severity::Critical => RED,
        }
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn span(colour: &str, body: &str) -> String {
    format!("<span foreground='{colour}'>{body}</span>")
}

fn bold(colour: &str, body: &str) -> String {
    format!("<span font_weight='bold' foreground='{colour}'>{body}</span>")
}

/// A 20-cell progress bar, as claudebar draws.
fn bar(fraction: f64, colour: &str) -> String {
    const WIDTH: usize = 20;
    let filled = ((fraction.clamp(0.0, 1.0)) * WIDTH as f64).round() as usize;
    format!(
        "{}{}",
        span(colour, &"█".repeat(filled)),
        span(TRACK, &"░".repeat(WIDTH - filled))
    )
}

fn placeholder(name: &str, s: &Snapshot) -> Option<String> {
    let v = match name {
        "total_usd" => s.total().compact(),
        "gh_usd" => s.github.net.compact(),
        "bs_usd" => s.blacksmith_usd().compact(),
        "gross_usd" => s.github.gross.compact(),
        "budget_usd" => s.budget.compact(),
        "proj_usd" => s.projected.compact(),
        "proj_pct" => s
            .projected_pct()
            .map(|p| format!("{p:.0}"))
            .unwrap_or_else(|| "–".into()),
        "cycle_reset" => s.cycle.resets_in_human(s.now),
        "running" => s.running.to_string(),
        "queued" => s.queued.to_string(),
        "failed" => s.failures.to_string(),
        "inflight_usd" => s.in_flight_estimate.compact(),
        "run_glyph" => {
            if s.failures > 0 {
                "✖ ".into()
            } else if s.running > 0 {
                "● ".into()
            } else if s.queued > 0 {
                "◌ ".into()
            } else {
                "✓ ".into()
            }
        }
        "stale" => {
            if s.stale_reason.is_some() {
                " ⏸".into()
            } else {
                String::new()
            }
        }
        _ => return None,
    };
    Some(v)
}

/// Expand a `--format` string. An unknown placeholder is an error rather than
/// silent literal output, so a typo in the waybar config is visible at once.
pub fn expand(fmt: &str, s: &Snapshot) -> anyhow::Result<String> {
    let mut out = String::with_capacity(fmt.len());
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    name.push(c);
                }
                match placeholder(&name, s) {
                    Some(v) => out.push_str(&v),
                    None => anyhow::bail!("unknown placeholder {{{name}}}"),
                }
            }
            _ => out.push(c),
        }
    }
    Ok(out)
}

fn rule() -> String {
    span(FG_DIM, &"─".repeat(44))
}

pub fn tooltip(s: &Snapshot) -> String {
    let sev = s.severity();
    let mut t = String::new();

    t.push_str(&format!(
        " {}\n {}\n\n",
        bold(BLUE, &format!("CI/CD spend — {}", escape(&s.cycle.label()))),
        rule()
    ));

    // GitHub
    t.push_str(&format!(
        " {}   {}\n",
        span(FG_TEXT, "  󰊤  GitHub Actions"),
        bold(FG_TEXT, &s.github.net.to_string())
    ));
    if let Some(f) = s.github.net.pct_of(s.budget) {
        t.push_str(&format!("   {}\n", bar(f / 100.0, sev.colour())));
    }
    t.push_str(&format!(
        " {}\n",
        span(
            FG_DIM,
            &format!(
                "     net of {} gross ({} discounted)",
                escape(&s.github.gross.to_string()),
                escape(&s.github.discount.to_string())
            )
        )
    ));
    for (org, amt) in s.per_org.iter().take(5) {
        t.push_str(&format!(
            " {}\n",
            span(FG_DIM, &format!("     {}  {}", escape(org), escape(&amt.to_string())))
        ));
    }
    let skus: Vec<String> = s
        .github
        .by_sku()
        .iter()
        .take(5)
        .map(|(k, v)| {
            let short = k.replace("Actions ", "");
            format!("{} {}", escape(&short), escape(&v.compact()))
        })
        .collect();
    if !skus.is_empty() {
        t.push_str(&format!(" {}\n", span(FG_DIM, &format!("     {}", skus.join(" · ")))));
    }
    let repos: Vec<String> = s
        .github
        .top_repos(4)
        .iter()
        .map(|(k, v)| format!("{} {}", escape(k), escape(&v.compact())))
        .collect();
    if !repos.is_empty() {
        t.push_str(&format!(" {}\n", span(FG_DIM, &format!("     {}", repos.join(" · ")))));
    }

    // Blacksmith
    let bs_label = if s.blacksmith_is_estimate { "Blacksmith ~est" } else { "Blacksmith" };
    t.push_str(&format!(
        "\n {}   {}\n",
        span(FG_TEXT, &format!("  󰛨  {bs_label}")),
        bold(FG_TEXT, &s.blacksmith_usd().to_string())
    ));

    // Projection
    t.push_str(&format!("\n {}\n", rule()));
    let proj_line = match s.projected_pct() {
        Some(p) => format!(
            "  󰄉  Projected month-end {}  ({:.0}% of {})",
            s.projected.to_string(),
            p,
            s.budget
        ),
        None => format!("  󰄉  Projected month-end {}  (no budget set)", s.projected),
    };
    t.push_str(&format!(" {}\n", bold(sev.colour(), &escape(&proj_line))));
    t.push_str(&format!(
        " {}\n",
        span(FG_DIM, &format!("  󰥔  Cycle resets in {}", s.cycle.resets_in_human(s.now)))
    ));

    // CI status
    t.push_str(&format!("\n {}\n", rule()));
    t.push_str(&format!(
        " {}\n",
        span(
            FG_TEXT,
            &format!(
                "  󰑮  {} running · {} queued · {} failing   {}",
                s.running,
                s.queued,
                s.failures,
                if s.in_flight_estimate > Usd::zero() {
                    format!("(~{} in flight)", s.in_flight_estimate)
                } else {
                    String::new()
                }
            )
        )
    ));
    for f in s.in_flight.iter().take(8) {
        let elapsed = f
            .run
            .started()
            .map(|st| elapsed_short(st, s.now))
            .unwrap_or_else(|| "–".into());
        let cost = f.estimate.map(|c| format!(" · ~{c}")).unwrap_or_default();
        t.push_str(&format!(
            " {}\n",
            span(
                GREEN,
                &escape(&format!(
                    "   ●  {}/{} · {} · {} · {} · {}{}",
                    f.run.owner, f.run.repo, f.run.workflow, f.run.branch, elapsed,
                    f.runner.short(), cost
                ))
            )
        ));
    }
    for f in s.failure_runs.iter().take(6) {
        t.push_str(&format!(
            " {}\n",
            span(
                RED,
                &escape(&format!(
                    "   ✖  {}/{} · {} · {}",
                    f.owner, f.repo, f.workflow, f.branch
                ))
            )
        ));
    }

    // Notes and health
    if !s.notes.is_empty() || s.stale_reason.is_some() {
        t.push_str(&format!("\n {}\n", rule()));
    }
    for n in &s.notes {
        t.push_str(&format!(" {}\n", span(ORANGE, &format!("  󰀪  {}", escape(n)))));
    }
    if let Some(r) = &s.stale_reason {
        t.push_str(&format!(
            " {}\n",
            span(
                ORANGE,
                &escape(&format!(
                    "  ⏸  Stale — {} ({} old)",
                    r,
                    human_duration(s.age_secs as i64)
                ))
            )
        ));
    }

    t.trim_end().to_string()
}

#[derive(serde::Serialize)]
struct WaybarOut {
    text: String,
    tooltip: String,
    class: String,
}

pub fn waybar_json(s: &Snapshot, fmt: &str) -> String {
    let sev = s.severity();
    let text = match expand(fmt, s) {
        Ok(t) => span(sev.colour(), &escape(&t)),
        Err(e) => span(RED, &escape(&format!("⚠ {e}"))),
    };
    let out = WaybarOut { text, tooltip: tooltip(s), class: sev.class().to_string() };
    serde_json::to_string(&out).unwrap_or_else(|_| {
        r#"{"text":"⚠","tooltip":"cicdbar: serialisation failed","class":"critical"}"#.into()
    })
}

/// Nothing worked at all. Waybar still gets a parseable line.
pub fn failure_json(reason: &str) -> String {
    let out = WaybarOut {
        text: span(RED, "⚠"),
        tooltip: format!(" {}\n {}", bold(RED, "cicdbar"), span(FG_TEXT, &escape(reason))),
        class: "critical".into(),
    };
    serde_json::to_string(&out)
        .unwrap_or_else(|_| r#"{"text":"⚠","tooltip":"cicdbar failed","class":"critical"}"#.into())
}
