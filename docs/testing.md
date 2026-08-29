# Testing

There are no mocks in this project, and no recorded fixtures. Every test
talks to a real system: the live GitHub API, the live Blacksmith dashboard,
the real filesystem, a real closed TCP port, or the real compiled binary.

That is a deliberate trade with a real cost — the suite is subject to the
world changing underneath it, and it needs credentials to run. It has been
worth it. Tests written against reality caught, in order:

1. `created>=` on the workflow-runs query had **no upper bound**, so a query
   for January 2025 silently matched every later month too. A fixture would
   have returned whatever I had recorded and agreed with me.
2. Summing `gross`, `discount` and `net` independently broke the
   `gross - discount = net` invariant by 2 micro-dollars across 1,608 real
   rows. Discount is now derived, so the invariant holds by construction.
3. The Blacksmith dashboard sends `"current_usage": null` when idle — the
   common case at night — which `#[serde(default)]` does not cover.
4. A GitHub **403** means either "no access" or "too fast", and the widget was
   reporting a burst throttle as "no billing access".
5. Polling 15 repos serially took **17 seconds**.

None of those would have failed against a mock built from my own assumptions,
because my assumptions were the bug in four of the five cases.

## Running them

```sh
cargo test          # offline suites only — no credentials needed
./run-tests.sh      # everything, including the live suites
```

Live tests are `#[ignore]`d with a reason, so a fresh clone runs 42 offline
tests and passes. `run-tests.sh` adds `--include-ignored`.

## What each suite touches

| Suite | Talks to | Credentials |
|---|---|---|
| `money` | nothing | — |
| `config_and_cycle` | real filesystem | — |
| `render` | nothing | — |
| `cache` | real filesystem, a real closed TCP port | — |
| `github_billing_live` | live GitHub billing API | gh token, billing-read on the org |
| `github_runs_live` | live GitHub Actions API | gh token |
| `blacksmith_live` | live GitHub + Blacksmith dashboard | gh token, Blacksmith session |
| `etag_live` | live GitHub API | gh token |
| `performance_live` | the real binary against live GitHub | gh token |

## These tests are not runnable by a stranger

The live suites authenticate as their author against specific organisations,
and assert against those organisations' real CI activity. Cloning this repo
does not make them pass for you — you would need your own org, your own token,
and to change the org names in the test files.

That is a genuine limitation and not one worth engineering around: making them
runnable by anyone would mean recording fixtures, and the fixtures would be my
assumptions again. Contributors get the offline suites, which are the gate
that matters for refactoring.

If you are adapting this for your own org, the names to change are in
`tests/*_live.rs` — search for `DataZooDE`.

## Pacing, and why the suites run one at a time

`run-tests.sh` runs live suites serially with a pause between them. Cargo runs
test *binaries* concurrently by default, and the resulting burst reliably trips
GitHub's secondary rate limit (see `docs/github-billing.md`), which then
lingers for minutes and looks exactly like a code failure.

The performance suite additionally runs with `--test-threads=1`, because it
measures wall-clock latency and cannot share bandwidth with its neighbours.

## No tests in CI

CI builds, formats and lints; it does not test. The suite needs live
credentials, and putting them in a public repo's CI would mean secrets
inaccessible to fork PRs, a rate-limit budget spent on every push, and a
14-day Blacksmith cookie that expires and reddens the build.

The consequence, stated plainly: **nothing automatically catches GitHub or
Blacksmith changing their APIs.** These tests are the detector, and they fire
when someone runs them. If that becomes a problem, the fix is a nightly
scheduled workflow with repo secrets.
