# The Blacksmith dashboard API

> **Unofficial and unaffiliated.** This documents the private HTTP backend
> behind `app.blacksmith.sh`, observed on **2026-08-29** by inspecting the
> requests the dashboard makes in a normal browser session. It is not endorsed
> by Blacksmith, carries no stability guarantee, and may change or disappear
> without notice. It describes endpoints and response shapes only; it contains
> no credentials, and every call listed is one your own dashboard already makes
> on your behalf. Written because Blacksmith publishes no API documentation at
> all and `cicdbar` needed something to build against.

Blacksmith publishes **no billing API**. The only documented public endpoint
is their Statuspage (`https://status.blacksmith.sh/v3/summary.json`), which
carries service status and nothing else. Everything below was captured from
the `app.blacksmith.sh` frontend with Chrome DevTools while logged in.

## Backend

Base URL: `https://dashboardbackend.blacksmith.sh`

The dashboard is a Next.js app talking to a **Laravel** backend. Responses
carry `X-Correlation-Id`, `Server-Timing`, and CloudFront headers.

### Endpoints that matter

| Endpoint | Returns |
|---|---|
| `GET /api/user/github/orgs/{org}/billing/projected` | `{"amount_cents":140,"charges":{"sticky_disk":{"amount_cents":3,"units":135478.125,"gb_hours":37.63}}}` — current billing period charge, in **cents** |
| `GET /api/user/github/orgs/{org}/metrics/core-usage/current` | `{"current_usage":{"amd64":{"vcpus":16,"jobs":4,"held":0},"arm64":{…},"macos":{…}},"timestamp":"…"}` — live runner concurrency |
| `GET /api/user/github/orgs/{org}/metrics/core-usage/timeseries?window_size=15&start_date=…&end_date=…` | concurrency over time |
| `GET /api/user/github/orgs/{org}/billing/invoices?limit=50&page=1` | invoice history |
| `GET /api/user/github/orgs/{org}/billing/credits` | credit balance |
| `GET /api/user/github/orgs/{org}/billing/has-payment-method` | boolean |
| `GET /api/user/github/orgs` | orgs the session can see |

`cicdbar` uses the first two.

## Authentication — the part that bites

Auth is a **Laravel cookie pair**, not a bearer token:

* `remember_web_<hash>` — the durable credential (Laravel "remember me").
* `blacksmith_session` — **rotated by the server on every single response**,
  with a fresh `Set-Cookie` each time.

Two consequences, both learned the hard way:

1. **The session cookie alone is not enough.** Replaying only
   `blacksmith_session` returns `{"message":"Unauthenticated."}` (HTTP 401).
   Both cookies must be sent.
2. **A client that does not write the rotated cookie back authenticates
   exactly once.** The value captured from a browser request goes stale as
   soon as the browser (or an earlier call of your own) makes another request.

### The rotation problem

`cicdbar` keeps a small cookie jar: it merges every `Set-Cookie`
from the response and rewrites the session file (mode 0600) whenever a value
changed. `tests/blacksmith_live.rs::a_rotated_session_cookie_is_persisted_so_the_next_run_still_works`
pins exactly this — it asserts the file changed and that a second, independent
client can still authenticate with what was written.

Cookies carry `Max-Age=1209600` (14 days), refreshed on each call, so a widget
polling every 60s keeps the session alive indefinitely. When it does expire,
the provider raises an explicit "session expired" error and the widget falls
back to the estimate — it never reports `$0.00`, which would read as
"you spent nothing".

## Capturing a session

1. Open `https://app.blacksmith.sh/<org>/usage` in a browser and log in.
2. In DevTools → Network, pick any `dashboardbackend.blacksmith.sh` request.
3. Copy the whole `Cookie:` request header.
4. Write it to `~/.config/cicdbar/blacksmith-session`, mode `0600`.

The file is never committed and lives outside the repo.

## Fallback: pricing Blacksmith from GitHub's own data

Independently of the dashboard, Blacksmith spend can be *derived*: GitHub
reports each job's `labels` and timings, and Blacksmith runners are selected
by `runs-on: blacksmith-*`. `providers/blacksmith.rs` prices those minutes at
the published list rates (Ubuntu x64 $0.004/min, ARM $0.0025, Windows $0.008,
macOS M4 $0.08, per the 2-vCPU tier, scaled linearly by vCPU count; 3,000 free
minutes/month).

Labels observed in DataZooDE: `blacksmith-4vcpu-ubuntu-2404`.

This is a floor, not an invoice — it cannot see sticky-disk, cache or static-IP
charges — so it is always rendered as `Blacksmith ~est` with a line saying it
was priced from job minutes.


## Client checklist

If you are writing your own client, these are the four things that will bite
you, in the order they bit us:

1. **Send both cookies.** `blacksmith_session` alone is a 401.
2. **Persist `Set-Cookie` after every response.** Otherwise your second call
   fails, and it will look like the API rejecting you rather than your client
   discarding its own credential.
3. **Re-authenticate with `remember_web_*` on a 401** before treating it as
   fatal. A concurrent caller may have rotated the session out from under you;
   the durable cookie recovers without a new login.
4. **Handle `null` for object fields.** `current_usage` is `null` when nothing
   is running — which is most of the time — and so is `charges` when there are
   none. In serde terms `#[serde(default)]` is not enough; an explicit
   `Option` unwrap is needed.

## Response shapes

```jsonc
// GET /api/user/github/orgs/{org}/billing/projected
{
  "amount_cents": 140,                    // current period, in cents
  "charges": {                            // may be null
    "sticky_disk": {
      "amount_cents": 3,
      "units": 135478.125,
      "gb_hours": 37.6328125
    }
  }
}

// GET /api/user/github/orgs/{org}/metrics/core-usage/current
{
  "current_usage": {                      // null when nothing is running
    "amd64":  { "vcpus": 16, "jobs": 4, "held": 0 },
    "arm64":  { "vcpus": 0,  "jobs": 0, "held": 0 },
    "macos":  { "vcpus": 0,  "jobs": 0, "held": 0 }
  },
  "timestamp": "2026-08-29T06:35:57+00:00"
}
```

Amounts are **cents**, not dollars. `cicdbar` converts on the way in
(`usd_from_cents`) and keeps integer micro-dollars internally.

## What this cannot tell you

`billing/projected` is the charge for the current period as Blacksmith
computes it, which is the number you want. The label-derived fallback in
`providers/blacksmith.rs` is *not* equivalent: it prices job minutes at list
rates and cannot see sticky-disk, cache or static-IP charges, nor any
negotiated pricing. That is why it renders as `~est` with a line saying so,
rather than being silently substituted.
