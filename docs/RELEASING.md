# Releasing Glass Mixer

How a version goes from the repository to a GitHub release, what is automated,
what is checked by hand, and what code signing needs.

## What is automated

| Step | Where |
| --- | --- |
| Format, lint, unit tests, release build on Windows Server 2022 and the latest runner image | `ci.yml`, on every push and pull request |
| Version consistency (`Cargo.toml`, `Cargo.lock`, `assets/app.manifest`, the installer, the changelog, the tag, the built exe) | `scripts/check-version.ps1`, run by CI, by `package.ps1` and by `release.yml` |
| Third-party license notices | `scripts/third-party-notices.ps1`, run by `package.ps1` |
| Portable ZIP, installer, SHA-256 checksums | `scripts/package.ps1` |
| Package verification (exact contents, version, 64-bit, the embedded font notice, byte-exact licenses, signature state, checksums) | `scripts/verify-package.ps1`, run by `package.ps1` and `release.yml` |
| Release notes from the changelog | `scripts/release-notes.ps1` |
| Everything above, from a clean checkout, ending in a **draft** GitHub release | `release.yml`, on a pushed `v*` tag |
| Installer install, upgrade-over-a-running-copy and uninstall, with the real installation proven untouched | `scripts/installer-smoke.ps1` (by hand: it needs a desktop session) |

Nothing is published automatically: `release.yml` leaves a draft for you to review.

## Releasing a version

1. **Choose the version** and set it everywhere it appears:
   - `version` in `Cargo.toml`, then `cargo check` to update `Cargo.lock`;
   - `assets/app.manifest`, the `assemblyIdentity` version (four parts, for example `0.5.0.0`);
   - `installer/glass-mixer.iss`, the `AppVersion` and `AppVersionNumeric` defaults;
   - `CHANGELOG.md`: a `## <version> (<date>)` heading, with nothing left under `## Unreleased`.

   `scripts\check-version.ps1` tells you if any of them disagree.
2. **Run the checks.** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
   `cargo test --locked`, the real-device tests
   (`cargo test --test audio_integration -- --ignored --test-threads=1`),
   `scripts\lifecycle.ps1` with OpenGL and with `-Renderer wgpu`, and
   `scripts\installer-smoke.ps1`. Record the results in `docs/TESTING.md`, saying what
   was not run.
3. **Build the packages locally** with `scripts\package.ps1`. It writes
   `dist\glass-mixer-<version>-portable.zip`, `…-setup.exe` (when Inno Setup 6 is
   installed) and `…-SHA256SUMS.txt`, and fails if any check fails.
4. **Write the release record** in `docs/releases/<version>.md` (sizes, checksums,
   verification). The checksums of the published files are the ones from the release.
5. **Tag and push:** `git tag v<version>` then `git push origin v<version>`. `release.yml`
   repeats the checks from a clean checkout, builds, signs when configured, verifies, uploads
   the packages as workflow artifacts and creates a **draft** release.
6. **Review the draft**, compare its checksums with `SHA256SUMS.txt`, and publish it from
   the GitHub Releases page.

To rehearse without a tag, run the Release workflow by hand from the Actions tab: it
builds and uploads the packages but creates no release.

## Code signing

Without a certificate, releases are **unsigned**, and the release notes say so: Windows
SmartScreen may warn the first time a download is run. A self-signed certificate does not
change that and is not used.

Signing needs a code-signing certificate from a certificate authority. Windows decides how
much to trust a signed download from the certificate and from the reputation the file has
built up; an extended-validation certificate starts with reputation, others earn it over
time. Signing proves who published the file and that it was not altered; it does not by
itself remove every warning.

**Locally**, install the certificate with its private key in the current user's store and set:

| Variable | Meaning |
| --- | --- |
| `GLASS_MIXER_SIGN_CERT_SHA1` | The certificate's 40-digit SHA-1 thumbprint. Setting it turns signing on. |
| `GLASS_MIXER_SIGNTOOL` | Path of `signtool.exe`, when it is not on `PATH`. |
| `GLASS_MIXER_SIGN_TIMESTAMP_URL` | An RFC 3161 timestamp service, so signatures outlive the certificate. |

`package.ps1` signs the executable before packaging it and the installer after building it,
verifies both with `signtool verify /pa`, and `verify-package.ps1 -RequireSigned` fails if
either is not validly signed. The checksums are written after signing.

**In GitHub Actions**, `release.yml` signs when these exist and is unsigned when they do not:

| Setting | Kind | Meaning |
| --- | --- | --- |
| `WINDOWS_SIGN_PFX_BASE64` | secret | The `.pfx` file, base64 encoded: `[Convert]::ToBase64String([IO.File]::ReadAllBytes('certificate.pfx'))` |
| `WINDOWS_SIGN_PFX_PASSWORD` | secret | The `.pfx` password |
| `WINDOWS_SIGN_TIMESTAMP_URL` | variable | The timestamp service URL |

The workflow imports the certificate into the runner's user store, signs and verifies, and
removes it at the end. Keep the certificate and its password out of the repository.

**Status:** no code-signing certificate is available to this project, so the signing path
has not been exercised end to end. The pieces that do run without one are tested (unsigned
builds are reported as unsigned, and `-RequireSigned` rejects them).

## Windows versions

Hosted runners are Windows Server images (Server 2022, build 20348, and the latest image),
not Windows 10 or 11 client editions, and have no audio hardware or interactive desktop. CI
therefore validates the build, the lints and the unit tests on two Windows generations.
Everything else is a manual check on real machines; `docs/VALIDATION_MATRIX.md` records what
has and has not been run, and on what.
