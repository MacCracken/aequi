# Versioning

## Format

Aequi uses **calendar versioning (CalVer):**

```
YYYY.M.D        — release on a given date
YYYY.M.D-N      — Nth patch to a release (1-indexed)
```

Examples:
- `2026.3.10` — released March 10, 2026
- `2026.3.10-1` — first patch to the March 10 release
- `2026.3.10-2` — second patch

### Release filenames

For release artifacts (binaries, archives), separators are removed to avoid issues with filesystem and URL handling:

```
YYYYMMDD        — e.g., aequi-20260310-linux-x64.tar.gz
YYYYMMDDN       — e.g., aequi-202603101-linux-x64.tar.gz (patch 1)
```

## Source of truth

The **`VERSION`** file in the repository root is the single source of truth. All other version references are derived from it:

| File | Field |
|---|---|
| `VERSION` | Source of truth |
| `Cargo.toml` | `[workspace.package] version` |
| `crates/app/tauri.conf.json` | `"version"` |
| `package.json` | `"version"` |

## Updating the version

Use the bump script:

```sh
# Set version from VERSION file (sync all references)
./scripts/bump-version.sh

# Set a specific version
./scripts/bump-version.sh 2026.4.1

# Set to today's date
./scripts/bump-version.sh today

# Increment patch number (2026.3.10 → 2026.3.10-1 → 2026.3.10-2)
./scripts/bump-version.sh patch
```

The script updates `VERSION`, `Cargo.toml`, `tauri.conf.json`, and `package.json` in one step.

## Changelog

Each release is documented in `CHANGELOG.md` with the version and date. The changelog follows the [Keep a Changelog](https://keepachangelog.com/) format.
