# cicdbar — implementation plan

A waybar widget in Rust showing **CI/CD dollar spend** (GitHub Actions +
Blacksmith) and **live job status**, with a detailed hover tooltip.
Modelled on `claudebar` (`/usr/bin/claudebar`, pacman package
`claudebar 0.8.1-1`).

Built red/green TDD, no mocks: tests exercise the real code over a real
loopback HTTP server replaying captured fixtures, and the definition of
done is the widget working end-to-end in the live waybar.

---

## 1. Decisions (agreed 2026-08-29)

| Question | Decision |
|---|---|
| GitHub scope | All owned orgs, **listed in a config file** (DataZooDE, anofox, octoopt, sparrowbi, zoitech-internal) |
| GitHub auth | Reuse the `gh` CLI token from `~/.config/gh/hosts.yml` |
| Blacksmith source | **Reverse-engineer the `app.blacksmith.sh` dashboard API** |
| Bar text | Total spend **and** Actions job progress/status |
| Status surfaces | running/queued count, failures since last success, estimated cost of in-flight runs, per-run tooltip detail |
| Architecture | Stateless CLI + on-disk cache (billing ~15 min, runs ~30 s), stale marked with ⏸ |
| Cost basis | `netAmount` (what is actually paid); gross + discount in tooltip |
| Colour | Pace-based projection against a monthly budget |
| Location | `2026-08-29-cicdbar/` here now, extract to its own repo + PKGBUILD once it works |

---

## 2. What the probe established

**GitHub billing already works with the current token.** No re-auth needed.

```
GET /organizations/{org}/settings/billing/usage
```

returns rows of:

```json
{"date":"2026-08-01T00:00:00Z","product":"actions","sku":"Actions macOS 3-core",
 "quantity":29177.0,"unitType":"Minutes","pricePerUnit":0.062,
 "grossAmount":1808.974,"discountAmount":1686.482,"netAmount":122.492,
 "organizationName":"DataZooDE","repositoryName":"datazoo-oauth2"}
```

DataZooDE, August 2026 to date, `netAmount`: Linux $79.98, Windows $25.34,
macOS $122.49, ARM $0.84, storage $58.59 → **≈ $287**.

- The legacy `/orgs/{org}/settings/billing/actions` endpoint is **gone** (HTTP 410,
  "This endpoint has been moved").
- ⚠ **Open question**: the unfiltered response came back with one row per
  `(month, sku)` carrying a *single* `repositoryName`. Either GitHub aggregates
  and reports a representative repo, or per-repo rows require explicit
  `?year=&month=` filtering. **Step 1 of the build verifies this** — the per-repo
  tooltip breakdown depends on the answer.

**Blacksmith has no billing API.** The only public API is their Statuspage
(`https://status.blacksmith.sh/v3/summary.json` — status only). Billing lives
behind `app.blacksmith.sh`. Published rates, for cross-checking whatever the
dashboard returns: Ubuntu x64 $0.004/min, Ubuntu ARM $0.0025/min, Windows x64
$0.008/min, macOS M4 $0.08/min; 3,000 free minutes/month; Docker layer caching
and sticky disks $0.50/GB/month; static IPs $100/IP/month. Rates are quoted for
the base vCPU tier and scale with runner size.

I attempted to read the Brave cookie DB for a `blacksmith.sh` session and the
sandbox blocked it; I did not work around that. Phase 0 below does the capture
with you in the loop instead.

---

## 3. Phase 0 — capture the Blacksmith dashboard API (do this first, together)

Blocking for the Blacksmith half only; the GitHub half proceeds in parallel.

1. Launch Chrome/Brave with remote debugging against your **existing profile**
   so the login session is present:
   `--remote-debugging-port=9222 --user-data-dir=<your Brave profile>`.
2. You log in at `app.blacksmith.sh` and open **Billing and Usage**.
3. Capture the XHR traffic (chrome-devtools `list_network_requests` /
   `get_network_request`), and record:
   - the usage/billing endpoint URL and its query parameters,
   - the auth mechanism (session cookie vs `Authorization: Bearer` — if it is a
     Clerk/Auth0 JWT, note the **expiry**),
   - the response JSON shape, saved (redacted) as a test fixture,
   - whether an org/tenant id must be passed.
4. Decide token storage from what we find: long-lived cookie → `pass` entry;
   short-lived JWT → we need a refresh path, and if there is none we fall back to
   computing Blacksmith spend from GitHub job data (`runs-on: blacksmith-*`
   labels × the rate table above), marked `~est` in the tooltip.

Deliverable: `2026-08-29-cicdbar/blacksmith-api-notes.md` + fixtures.

---

## 4. Architecture

Single Rust binary, invoked by waybar every 60 s, always exits 0 and always
prints one line of valid waybar JSON — including on total failure (claudebar's
`{"text":"⚠",...,"class":"critical"}` pattern).

```
2026-08-29-cicdbar/
  Cargo.toml
  config.example.toml
  src/
    main.rs               CLI parse → orchestrate → emit JSON, never panics
    config.rs             TOML load, XDG paths, defaults
    money.rs              Usd newtype over i64 micro-dollars (no float money)
    model.rs              Spend, SpendRow, RunSummary, JobRun, Snapshot, Health
    clock.rs              trait Clock (real + fixed, for month-boundary tests)
    cache.rs              TTL read/write under XDG_CACHE_HOME, stale detection
    http.rs               blocking client, ETag store, retry/backoff, timeouts
    providers/
      mod.rs              trait Provider -> Result<Partial>; per-provider Health
      github_billing.rs   /organizations/{org}/settings/billing/usage
      github_runs.rs      repo discovery + in-flight runs + failures + jobs
      blacksmith.rs       dashboard API (shape fixed by Phase 0)
    aggregate.rs          merge partials, per-cycle totals, pace projection
    render/
      bar.rs              --format placeholder expansion
      tooltip.rs          Pango markup: sections, progress bars, colours
      pango.rs            escaping + colour palette (claudebar's One Dark)
  tests/
    fixtures/*.json       real captured responses, redacted
    *.rs                  behaviour tests over a real loopback HTTP server
```

**Concurrency**: providers fetch in parallel; a provider that fails degrades to
its cached value and reports `Health::Degraded(reason)` rather than failing the
whole widget — the tooltip shows a per-provider status line, as claudebar does
for its HTTP 429.

**Crates**: `reqwest` (blocking, rustls), `serde`/`serde_json`, `toml`,
`jiff` or `time` for month boundaries, `anyhow`/`thiserror`, `clap`,
`rayon` or plain threads. Tests: `wiremock` (real HTTP server on loopback),
`insta` for snapshotting rendered Pango output. Per repo convention the build
is plain `cargo`; no Python involved.

**Config** `~/.config/cicdbar/config.toml`:

```toml
budget_usd = 400            # monthly, drives pace colouring

[github]
orgs = ["DataZooDE", "anofox", "octoopt", "sparrowbi", "zoitech-internal"]
token_source = "gh-cli"     # or "env:GH_TOKEN" / "pass:github/cicdbar"

[runs]
active_days = 7             # only poll repos pushed within this window
max_repos = 40              # hard cap on per-tick API cost

[blacksmith]
enabled = true
org = "DataZooDE"
token_source = "pass:blacksmith/session"

[cache]
billing_ttl_secs = 900
runs_ttl_secs = 30
```

**Rate-limit budget**: GitHub core is 5,000 req/h; at 60 s ticks that is ~83
requests per tick available. Repo discovery is 1 request per org (`?sort=pushed`),
in-flight runs 1 per active repo, job detail only for runs that are actually
running. All requests carry `If-None-Match`; 304s are cheap. `max_repos` plus the
30 s runs-cache keeps a tick well inside budget.

---

## 5. Output contract

Bar text is a `--format` string, claudebar-style, so the waybar config stays
declarative:

```
cicdbar --format '{total_usd} · {run_glyph}{running} · {proj_pct}%'
```

Placeholders: `{total_usd}` `{gh_usd}` `{bs_usd}` `{gross_usd}` `{budget_usd}`
`{proj_usd}` `{proj_pct}` `{cycle_reset}` `{running}` `{queued}` `{failed}`
`{inflight_usd}` `{run_glyph}` `{stale}`.

Tooltip sections (Pango markup, One Dark palette, progress bars like claudebar):

```
 Spend — August 2026
 ────────────────────────────────
   󰇙  GitHub Actions        $287.41
      ██████████░░░░░░░░  net of $2,746 gross
      DataZooDE  $287.41 · anofox  $0.00 …
      macOS $122 · Linux $80 · Windows $25 · storage $59
   󰡨  Blacksmith             $41.20
 ────────────────────────────────
   󰄉  Projected month-end  $372  (93% of $400 budget)
   󰥔  Cycle resets in 2d 6h
 ────────────────────────────────
 In flight
   ●  DataZooDE/erpl-idoc · release.yml · main · 6m12s · blacksmith-8vcpu · ~$0.19
   ●  DataZooDE/flapi     · ci.yml      · pr/412 · 1m03s · ubuntu-latest  · ~$0.01
   ✖  DataZooDE/heron     · ci.yml      · main   · failed 22m ago
 ────────────────────────────────
   ⏸  Stale — data from 06:14 (2 min ago)
```

Waybar module:

```json
"custom/cicdbar": {
    "exec": "cicdbar --format '{total_usd} · {run_glyph}{running} · {proj_pct}%'",
    "return-type": "json",
    "interval": 60,
    "tooltip": true,
    "on-click": "xdg-open https://github.com/organizations/DataZooDE/settings/billing",
    "on-click-right": "xdg-open https://app.blacksmith.sh"
}
```

`class` is `ok` / `low` / `warning` / `critical` so `style.css` can colour it
alongside the existing claudebar rules.

---

## 6. TDD sequence

Each step: write the failing test, watch it fail for the right reason, make it
pass, refactor. No trait-level mocking — HTTP tests run against a real
`wiremock` server on loopback serving captured fixtures, so the real
`reqwest` client, real headers, real deserialisation are all exercised.

| # | Red test | Green implementation |
|---|---|---|
| 1 | `usd_arithmetic_is_exact` — sums of `0.006`-priced rows do not drift | `money.rs`: micro-dollar integer newtype, `Display` as `$1,234.56` |
| 2 | `parses_real_billing_fixture` — captured DataZooDE JSON → typed rows | `github_billing.rs` deserialisation |
| 3 | **`per_repo_rows_are_present_for_current_month`** — resolves the open question from §2 by asserting against a fixture captured *with* `?year=2026&month=8` | filter/aggregation by cycle; if GitHub really does collapse per-repo detail, the tooltip drops to per-SKU and the test is rewritten to pin that |
| 4 | `sums_net_gross_discount_across_orgs`, `empty_org_contributes_zero` | multi-org aggregation |
| 5 | `config_defaults_and_overrides`, `rejects_unknown_keys` | `config.rs` |
| 6 | `reads_gh_cli_token`, `missing_token_is_degraded_not_panic` | token sources |
| 7 | `cache_hit_within_ttl_skips_http` (assert zero server hits), `expired_ttl_refetches`, `corrupt_cache_file_recovers`, `unreachable_api_serves_stale_and_marks_it` | `cache.rs` |
| 8 | `projection_at_mid_month`, `projection_on_first_day`, `dst_and_month_boundaries_are_utc`, `colour_class_boundaries` (59/60/85/86%) | `aggregate.rs` + fixed `Clock` |
| 9 | `discovers_repos_pushed_within_window`, `respects_max_repos`, `sends_if_none_match_and_handles_304` | `github_runs.rs` |
| 10 | `counts_running_and_queued`, `detects_failure_since_last_success_on_default_branch` | run classification |
| 11 | `classifies_blacksmith_jobs_by_label`, `inflight_cost_accrues_with_elapsed_time`, `unknown_label_is_not_charged` | label → rate table |
| 12 | `parses_blacksmith_dashboard_fixture`, `expired_token_degrades_to_estimate` | `blacksmith.rs` (after Phase 0) |
| 13 | `format_expands_all_placeholders`, `unknown_placeholder_is_an_error` | `render/bar.rs` |
| 14 | `tooltip_snapshot` (insta), `escapes_ampersand_in_repo_names`, `truncates_at_max_lines` | `render/tooltip.rs` |
| 15 | `emits_valid_waybar_json_on_total_failure` — no network at all still yields parseable JSON with `class:"critical"` | `main.rs` error path |
| 16 | `live_github_billing` *(`#[ignore]`, `--features live)`* — real API, real token, asserts a plausible non-zero net | end-to-end proof |

**Green gate** (per your standing rule that tests must be broad and wired in):
`cargo test` runs 1–15 on every change; `cargo test --features live -- --ignored`
runs 16 before any commit that touches a provider. Any bug found later gets a
regression test at the layer where it actually broke, added to the same gate.

---

## 7. Definition of done

Not "tests pass" — the widget works on the real system:

1. `cargo test` green, including the live GitHub test.
2. `cicdbar --format …` run by hand emits correct JSON; the dollar figure
   reconciles against the GitHub billing page (and, for Blacksmith, against the
   dashboard) to within rounding.
3. The module is added to `~/.config/waybar/config` and styled in `style.css`;
   waybar reloaded; **the widget is visible and hovering shows the tooltip** —
   confirmed by a screenshot in the experiment directory.
4. It survives the failure cases live: network down, token revoked, an org with
   no spend, a Blacksmith token expiry.
5. Then, and only then, extract to `~/Projects/cicdbar` with a PKGBUILD so it
   installs like claudebar.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Billing API lacks per-repo granularity | Verified in test 3 before the tooltip is designed around it; per-SKU fallback |
| Blacksmith auth is a short-lived JWT | Fall back to the computed estimate from GitHub job labels × rate table, labelled `~est` |
| Blacksmith changes their frontend | Fixture-backed parser fails loudly into `Health::Degraded`; the widget still shows GitHub spend |
| Rate limits across 5 orgs | `active_days` + `max_repos` caps, ETags, split cache TTLs |
| Blocking waybar on slow HTTP | Hard per-request timeout; on timeout serve cache and mark ⏸ |
| Money drift from f64 | Integer micro-dollars throughout (test 1) |
