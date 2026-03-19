#!/usr/bin/env bash
# bump-version.sh — Update all version references from the VERSION file.
#
# Usage:
#   ./scripts/bump-version.sh              # set version from VERSION file
#   ./scripts/bump-version.sh 2026.3.15    # set specific version
#   ./scripts/bump-version.sh patch        # bump to YYYY.M.D-N (increment N)
#
# Version format: YYYY.M.D or YYYY.M.D-N for patches
# Release filenames use: YYYYMMDD or YYYYMMDDN (no separators)

set -euo pipefail
cd "$(dirname "$0")/.."

VERSION_FILE="VERSION"

if [[ $# -ge 1 ]]; then
    if [[ "$1" == "patch" ]]; then
        current=$(cat "$VERSION_FILE" | tr -d '[:space:]')
        if [[ "$current" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-([0-9]+)$ ]]; then
            base="${BASH_REMATCH[1]}"
            n="${BASH_REMATCH[2]}"
            new_version="${base}-$((n + 1))"
        else
            new_version="${current}-1"
        fi
        echo "$new_version" > "$VERSION_FILE"
    elif [[ "$1" == "today" ]]; then
        year=$(date +%Y)
        month=$(date +%-m)
        day=$(date +%-d)
        echo "${year}.${month}.${day}" > "$VERSION_FILE"
    else
        echo "$1" > "$VERSION_FILE"
    fi
fi

VERSION=$(cat "$VERSION_FILE" | tr -d '[:space:]')
echo "Setting version to: $VERSION"

# Cargo workspace version
sed -i "s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml

# tauri.conf.json — app version + Windows MSI version (MSI requires each component ≤ 255)
MSI_VERSION=$(echo "$VERSION" | sed 's/^20//' | sed 's/-[0-9]*//')
python3 -c "
import json, sys
conf = json.load(open('crates/app/tauri.conf.json'))
conf['version'] = '$VERSION'
conf.setdefault('bundle', {}).setdefault('windows', {}).setdefault('wix', {})['version'] = '$MSI_VERSION'
json.dump(conf, open('crates/app/tauri.conf.json', 'w'), indent=2)
print('', file=open('crates/app/tauri.conf.json', 'a'))  # trailing newline
"

# package.json (top-level version field only)
sed -i "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" package.json

echo ""
echo "Updated:"
echo "  VERSION              → $VERSION"
echo "  Cargo.toml           → $VERSION"
echo "  tauri.conf.json      → $VERSION (MSI: $MSI_VERSION)"
echo "  package.json         → $VERSION"
echo ""

# Show release filename format
RELEASE_NAME=$(echo "$VERSION" | sed 's/\.//g; s/-//g')
echo "Release filename stem: aequi-${RELEASE_NAME}"
