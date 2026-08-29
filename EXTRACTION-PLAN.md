# Extracting cicdbar to `jrosskopf/cicdbar`

Plan for lifting the widget out of the research repo into a standalone public
project. Agreed 2026-08-29.

## Decisions

| Question | Decision |
|---|---|
| Repo | `jrosskopf/cicdbar`, **public**, MIT (matching `jrosskopf/padctl`) |
| Blacksmith API docs | **First-class document in the repo**, published |
| CI | Build + clippy + fmt only. **No tests in CI** — they need live credentials |
| Distribution | AUR **and** GitHub Releases (prebuilt) **and** crates.io |
| Scope | Config-driven, same two providers (GitHub Actions + Blacksmith) |

The name is free on all three registries — GitHub, crates.io and AUR — so the
project is `cicdbar` everywhere. Verified 2026-08-29.

---

## 1. What moves, and how

Seven commits touch `2026-08-29-cicdbar/` and nothing else outside it, so the
red/green history is worth preserving rather than flattening into an initial
commit — the sequence *is* the argument for how the thing was built.

```sh
git subtree split --prefix=2026-08-29-cicdbar -b cicdbar-export
gh repo create jrosskopf/cicdbar --public \
    --description "waybar widget for GitHub Actions + Blacksmith CI/CD spend and live job status"
git push git@github.com:jrosskopf/cicdbar.git cicdbar-export:main
```

`subtree split` rewrites only that prefix's changes, so the `EXPERIMENTS.md`
edits riding in those commits do not follow.

**Verify before pushing** — the repo becomes public and history is hard to
retract:

```sh
git log --all -p cicdbar-export | grep -nE 'eyJ|remember_web_[a-f0-9]{20}|gho_|ghp_'
```

Must return nothing. (It does today: credentials have only ever lived in
`~/.config/cicdbar/`, and the working tree was checked at each commit.)

**Stays behind** in the research repo: `PLAN.md` and `EXTRACTION-PLAN.md` are
research artefacts about the *process*, and `EXPERIMENTS.md` keeps pointing at
the directory with a note that the code now lives upstream. The directory is
not deleted — the repo is time-ordered and its convention is that entries
persist.

## 2. De-personalising the code

The tool currently assumes my accounts in three places. None is deep.

| Where | Now | After |
|---|---|---|
| `config.example.toml` | `orgs = ["DataZooDE", "anofox"]` | `orgs = ["your-org"]`, with the 403/404 access notes rewritten as general guidance |
| `snapshot.rs::demo()` | `DataZooDE/demo-repo` | `acme/widget-service`, invented figures |
| `tests/*` | real org names | **keep** — they must hit a real org, and mine is the one I can authenticate against. Documented as such |
| `render.rs` palette | One Dark constants | `[theme]` config section, One Dark as default |
| `blacksmith.rs` rates | `BS_BASE_PER_MIN` const | seeded from `config.blacksmith.rates`, published rates as default |

The test-suite point deserves stating plainly in the README rather than
engineered around: **these tests are not runnable by a stranger.** They
authenticate as me against my orgs. A contributor can run the offline suites;
the live ones need their own org, token and config. Pretending otherwise would
mean adding the mocks the project exists to avoid.

## 3. Making `cargo test` honest on a fresh clone

Today `cargo test` on a machine with no `gh` token fails, which is a poor first
impression and would read as a broken project.

Live tests get `#[ignore]` with a reason, so a bare `cargo test` runs the
offline suites and passes:

```rust
#[test]
#[ignore = "hits the live GitHub API; needs a gh token — see run-tests.sh"]
fn fetches_current_month_usage_with_per_repo_granularity() { … }
```

`run-tests.sh` passes `-- --include-ignored` and keeps its existing pacing
between live suites. The README states which suites need what.

This is the one place the extraction changes behaviour rather than packaging,
and it is worth the churn: an ignored test that says why is honest, a red test
on first clone is not.

## 4. Documentation

Four documents, all in-repo:

* **`README.md`** — what it is, install, config, format placeholders, the
  measured request/latency table, and the rate-limit explanation. Mostly
  written already; needs the install matrix and a screenshot.
* **`docs/blacksmith-api.md`** — the captured API, promoted from
  `blacksmith-api-notes.md` and expanded. This is the piece with value beyond
  the tool: Blacksmith publishes no API documentation at all, so this is
  currently the only written description of `dashboardbackend.blacksmith.sh`.
  Covers the endpoint table, the response shapes, and the Laravel cookie-pair
  auth with its rotation behaviour — the thing that makes naive clients
  authenticate exactly once. It documents endpoints and never contains a
  credential, and every call is one the account's own dashboard already makes.
  It should carry a short preamble saying it is unofficial, unaffiliated,
  observed on a stated date, and liable to change without notice.
* **`docs/github-billing.md`** — the two findings worth their own page: the
  `?year=&month=` requirement for per-repo granularity, and the rollup/detail
  storage-discount divergence with the reconciliation guidance.
* **`docs/testing.md`** — why there are no mocks, what each suite touches, the
  five bugs the approach caught, and how to point the live suites at your own
  org.

## 5. Packaging

**Cargo.toml** needs publish metadata it currently lacks: `description`,
`license = "MIT"`, `repository`, `homepage`, `keywords`
(`waybar`, `github-actions`, `ci`, `billing`, `status-bar`), `categories`
(`command-line-utilities`), `readme`, and an explicit `rust-version` MSRV
pinned to what the toolchain here actually is (1.97). Add `LICENSE` (MIT,
"Joachim Rosskopf").

**Static binary.** Build releases against `x86_64-unknown-linux-musl` for a
fully static artefact — no glibc version coupling, one file to drop on any
distro. The dependency tree already cooperates: `reqwest` is on `rustls`, so
there is no OpenSSL to link. `rustup target add x86_64-unknown-linux-musl`.

**Three channels:**

1. **GitHub Releases** — `release.yml` on tag push: build musl, strip, tarball
   with README and LICENSE, checksum, attach.
2. **AUR** — two packages, per Arch convention: `cicdbar` building from the
   release tarball, and `cicdbar-bin` unpacking the prebuilt one. Needs an AUR
   account with an SSH key registered; the `.SRCINFO` is generated by
   `makepkg --printsrcinfo`. Test in a clean chroot before submitting.
3. **crates.io** — `cargo publish`. Requires a token; the crate is a binary,
   so it reaches people via `cargo install cicdbar`.

Releases are cut by tag (`v0.1.0`), and the three channels are updated in that
order, since AUR and crates.io both reference the released tarball.

## 6. CI

`.github/workflows/ci.yml`, on push and PR — **no tests**, by decision:

```yaml
- cargo fmt --check
- cargo clippy --all-targets -- -D warnings
- cargo build --release
- cargo build --release --target x86_64-unknown-linux-musl
```

The last line matters: a musl build break should surface on a PR, not at
release time.

`.github/workflows/release.yml`, on `v*` tags: build musl, package, publish
the GitHub Release.

The consequence, stated so it is a choice and not an oversight: **nothing
automatically catches Blacksmith or GitHub changing their APIs.** The live
suites are the detector and they run when I run them. If this becomes a
problem, the follow-up is a nightly scheduled workflow using repo secrets —
noted, not built.

## 7. Sequence

1. Pre-flight: secret scan of the export branch; confirm the three names are
   still free.
2. De-personalise (§2) and add the `[theme]` / rates config, in the research
   repo, tests staying green — so the extraction moves working code.
3. `#[ignore]` the live tests, update `run-tests.sh`, confirm bare
   `cargo test` passes with no token (test it by unsetting `GH_TOKEN` and
   pointing `GH_CONFIG_DIR` at an empty dir).
4. Cargo.toml metadata + LICENSE.
5. Write `docs/blacksmith-api.md`, `docs/github-billing.md`, `docs/testing.md`.
6. `subtree split`, create the repo, push.
7. Add CI + release workflows; confirm green.
8. Screenshot for the README — needs `grim` installed (currently missing;
   `scrot`/`import` are X11-only and this is Sway).
9. Tag `v0.1.0`, confirm the release artefact runs on a clean machine.
10. `cargo publish`.
11. AUR: PKGBUILD, clean-chroot build, submit both packages.
12. Point the research repo's `EXPERIMENTS.md` entry at the new repo; switch
    the local install to the packaged binary so I am running what I ship.

Steps 1–7 are a session's work. 8–12 each need something outside the code — a
screenshot tool, an AUR account, a crates.io token — so they are checkpoints,
not a continuous run.

## 8. Risks

| Risk | Handling |
|---|---|
| Public repo documents a vendor's undocumented API | Endpoints only, no credentials, no auth bypass; unofficial/unaffiliated disclaimer with an observation date |
| Blacksmith changes the API and users see errors | Provider already degrades to the labelled estimate rather than to a wrong number; docs state the observation date |
| Nobody can run the live tests but me | Stated plainly in `docs/testing.md`; offline suites are the contributor-facing gate |
| AUR package rots | `cicdbar-bin` tracks releases and needs a bump per tag; skip AUR entirely if that upkeep is unwelcome |
| MSRV drift breaks `cargo install` | Explicit `rust-version`, and CI builds release + musl on every PR |
| A 14-day Blacksmith cookie expiring makes the tool look broken | Already handled — explicit "session expired" note in the tooltip and fallback to the estimate; README documents re-capture |
