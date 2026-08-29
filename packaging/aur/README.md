# AUR packaging

Two packages, per Arch convention:

* **`cicdbar`** — builds from the GitHub release tarball. Runs the offline
  test suite in `check()`; the live suites are `#[ignore]`d and need
  credentials, so they cannot run in a build environment.
* **`cicdbar-bin`** — unpacks the prebuilt static musl binary from the same
  release. No Rust toolchain needed.

Both were built and verified locally with `makepkg` before submission.

## Releasing a new version

1. Tag and let `release.yml` publish the GitHub Release.
2. Bump `pkgver` in both PKGBUILDs, reset `pkgrel=1`.
3. Update the checksums:
   * `cicdbar-bin` — the `sha256` from the release assets.
   * `cicdbar` — `updpkgsums`, or `sha256sum` of the archive tarball.
4. Rebuild both: `makepkg -f` (the source package must pass `check()`).
5. Regenerate metadata: `makepkg --printsrcinfo > .SRCINFO`.
6. Commit `PKGBUILD` and `.SRCINFO` to the AUR repo and push.

## First submission

Needs an AUR account with an SSH key registered at
<https://aur.archlinux.org/account/>. Then, per package:

```sh
git clone ssh://aur@aur.archlinux.org/cicdbar.git aur-cicdbar
cp packaging/aur/cicdbar/{PKGBUILD,.SRCINFO} aur-cicdbar/
cd aur-cicdbar && git add -A && git commit -m "Initial import: cicdbar 2026.8.29" && git push
```

Ideally build once in a clean chroot first (`extra-x86_64-build`) to catch
dependencies that happen to be installed on the packaging machine.
