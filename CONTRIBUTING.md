# Contributing

Bug reports and patches are welcome.

## Before you open a PR

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test            # offline suites; no credentials needed
```

CI runs exactly those three, plus a musl build. It does **not** run the live
test suites — they need a GitHub token and a Blacksmith session, which a fork
cannot have. See [docs/testing.md](docs/testing.md).

## A note on the test suite

This project has no mocks and no recorded fixtures, on purpose: the live tests
have caught five bugs that a fixture built from the author's assumptions would
have happily confirmed. The trade-off is that the live suites are not runnable
by a stranger — they authenticate against specific organisations.

If you are changing provider code, please say in the PR whether you were able
to run the live suites against your own org, and what you saw. If you could
not, that is fine; say so, and the maintainer will run them.

If you are changing pure logic — money, cycle maths, rendering, caching — the
offline suites cover it and are the gate that matters.

## Adding a test

New behaviour needs a test that exercises the real thing: a real file, a real
socket, a real API. If you find yourself wanting a mock, that is usually a
sign the seam is in the wrong place — open an issue and let us talk about it.

## Versioning

Calendar versioning: `YYYY.M.D`, tagged `vYYYY.M.D` — e.g. `v2026.8.29`.

Note the month and day are **not zero-padded**. Semantic versioning forbids
leading zeros in numeric identifiers, and Cargo enforces it (`2026.08.29` is
rejected outright with "invalid leading zero in minor version number"), so the
crate version could not carry them. The git tag matches the crate version
exactly, and CI fails the release if they ever drift.

There is no semantic promise in the numbers: a later date is a later release,
nothing more. Breaking changes are called out in the release notes.

## Releasing

Tagging `vYYYY.M.D` builds and publishes everything:

1. **GitHub Release** — static musl tarball plus sha256.
2. **PyPI wheels** — a `linux-x86_64` wheel built with
   [`bin-to-wheel`](https://github.com/DataZooDE/bin-to-wheel), vendored at
   `vendor/bin-to-wheel`, so `uvx cicdbar` works with no Rust toolchain.
   Published by OIDC trusted publishing; there is no API token anywhere.
   Clone with `--recurse-submodules`, or run `git submodule update --init`.

Linux-only wheels are deliberate: this is a waybar module talking to D-Bus,
so a macOS or Windows wheel would install a binary that cannot do anything.

AUR is a manual step afterwards; see `packaging/aur/README.md`.
