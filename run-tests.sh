#!/usr/bin/env bash
# Live tests are #[ignore]d so a bare `cargo test` on a fresh clone runs the
# offline suites and passes without credentials. This script includes them.
#
# Every test in this suite talks to a real system -- the live GitHub API, the
# live Blacksmith dashboard, the real filesystem, the real binary. Cargo runs
# test *binaries* concurrently, which bursts enough requests at GitHub to trip
# its secondary rate limit (distinct from the 5,000/hr quota, and it lingers
# for minutes). So the live suites run one at a time.
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release --tests

offline=(money config_and_cycle render cache transitions)
live=(github_billing_live github_runs_live blacksmith_live etag_live notify_live performance_live)

fail=0
for t in "${offline[@]}"; do
    printf '\n\033[1m== %s ==\033[0m\n' "$t"
    cargo test --release --test "$t" || fail=1
done

# Live suites are paced apart. Back to back they trip GitHub's secondary
# rate limit even though the hourly quota is nowhere near exhausted, and it
# then lingers for minutes -- which looks like a code failure but is not.
first=1
for t in "${live[@]}"; do
    [ "$first" -eq 1 ] || { echo "   (pausing to stay under GitHub's burst limit)"; sleep 25; }
    first=0
    printf '\n\033[1m== %s ==\033[0m\n' "$t"
    # The performance suite measures wall-clock latency, so it runs alone.
    # notify_live shares one global resource -- the notification daemon --
    # so concurrent tests dismiss each other's notifications.
    if [ "$t" = performance_live ] || [ "$t" = notify_live ]; then
        cargo test --release --test "$t" -- --include-ignored --test-threads=1 || fail=1
    else
        cargo test --release --test "$t" -- --include-ignored --test-threads=2 || fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo; echo "FAILURES"; exit 1
fi
echo; echo "all suites green"
