# Releasing spelunk

This document describes how to cut a release of spelunk.

## Overview

Releases are fully automated via GitHub Actions. Pushing a version tag triggers
`.github/workflows/release.yml`, which:

1. Builds `spelunk` and `spelunk-server` release binaries for all supported platforms.
2. Strips binaries where possible to reduce download size.
3. Packages each platform's binaries into a `.tar.gz` archive.
4. Creates a macOS universal binary by combining x86_64 and aarch64 slices with `lipo`.
5. Creates a GitHub Release and attaches all archives as downloadable assets.
6. Auto-generates release notes from merged pull requests and commits.

## Supported platforms

| Target | Runner | Notes |
|--------|--------|-------|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | Native build |
| `aarch64-unknown-linux-gnu` | ubuntu-latest | Cross-compiled via `cross` |
| `x86_64-apple-darwin` | macos-latest | Native build |
| `aarch64-apple-darwin` | macos-latest | Native build (Apple Silicon) |
| `universal-apple-darwin` | macos-latest | Fat binary: x86_64 + aarch64 merged with `lipo` |

## Cutting a release

### 1. Bump the version in `Cargo.toml`

Edit the `version` field in `Cargo.toml`:

```toml
[package]
name = "spelunk"
version = "0.5.0"   # <-- update this
```

### 1a. Update version references in docs

After bumping `Cargo.toml`, update the hardcoded version strings in any docs that reference download URLs. Currently that includes:

- **`docs/getting-started.md`** — five `curl` commands in the Install section each contain `spelunk-v<old>-<target>.tar.gz`; replace the version segment in all of them.

Search for the old version to catch any others:

```bash
grep -r "spelunk-v" docs/
```

Commit everything together:

```bash
git add Cargo.toml Cargo.lock docs/getting-started.md
git commit -m "chore: bump version to 0.5.0"
git push origin main
```

### 2. Tag and push

```bash
git tag v0.5.0
git push origin v0.5.0
```

That's it. The release workflow triggers automatically on the pushed tag.

### 3. Monitor the workflow

Watch progress at:
`https://github.com/usercise/spelunk/actions/workflows/release.yml`

Once all jobs pass, the release appears at:
`https://github.com/usercise/spelunk/releases/tag/v0.5.0`

## Pre-releases

Append a pre-release suffix to the tag. The workflow automatically marks the
GitHub Release as a pre-release when the tag contains `-rc`, `-beta`, or
`-alpha`:

```bash
git tag v0.5.0-rc.1
git push origin v0.5.0-rc.1
```

## Download URLs

After a release is published, assets follow this URL pattern:

```
https://github.com/usercise/spelunk/releases/latest/download/spelunk-<version>-<target>.tar.gz
```

Examples:

```bash
# macOS Apple Silicon
https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.5.0-aarch64-apple-darwin.tar.gz

# macOS universal (x86_64 + Apple Silicon)
https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.5.0-universal-apple-darwin.tar.gz

# Linux x86_64
https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.5.0-x86_64-unknown-linux-gnu.tar.gz

# Linux ARM64
https://github.com/usercise/spelunk/releases/latest/download/spelunk-v0.5.0-aarch64-unknown-linux-gnu.tar.gz
```

## Deleting a bad release

If a release needs to be pulled:

```bash
# Delete the tag locally and on remote
git tag -d v0.5.0
git push origin :refs/tags/v0.5.0

# Delete the GitHub Release (requires gh CLI)
gh release delete v0.5.0 --yes
```

Then fix the issue, re-commit, and re-tag.
