# GitHub Actions billing, as it actually behaves

Two things about `GET /organizations/{org}/settings/billing/usage` cost real
time to discover. Both are pinned by tests in
`tests/github_billing_live.rs`, so if GitHub changes them, the suite says so.

Observed 2026-08-29.

## 1. Per-repo detail requires `?year=&month=`

The endpoint returns two quite different things depending on whether you
filter it:

| Call | Rows | Repos named | Resolution |
|---|---|---|---|
| `…/billing/usage` | 40 | 17 | one row per (month, SKU) |
| `…/billing/usage?year=2026&month=8` | 1,608 | 43 | per repo, per SKU, hourly |

Unfiltered, each `(month, SKU)` row carries a **single** `repositoryName` —
which looks like per-repo data but is one arbitrary representative. Summing
those to build a per-repo breakdown produces a plausible, wrong answer.

Any per-repo view needs the filtered call.

## 2. The rollup and the detail disagree on storage

For the same month and the same org:

| SKU | Detail (net) | Rollup (net) |
|---|---|---|
| Actions Linux | $79.98 | $79.98 |
| Actions Linux ARM | $0.84 | $0.84 |
| Actions Windows | $25.34 | $25.34 |
| Actions macOS 3-core | $122.49 | $122.49 |
| **Actions storage** | **$3.37** | **$58.78** |

Gross is identical on both sides ($58.7756) and the quantity is identical
(174,917 GB-hours). The difference is entirely the discount: the detail rows
apply $55.41 of included-storage allowance; the rollup applies **zero**.

Compute SKUs agree to the cent, which is what makes the storage gap look like
a property of the endpoints rather than a timing artefact.

### The rollup is the one that matches the invoice

Settled against a real bill. July 2026 is a completed month, so its invoice is
final, and the two interpretations predicted different totals:

| | Storage | Month total |
|---|---|---|
| if the detail were right | $2.91 | $167.55 |
| **if the rollup is right** | **$45.99** | **$210.63** |

The invoice said **$210.63**. The discount the detail rows report against
storage — $43.08 for July — is **not applied to the bill**.

So `cicdbar` takes **compute from the detail** (the only source with repo
granularity, and it agrees with the rollup to the cent) and **storage from the
rollup** (the only one that matches what you pay). Sourcing storage from the
detail understated spend by roughly $55 a month.

Storage is deliberately left out of the per-repo breakdown: the rollup names a
single arbitrary repository per SKU, so attributing it would be inventing a
breakdown that does not exist.

If you are writing your own client, this is the trap: the detail rows look
more precise, carry more structure, and are wrong about money.

## Access and scopes

* The **legacy** endpoints (`/orgs/{org}/settings/billing/actions`) are gone —
  HTTP 410, "This endpoint has been moved".
* The usage endpoint needs **no billing scope**. A token with `repo` and
  `read:org` — which is what `gh auth login` gives you by default — is enough.
* An org where you lack billing access returns **403** with
  `"No access to billing usage data."`; one that does not exist (to you)
  returns **404**. `cicdbar` degrades both to a tooltip note.

## Two rate limits, not one

Beyond the documented 5,000 requests/hour, GitHub enforces a **secondary**
limit on burst concurrency. You can be throttled with 5,000 requests still
showing as remaining, and it lingers for minutes.

It arrives as a **403** whose body reads `API rate limit exceeded for user ID
…` — structurally identical to a permissions 403. Distinguishing them requires
reading the message; `http::classify` does, because getting it wrong made the
widget report "no billing access" mid-burst.

Also worth knowing: **`/rate_limit` is not a reliable measuring instrument.**
It is eventually consistent and resets underneath you — it once reported 452
requests consumed by a tick that issued 24. `cicdbar` counts its own requests
(`--stats`) instead.

Conditional requests are the cheap defence: **304 responses do not count
against the REST quota**. A steady-state tick issues 22 requests of which 22
are 304s.
