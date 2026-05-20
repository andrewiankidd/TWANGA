# Distribution

How TWANGA ships, what's signed, and why some downloads show OS
warnings. The CI side lives in [`.github/workflows/release.yml`](../.github/workflows/release.yml);
this doc is the human-facing summary.

## What we ship

Every push to `main` refreshes the rolling [`latest-main`](https://github.com/andrewiankidd/TWANGA/releases/tag/latest-main)
pre-release with the full set of artefacts. Tagged `v*` pushes
produce a draft versioned release on top.

| Component | Windows | macOS (universal) | Linux | Android | iOS |
|-----------|---------|-------------------|-------|---------|-----|
| CLI binary | `twanga-cli-windows.zip` | `twanga-cli-macos.tar.gz` | `twanga-cli-linux.tar.gz` | — | — |
| Desktop installer | `twanga-desktop-windows-setup.msi` + `…-setup.exe` (NSIS) | `twanga-desktop-macos-setup.dmg` | `twanga-desktop-linux-setup.deb` | — | — |
| Desktop portable | `twanga-desktop-windows-portable.zip` | `twanga-desktop-macos-portable.app.tar.gz` | `twanga-desktop-linux-portable.AppImage` | — | — |
| Mobile | — | — | — | `twanga-mobile-android.apk` | `twanga-mobile-ios-simulator.app.tar.gz` |

macOS artefacts are universal — same file runs natively on both
Apple Silicon and Intel.

## Signing status

| Platform | Today | Warning user sees | What would change it |
|----------|-------|-------------------|----------------------|
| Windows | Unsigned `.exe` / `.msi` | SmartScreen "Microsoft Defender prevented an unrecognised app from starting" — click "More info" → "Run anyway" | A code-signing cert ($300–700/yr, EV cert ~$400+) or built-up SmartScreen reputation over many downloads |
| macOS | Ad-hoc signed (`signingIdentity = "-"`), not notarised | "TWANGA cannot be opened because Apple cannot check it for malicious software" — workaround instructions ship inside the DMG + portable tarball as `README-FIRST.txt` | Apple Developer Program ($99/yr) — gives a Developer ID for codesigning + the notarisation API |
| Linux | Unsigned, `.deb` and `.AppImage` standard | None — Linux package tooling doesn't gate on signing by default | Optional: GPG-signed `.deb` for repo distribution |
| Android | Debug-signed (Tauri auto-generated debug keystore via `--debug`) | None for sideload; rejected by Play Store | A release keystore in repo secrets + `tauri.conf.json` signing config |
| iOS | Simulator-only `.app` (no `.ipa`) | Won't install on a real device | Apple Developer Program — same as macOS notarisation; need provisioning profiles + signing certs |

The `MACOS-README.txt` instructions live at
[`crates/twanga-app/dist/MACOS-README.txt`](../crates/twanga-app/dist/MACOS-README.txt).

## How releases fire

```
push to main          → CI + Release + Pages
push tag v*           → Release only (draft versioned release)
workflow_dispatch     → Release with manually-entered tag
other branches / PRs  → CI only (no Release, no Pages)
```

Two aggregators in `release.yml`:

- **`release-rolling`** (main pushes) — overwrites `latest-main`
  in place with the freshly-built artefacts. Marked
  `prerelease: true` and `make_latest: false` so it doesn't
  displace the "Latest" badge that future `v*` tags will own.
- **`release-tag`** (tag pushes + dispatch) — creates a draft
  versioned release for review before publishing.

Both depend on `build-platform`, `build-android`, and `build-ios`
with `if: always() && needs.build-platform.result == 'success'` —
desktop must succeed; mobile is best-effort.

Concurrency: `release-${{ github.ref }}` with
`cancel-in-progress: true`, so a newer main push cancels the
older in-flight matrix run instead of stacking 10-job builds.

## Cutting a versioned release

```bash
# 1. Update CHANGELOG: move [Unreleased] → [vX.Y.Z] - YYYY-MM-DD.
# 2. Bump workspace `version` in Cargo.toml.
git commit -am "chore: bump to vX.Y.Z"
git tag -a vX.Y.Z -m "TWANGA vX.Y.Z"
git push origin main vX.Y.Z
```

The tag push triggers `release-tag`, which builds + uploads to a
new **draft** release. Review the artefacts on the Releases page,
edit the auto-generated release notes if needed, then click
"Publish release". The first non-prerelease release becomes the
target of `releases/latest/download/<file>` URLs.

## Future work

- **Apple Developer ID + notarisation** for macOS — would let the
  app launch without the Gatekeeper warning. ~$99/yr; signing +
  notarisation steps in CI.
- **Android release keystore** in repo secrets — would produce
  Play-Store-acceptable APKs/AABs.
- **iOS signing pipeline** — provisioning profiles + certs in repo
  secrets; would produce installable `.ipa` files.
- **Windows EV certificate** — gets past SmartScreen on day one;
  cheaper alternative is "build reputation" by signing with a
  standard OV cert and accumulating downloads.

All deferred until there's user demand justifying the recurring
cost.

## See also

- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — the build matrix.
- [`.github/workflows/pages.yml`](../.github/workflows/pages.yml) — Pages deploy.
- [`docs/features/hardware.md`](features/hardware.md) — what users actually run on.
- [`CHANGELOG.md`](../CHANGELOG.md) — full shipped-feature history.
