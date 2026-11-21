# Releasing

Releases are cut from `main` by pushing a tag. The
[release workflow](../.github/workflows/release.yml) does the rest.

## What a release contains

| File | Contents |
|---|---|
| `cross-control-VERSION-x86_64-unknown-linux-gnu.tar.gz` | `cross-control`, README, CHANGELOG, licences, `config.example.toml` |
| `cross-control-VERSION-aarch64-unknown-linux-gnu.tar.gz` | the same, for 64-bit ARM (Raspberry Pi 4/5 with a 64-bit OS, ARM servers) |
| `cross-control-VERSION-x86_64-pc-windows-msvc.zip` | `cross-control.exe`, `cross-controlw.exe`, the `CrossControl` PowerShell module, and the same documents |
| `cross-control-VERSION-aarch64-apple-darwin.tar.gz`, `…-x86_64-apple-darwin.tar.gz` | `cross-control` for Apple silicon and Intel Macs, and the same documents |
| `cross-control_VERSION_amd64.deb`, `…_arm64.deb` | Debian / Ubuntu packages: `/usr/bin/cross-control`, a udev rule giving the `input` group `/dev/uinput`, and `uinput` loaded at boot |
| `cross-control-VERSION-1.x86_64.rpm`, `…aarch64.rpm` | the same, for Fedora / RHEL / openSUSE |
| `SHA256SUMS` | SHA-256 of every file above |

Linux binaries are built on Ubuntu 22.04, so they run on any distribution
with glibc 2.35 or newer.

Every archive also gets a signed
[build provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations).
It records that this repository's workflow built the archive, from which
commit. To check one:

```bash
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify cross-control-0.3.0-x86_64-unknown-linux-gnu.tar.gz --repo Adjoint-uk/cross-control
```

## Cutting a release

1. Bump the version everywhere it appears: the workspace `Cargo.toml`
   (package and internal dependencies), `Cargo.lock`, and the PowerShell
   module manifest. Also open the new CHANGELOG section:

   ```bash
   scripts/release/release.py bump 0.3.0
   ```

   This inserts `## [0.3.0] - DATE` under `## [Unreleased]`. Move the
   unreleased entries under it, tidy them, and leave `[Unreleased]` empty.

2. Open a PR with the bump. CI and a dry run of the release workflow (every
   build, packaging and installer test, no publishing) must pass. Merge it.

3. Tag the merge commit and push the tag:

   ```bash
   git switch main && git pull
   git tag -a v0.3.0 -m "cross-control 0.3.0"
   git push origin v0.3.0
   ```

   Or, without pushing a tag: in the Actions tab, run the **Release**
   workflow on `main` with **publish** ticked. The workflow then creates the
   tag `vX.Y.Z` itself, for the version in the code, on the commit it built.
   It refuses if that tag already exists or the run is not on `main`.

The workflow then checks that the tag matches the version in the code, and
refuses to release if it does not. It also checks that the CHANGELOG has a
section for it, which becomes the release notes. It builds and smoke-tests
every archive: it runs the binary, installs it with `install.sh` /
`install.ps1` from a local mirror, and loads the bundled module. Finally it
attests the archives and publishes the release.

It also builds the `.deb` and `.rpm` packages (with
[nfpm](https://nfpm.goreleaser.com)) and installs and removes each on Ubuntu
and in a Fedora container. Before publishing, it installs the Homebrew formula
and the Scoop manifest from the local mirror. After publishing, it opens a
pull request pointing `Formula/cross-control.rb` (the Homebrew tap) and
`bucket/cross-control.json` (the Scoop bucket) at the new release. **Merge
that pull request:** Homebrew and Scoop users get the new version from it. If
the repository does not allow Actions to open pull requests, the step says so
and you can generate both files yourself:

```bash
curl -fsSLO https://github.com/Adjoint-uk/cross-control/releases/download/v0.3.0/SHA256SUMS
scripts/release/release.py homebrew --version 0.3.0 --sums SHA256SUMS > Formula/cross-control.rb
scripts/release/release.py scoop --version 0.3.0 --sums SHA256SUMS > bucket/cross-control.json
``` Versions with a pre-release
suffix (`0.3.0-alpha.1`) are marked as pre-releases.

`scripts/release/release.py version` (run in CI) fails if the version
locations disagree, so a half-done bump cannot ship.

## Dry runs

The workflow runs without publishing on any PR that touches it,
`scripts/release/`, or the install scripts. It can also be started manually
from the Actions tab ("Run workflow"). The archives are kept as workflow
artifacts for inspection.
