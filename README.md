# cicdbar

A [waybar](https://github.com/Alexays/Waybar) widget showing **CI/CD dollar
spend** — GitHub Actions and [Blacksmith](https://blacksmith.sh) — alongside
**live job status**, with a detailed hover tooltip.

<img src="docs/img/bar.png" alt="the cicdbar waybar module" width="156">

Total spend this billing cycle, a CI glyph with the running count, and
projected month-end spend as a percentage of your budget. Hovering expands
it:

<img src="docs/img/tooltip.png" alt="the cicdbar tooltip" width="394">

*(Screenshots taken with `--demo`, so the figures and repository names are
synthetic.)*

[![CI](https://github.com/jrosskopf/cicdbar/actions/workflows/ci.yml/badge.svg)](https://github.com/jrosskopf/cicdbar/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cicdbar.svg)](https://crates.io/crates/cicdbar)
[![licence](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)

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

**Arch (AUR)**

```sh
paru -S cicdbar        # builds from source
paru -S cicdbar-bin    # prebuilt static binary
```

**cargo**

```sh
cargo install cicdbar
```

**Prebuilt binary** — grab the static `x86_64` tarball from
[Releases](https://github.com/jrosskopf/cicdbar/releases); it has no runtime
dependencies.

**From source**

```sh
cargo build --release
cp target/release/cicdbar ~/.local/bin/
```

Then configure:

```sh
mkdir -p ~/.config/cicdbar
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

## Requirements

* A GitHub token with `repo` and `read:org` — **no billing scope needed**. By
  default it reads the `gh` CLI's own token, so if `gh auth status` works, so
  does this.
* Billing-read access to the orgs you list.
* Blacksmith is optional; set `enabled = false` if you do not use it.

## How the numbers are obtained

**GitHub** — `GET /organizations/{org}/settings/billing/usage`, both filtered
and unfiltered. Two things about that endpoint cost real time to discover: the
`?year=&month=` filter is **mandatory** for per-repo detail, and the two calls
**disagree about storage** — by $55 a month, with only one of them matching
the invoice. cicdbar takes compute from the filtered call and storage from the
unfiltered one. Both findings, and the invoice that settled the second, are in
[`docs/github-billing.md`](docs/github-billing.md).

**Blacksmith** — they publish no billing API, so this reads the undocumented
backend behind their dashboard, documented in
[`docs/blacksmith-api.md`](docs/blacksmith-api.md). Auth is a Laravel cookie
pair whose session half rotates on *every* response, so a naive client
authenticates exactly once. If the session expires, the widget falls back to
pricing `blacksmith-*` job minutes from GitHub's own job records at published
list rates, labelled `~est` — it never reports `$0.00`, which would read as
"you spent nothing".

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

Latency, measured against the real API with 15 repos in scope:

| tick | requests | 304s | wall clock |
|---|---|---|---|
| cache warm (inside TTL) | 0 | – | 4 ms |
| cache expired, ETags warm | 22 | 22 | ~6 s |
| fully cold | 24 | 0 | ~6 s |

Polling those repos serially took **17 s**; a bounded fan-out (6 concurrent,
chosen to stay under GitHub's burst limit) brought it to ~6 s, and a live
performance test guards against the regression. The remaining 6 s is network
round-trips, not quota — nearly every request is a 304. Waybar spawns the
module asynchronously, so this never freezes the bar.

## Notifications

cicdbar can raise a desktop notification (D-Bus,
`org.freedesktop.Notifications`) when a run starts and when it finishes. A
run occupies **one** notification for its whole life: the "started" one is
replaced in place by its result rather than stacking a second.

Failures are sent at critical urgency, so daemons that honour it keep them
until dismissed.

**The defaults are loud on purpose** — start plus every finish. Across a busy
org that is a few hundred a day. Two lines make it about one an hour:

```toml
[notifications]
on_start = false
on_finish = "failures"
```

`--no-notify` suppresses them for a single invocation.

> **mako users:** cicdbar deliberately sends no `x-dunst-stack-tag` hint.
> mako 1.11.0 **crashes** when a notification carrying that hint is replaced
> via `replaces_id` — reproducible from `busctl` with no cicdbar involved.
> `replaces_id` alone gives the same coalescing, so nothing is lost.

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

See [`docs/testing.md`](docs/testing.md) for the full story.

```sh
cargo test          # offline suites — no credentials needed
./run-tests.sh      # everything, including the live suites
```

Every test talks to a real system: the live GitHub API with the real `gh`
token, the live Blacksmith dashboard, the real filesystem, a real closed TCP
port for the unreachable-API path, and the real compiled binary via
`Command`. There are no mocks and no recorded fixtures.

That has a cost and it has been worth it: tests written against the real API
caught five bugs that a mock built from my own assumptions would have
confirmed rather than caught. They are listed in
[`docs/testing.md`](docs/testing.md).

Live tests are `#[ignore]`d, so a fresh clone runs 42 offline tests and
passes without credentials.

## Versioning

Calendar versioning: `YYYY.M.D`, tagged `vYYYY.M.D`. Month and day are not
zero-padded — semver forbids leading zeros in numeric identifiers and Cargo
enforces it, so `2026.08.29` is not a legal crate version. A later date is a
later release; the numbers carry no other promise.

## Licence

MIT — see [LICENSE](LICENSE).
