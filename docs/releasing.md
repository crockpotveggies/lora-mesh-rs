# Building and publishing releases

## Local packages

Use Rust 1.95.0 and Python 3.11+ (CI pins Python 3.12). The Rust source MSRV remains
1.85. Native builds use `Cargo.lock`; no legacy or unauthenticated lab daemon is
included. Build on the target OS/architecture:

```sh
python3 scripts/package-release.py
# Or build only a portable archive:
python3 scripts/package-release.py --portable-only
```

Native prerequisites:

- macOS: Xcode command-line tools (`pkgbuild`, `hdiutil`, `codesign`).
- Linux: `pkg-config`, `libudev-dev`, `dpkg-deb`; iproute2 for actual TUN operation.
- Windows: MSVC Rust toolchain, Visual Studio C++ build tools, Inno Setup 6.
  Packaged EXEs statically link the C runtime using Rust’s [crt-static option](https://doc.rust-lang.org/reference/linkage.html).
  Set `ISCC` to the compiler path if it is not in its standard location or PATH.

Outputs under `dist/` have version and Rust target names. Each installer/archive
and build JSON has its own SHA-256 sidecar. Portable payload manifests cover every
shipped file. Third-party license files/index come from resolved Cargo packages;
`Cargo.lock`, the project license and revision are included. Packages never read
from deployment key directories. Local uncommitted builds have `dirty: true` and
are refused by the release publisher.

`--skip-build` is for packaging existing **native** release binaries: the packager
checks each tool's `--version`. Windows release binaries are read from
`target/x86_64-pc-windows-msvc/release` (built with static CRT); Unix binaries
come from `target/release`. It is not a cross-compilation switch. Output archives
are not claimed to be bit-for-bit reproducible across platforms, compilers, signing
timestamps or native installer tools.

## CI and tag releases

`.github/workflows/test.yml` runs on branch pushes and pull requests. It calls:

1. `checks.yml`: tests, formatting/Clippy on Linux x86_64/ARM64, macOS Apple Silicon/
   Intel and Windows x86_64; Rust 1.85 check; Linux namespace/TUN tests; coverage,
   fuzzing and dependency advisories. Unix-only device/filesystem tests are gated,
   while parser, crypto, routing and deterministic simulation tests run on Windows.
2. `packages.yml`: builds native formats on all five runners, verifies archive hashes
   and shipped simulator replay, and tests install → reinstall/upgrade → run →
   uninstall. CI installers run only on disposable runners. Artifacts remain
   downloadable from the workflow for 14 days. PR builds have no signing secrets
   or release-write token.

A pushed `vVERSION` tag triggers `release.yml`. The tag must exactly match
`Cargo.toml` (e.g. version `0.2.0` requires tag `v0.2.0`; `0.2.0-rc.1` is allowed).
The pipeline runs the same checks on the tagged commit before building installers.
The publish job receives `contents: write`; other jobs have read-only permissions.

To release, first update `Cargo.toml`, refresh the root/fuzz lockfiles if the version
or dependencies changed, and commit all intended source, packaging and workflow
files. For example, after setting version 0.2.0:

```sh
cargo check
cargo check --manifest-path fuzz/Cargo.toml
python3 scripts/package-release.py --check-version --tag v0.2.0
# Commit the reviewed work and push its branch first.
git tag -a v0.2.0 -m 'LoRa Mesh 0.2.0'
git push origin v0.2.0
```

No tags are created by the workflow or packager. Publication verifies the complete
five-platform artifact set, checksums, version, target, clean checkout and exact
commit. It adds a source archive from `git archive HEAD`, creates a draft GitHub
release, uploads all assets, verifies remote asset names/sizes, then publishes.
Any failed upload leaves a draft. Rerunning the workflow resumes that draft;
published releases are never overwritten. A version containing a prerelease suffix
is marked as a GitHub prerelease. Use a new version/tag to replace a published build.
There is no automatic package registry upload or deployment.

The repo must allow GitHub Actions to create releases with `GITHUB_TOKEN`.
Protect release tags in repository rulesets so only authorized maintainers can
publish. No personal access token is required. Source archives and every platform's
build metadata/limitations appear alongside installers in the GitHub release.

## Optional macOS signing and notarization

Unsigned release builds work without secrets. To enable signing, set repository
variable `MACOS_SIGNING_ENABLED=true` and configure these Actions secrets:

| Secret | Value |
| --- | --- |
| `MACOS_CERTIFICATE_P12_BASE64` | Base64 PKCS#12 export containing Developer ID Application **and** Developer ID Installer certificates/private keys |
| `MACOS_CERTIFICATE_PASSWORD` | PKCS#12 export password |
| `MACOS_SIGN_IDENTITY` | Full Developer ID Application identity name |
| `MACOS_INSTALLER_IDENTITY` | Full Developer ID Installer identity name |
| `APPLE_ID` | Apple developer account email |
| `APPLE_TEAM_ID` | Developer team ID |
| `APPLE_APP_PASSWORD` | App-specific password for notarization |

A disposable keychain is created per runner. Binaries use hardened-runtime signing;
PKGs use Installer signing; the PKG is notarized/stapled before inclusion in the
DMG, then the DMG is signed/notarized/stapled too. An `always()` cleanup step removes
the keychain and restores the search list. Missing credentials or a signing/notary
failure fails the release instead of silently publishing an unsigned fallback.
Certificates/private keys are never uploaded as artifacts. Signing has not been
performed locally without publisher credentials.

For local signing, provide `MACOS_SIGN_IDENTITY`, `MACOS_INSTALLER_IDENTITY` and
`MACOS_NOTARY_PROFILE` for identities/credentials already in your keychain;
`MACOS_KEYCHAIN` optionally selects a keychain for notarytool. Windows Authenticode
signing is not currently configured; Windows releases explicitly report unsigned.

References: [Apple notarization](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow),
[Inno Setup compiler](https://jrsoftware.org/ishelp/topic_compilercmdline.htm),
[GitHub release CLI](https://cli.github.com/manual/gh_release_create),
[runner platforms](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

## Verification commands

```sh
python3 -m unittest discover -s tests/packaging -v
python3 scripts/verify-package.py dist/loramesh-VERSION-TARGET.tar.gz
python3 scripts/verify-macos-installer.py dist/  # Mac; read-only mount/expand
cargo check --locked --all-targets --target x86_64-pc-windows-gnu
```

The last command needs `rustup target add x86_64-pc-windows-gnu`; it checks Windows
compilation from Unix and does not replace native Windows execution. The lifecycle
runner `scripts/installer-smoke.py` deliberately requires `CI=true`; do not use it
on your workstation, because it installs/uninstalls system packages on Unix.
