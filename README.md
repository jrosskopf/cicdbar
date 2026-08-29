# cicdbar

A waybar widget showing **CI/CD dollar spend** (GitHub Actions + Blacksmith)
and **live job status**, with a detailed hover tooltip. Rust, built red/green
TDD against real APIs — no mocks anywhere in the suite.



```
$235 · ✖ 3 · 64%
```

## What it shows

The collapsed bar carries total month-to-date spend, a CI glyph
(`✓` clean, `●` running, `◌` queued, `✖` failing) with the running count, and
projected month-end spend as a percentage of budget. The tooltip expands to:

```
 CI/CD spend — August 2026
 ────────────────────────────────────────────

   󰊤  GitHub Actions   $232.99
   ████████████░░░░░░░░
      net of $2,750.28 gross ($2,517.28 discounted)
      DataZooDE  $232.99
      anofox  $0.00
      macOS 3-core $123 · Linux $80.09 · Windows $25.46 · storage $3.37
      heron $171 · anofox-context $20.58 · hetzner-agent-substrate $20.51

   󰛨  Blacksmith   $1.64
      1 job on 4 vCPUs right now

 ────────────────────────────────────────────
   󰄉  Projected month-end $257.18  (64% of $400.00)
   󰥔  Cycle resets in 2d 17h

 ────────────────────────────────────────────
   󰑮  3 running · 0 queued · 3 failing
    ●  DataZooDE/anofox-tabfm · Main Extension Distribution Pipeline · 1h10m · github-hosted
    ✖  DataZooDE/community-extensions · Community Extension Build · main
```

Colour follows **projected** month-end spend against the budget, not spend so
far, so a runaway shows on day 8 rather than at the invoice.

## Install

```sh
cargo build --release
cp target/release/cicdbar ~/.local/bin/
cp config.example.toml ~/.config/cicdbar/config.toml   # then edit
```

waybar module:

```json
"custom/cicdbar": {
    "exec": "/home/jr/.local/bin/cicdbar --format '{total_usd} · {run_glyph}{running} · {proj_pct}%{stale}'",
    "return-type": "json",
    "interval": 60,
    "tooltip": true,
    "on-click": "xdg-open https://github.com/organizations/<org>/settings/billing",
    "on-click-right": "xdg-open https://app.blacksmith.sh/<org>/usage"
}
```

Format placeholders: `{total_usd}` `{gh_usd}` `{bs_usd}` `{gross_usd}`
`{budget_usd}` `{proj_usd}` `{proj_pct}` `{cycle_reset}` `{running}`
`{queued}` `{failed}` `{inflight_usd}` `{run_glyph}` `{stale}`. An unknown
placeholder is an error, not silent output, so a typo in the waybar config is
visible immediately.

`--demo` renders fixed sample data offline; `--tooltip-only` prints the
tooltip as plain text for eyeballing in a terminal.

## How the numbers are obtained

**GitHub** — `GET /organizations/{org}/settings/billing/usage?year=&month=`,
authenticated with the existing `gh` CLI token (`repo` + `read:org` suffice;
no billing scope needed). Two things worth knowing, both pinned by tests:

* The `?year=&month=` filter is **mandatory** for per-repo detail. Unfiltered,
  the endpoint collapses to a monthly rollup — 40 rows instead of 1,608 — with
  one arbitrary representative repo per SKU.
* The rollup and the detail **disagree on storage**: identical gross, but the
  detail applies the included-storage allowance and the rollup does not
  ($3.37 vs $58.78 for August 2026). They agree to the cent on every compute
  SKU. cicdbar sources everything from the detail and raises a tooltip note
  about the gap, so it can be reconciled against the invoice rather than
  silently papered over.

**Blacksmith** — they publish no billing API, so this reads the undocumented
`dashboardbackend.blacksmith.sh` backend behind their dashboard. See
[`blacksmith-api-notes.md`](blacksmith-api-notes.md); the short version is
that auth is a Laravel cookie pair whose session half rotates on every
response, so the client keeps a jar and writes it back. If the session
expires, the widget falls back to pricing `blacksmith-*` job minutes from
GitHub's own job records at published list rates, labelled `~est` — it never
reports `$0.00`, which would read as "you spent nothing".

**Job status** — GitHub has no org-wide in-flight endpoint, so cicdbar
discovers repos pushed within `active_days` and asks each one, bounded by
`max_repos`. A failure counts only when it is the newest run of its workflow
on the default branch — still broken, not merely broken once.

## Design notes

Stateless: waybar re-execs it every 60s and all state is an on-disk cache
under `$XDG_CACHE_HOME/cicdbar` (billing 15 min, runs 45 s). A failed fetch
never blanks the widget while any previous value survives — it is served and
flagged `⏸`. Whatever goes wrong, exactly one line of valid waybar JSON is
printed and the exit code is 0.

Money is integer micro-dollars throughout. The billing API quotes prices like
`0.00033602` across thousands of rows, and the aggregate must satisfy
`gross - discount = net` exactly; summing three f64 fields independently does
not.

Cold latency was 17s when every repo was polled serially, which would block
waybar's tick. A bounded fan-out over orgs, repos and job lookups brought it
to ~3.5s, held by a live performance test.

## Rate limiting

GitHub enforces a **secondary** limit on burst concurrency that is separate
from the 5,000/hr quota — you can be throttled with 5,000 requests still
showing as remaining. It arrives as a `403` whose body says "API rate limit
exceeded", which is indistinguishable from a permissions `403` unless you read
the message; classifying it wrongly makes the widget claim you lost billing
access mid-burst. `http::classify` separates them, and the retry is
deliberately short — secondary limits persist for minutes, and serving a
slightly stale number beats blocking waybar's tick.

A normal tick is ~20 requests at 4-way concurrency, well inside both limits.
The test suite is what trips them, which is why it runs serially:

```sh
./run-tests.sh
```

## Tests

Every test talks to a real system: the live GitHub API with the real `gh`
token, the live Blacksmith dashboard, the real filesystem, a real closed TCP
port for the unreachable-API path, and the real compiled binary via
`Command`. There are no mocks and no recorded fixtures.

That has a cost — the suite is subject to the world changing underneath it —
and it has repeatedly been worth it. Tests written against the real API caught
that `created>=` had no upper bound, that the billing aggregate's
`gross - discount = net` invariant drifted under f64 summation, that the
dashboard sends `"current_usage": null` when idle, that a 403 can mean two
different things, and that serial polling took 17s. None of those would have
failed against a mock built from my own assumptions.

Two tests had to be rewritten because they asserted facts about the world
rather than about the code: one assumed a repo did not use Blacksmith runners
(it adopted them mid-August), and one compared two billing endpoints for exact
equality while CI was actively accruing spend between the two calls.
