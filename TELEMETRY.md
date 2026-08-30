# Telemetry in cicdbar

`cicdbar` collects **anonymous, aggregate usage telemetry** so I can see which
features are used and where they fail. It is **on by default** and trivial to
turn off.

It emits the shared DataZoo `telemetry_schema: 2` envelope — the same one used
by `erpl`, `erpl-adt` and `flapi` — so this product is comparable with the rest
of the stack. Data is ingested by PostHog in the EU
(`https://eu.i.posthog.com`). The schema is vendored at
[`docs/TELEMETRY-SCHEMA.md`](docs/TELEMETRY-SCHEMA.md).

## How to opt out

Any **one** of these disables it completely — nothing leaves your machine, and
no local state is kept either:

| Method | Scope |
|---|---|
| `--no-telemetry` | the single invocation |
| `DATAZOO_DISABLE_TELEMETRY=1` (or `true`/`yes`) | cross-product kill switch |
| `DO_NOT_TRACK=1` | honours <https://consoledonottrack.com/> |
| `CICDBAR_NO_TELEMETRY=1` | product-local |
| `telemetry.enabled = false` in `config.toml` | permanent, per install |

On the first run with telemetry enabled, cicdbar prints one line to stderr
pointing here.

## The privacy contract

cicdbar can see what your CI costs, which repositories you have, and which
branches are failing. **None of that is ever transmitted.** Specifically, we
never send:

- **Any dollar amount** — not spend, not your budget, not even a bucket of
  either. Telemetry cannot learn what your CI costs.
- Organisation, repository, workflow, branch or actor **names**
- GitHub tokens or Blacksmith session cookies
- File paths, hostnames or usernames
- Error **messages** — only a class from a fixed enum

This is enforced by construction, not by discipline: every property passes an
**allow-list** in `src/telemetry.rs` naming both the permitted keys *and*, for
string properties, the exact permitted values. Anything else is dropped before
serialisation, so a mistake at a call site cannot become a leak. Strings are
additionally clamped to 512 bytes as a backstop.

Two tests pin this (`tests/telemetry.rs`): one feeds real-looking org names,
branches and tokens through the capture path and asserts none appear in any
payload; another does the same for currency amounts.

## Identifiers

`distinct_id` is the **salted SHA-256 of your OS machine id**
(`/etc/machine-id`, MAC address fallback). It is a stable pseudonymous machine
hash — not your username, not your IP — and matches the id the other DataZoo
products use, so one machine correlates across the stack. The salt means it
cannot be reproduced from a raw machine id held in some other dataset.

Where no stable hardware id exists (many containers and CI runners), a
per-process **ephemeral** id is used instead and stamped
`$process_person_profile: false`, so those events never create a Person.

## What is collected

Every event carries the shared envelope: `product` (`cicdbar`),
`product_version`, `product_edition` (`oss`), `os`, `arch`, `platform`,
`is_ci`, `is_container`, `$session_id`, `identity_source`.

### One event per day, not one per tick

waybar re-execs cicdbar **every 60 seconds**, and once per output — roughly
2,880 times a day on a two-monitor setup. Sending an event per invocation
would be a firehose of no analytical value, and would put an HTTPS round trip
on a path that otherwise completes in 4 ms.

So a tick only increments counters in the local cache. **At most once every 24
hours**, one `feature_used` event carries bucketed aggregates:

| Property | Example | Meaning |
|---|---|---|
| `feature` | `spend_shown` | enum |
| `install_kind` | `waybar` | enum |
| `ticks_bucket` | `20+` | how much it ran, bucketed |
| `orgs_bucket` | `2-5` | how many orgs are configured, bucketed |
| `repos_bucket` | `6-20` | how many repos are polled, bucketed |
| `blacksmith_enabled` | `true` | whether Blacksmith is in use |
| `dashboard_session_valid` | `true` | exact figures vs the fallback estimate |
| `notifications_enabled` | `true` | |
| `degraded` | `3` | ticks served from stale cache |

Counts are **bucketed, never exact** — an exact repo count is identifying, a
bucket is not.

`$exception` events carry only an `error_class` from a fixed enum
(`auth`, `access_denied`, `rate_limited`, `unreachable`, `decode`, `other`)
and a `phase`, at most one per class per day.

## Why it is on by default

Opt-in telemetry is answered by almost nobody, which makes it worthless for
deciding what to build. On-by-default with a genuine, verifiable privacy
contract and five ways to turn it off is the trade I have made. If you would
rather not, the table at the top takes one command.
