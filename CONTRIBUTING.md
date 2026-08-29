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
