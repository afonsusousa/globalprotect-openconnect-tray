#!/usr/bin/env bash
# Move the tray to a new GlobalProtect-openconnect release.
#
#   bump-upstream.sh [vX.Y.Z]      default: upstream's newest release tag
#
# It:
#   1. points the gpapi/common git dependencies at the new tag (cargo update),
#   2. sets rust-version to the toolchain upstream builds with (its Makefile),
#   3. bumps the tray's patch version (Cargo.toml, tauri.conf.json, package.json),
#   4. adds an entry to packaging/obs/globalprotect-openconnect-tray.changes,
#   5. runs clippy, the tests, the TypeScript check and the frontend build.
#
# It doesn't commit or upload. If a step fails (usually upstream changed an API the
# tray uses), fix the code and run the checks again with: bump-upstream.sh --check
set -euo pipefail

UPSTREAM=https://github.com/yuezk/GlobalProtect-openconnect
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI="$ROOT/src-tauri"
CHANGES="$ROOT/packaging/obs/globalprotect-openconnect-tray.changes"

usage() { sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }
step() { printf '\n==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

check() {
  step "Checking the tray (TypeScript, frontend build, clippy, tests)"
  # The frontend first: the Rust build embeds dist/.
  (cd "$ROOT" && npm ci --no-audit --no-fund --loglevel=error && npm run build) \
    || die "the frontend checks failed. Fix it, then run: $0 --check"
  (cd "$TAURI" && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked) \
    || die "the Rust checks failed; upstream may have changed an API the tray uses. Fix it, then run: $0 --check"
  step "All checks passed"
}

case "${1:-}" in
  -h|--help) usage ;;
  --check) check; exit 0 ;;
esac
[[ $# -le 1 ]] || usage 1

cd "$ROOT"
[[ -z "$(git status --porcelain)" ]] || die "commit or stash your changes first"

current_tag() { sed -n 's/^gpapi *=.*tag *= *"\([^"]*\)".*/\1/p' "$TAURI/Cargo.toml"; }
OLD_TAG="$(current_tag)"
[[ -n "$OLD_TAG" ]] || die "can't find the gpapi tag in src-tauri/Cargo.toml"

if [[ -n "${1:-}" ]]; then
  NEW_TAG="$1"
  [[ "$NEW_TAG" == v* ]] || NEW_TAG="v$NEW_TAG"
  git ls-remote --exit-code --tags "$UPSTREAM" "refs/tags/$NEW_TAG" >/dev/null \
    || die "upstream has no tag $NEW_TAG"
else
  step "Looking up upstream's newest release"
  # Plain vX.Y.Z tags only (no betas or release candidates), highest version first.
  NEW_TAG="$(git ls-remote --tags --refs "$UPSTREAM" 'v*' | sed 's,.*refs/tags/,,' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -1)"
  [[ -n "$NEW_TAG" ]] || die "can't list upstream's tags"
fi

if [[ "$NEW_TAG" == "$OLD_TAG" ]]; then
  echo "Already on $OLD_TAG, nothing to do."
  exit 0
fi
newest="$(printf '%s\n%s\n' "$OLD_TAG" "$NEW_TAG" | sort -V | tail -1)"
[[ "$newest" == "$NEW_TAG" ]] || die "$NEW_TAG is older than the current $OLD_TAG"
echo "Upstream: $OLD_TAG -> $NEW_TAG"

step "Pointing gpapi and common at $NEW_TAG"
sed -i -E "/^(gpapi|common) *=/ s/tag *= *\"$OLD_TAG\"/tag = \"$NEW_TAG\"/" "$TAURI/Cargo.toml"
[[ "$(grep -cE "^(gpapi|common) *=.*tag = \"$NEW_TAG\"" "$TAURI/Cargo.toml")" == 2 ]] \
  || die "couldn't update both tags in src-tauri/Cargo.toml"
(cd "$TAURI" && cargo update -p gpapi -p common)

step "Matching upstream's Rust toolchain"
GPAPI_MANIFEST="$(cd "$TAURI" && cargo metadata --format-version 1 --locked \
  | python3 -I -c 'import json, sys; print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"] == "gpapi"))')"
UPSTREAM_DIR="$(dirname "$(dirname "$(dirname "$GPAPI_MANIFEST")")")"
RUST_VERSION="$(sed -n 's/^RUST_VERSION ?= *//p' "$UPSTREAM_DIR/Makefile")"
OLD_RUST="$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' "$TAURI/Cargo.toml")"
if [[ -n "$RUST_VERSION" && "$RUST_VERSION" != "$OLD_RUST" ]]; then
  sed -i "s/^rust-version *= *\".*\"/rust-version = \"$RUST_VERSION\"/" "$TAURI/Cargo.toml"
  RUST_NOTE="Rust $OLD_RUST -> $RUST_VERSION"
  echo "$RUST_NOTE"
else
  RUST_NOTE=""
  echo "Unchanged (${OLD_RUST})"
fi

step "Bumping the tray's version"
OLD_VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$TAURI/Cargo.toml" | head -1)"
IFS=. read -r major minor patch <<<"$OLD_VERSION"
VERSION="$major.$minor.$((patch + 1))"
sed -i "0,/^version *= *\"$OLD_VERSION\"/s//version = \"$VERSION\"/" "$TAURI/Cargo.toml"
sed -i "s/\"version\": \"$OLD_VERSION\"/\"version\": \"$VERSION\"/" "$TAURI/tauri.conf.json" "$ROOT/package.json"
(cd "$ROOT" && npm install --package-lock-only --no-audit --no-fund --loglevel=error)
# Records the new version in Cargo.lock, so the --locked checks below pass.
(cd "$TAURI" && cargo metadata --format-version 1 >/dev/null)
echo "$OLD_VERSION -> $VERSION"

step "Adding a changelog entry"
{
  printf -- '-------------------------------------------------------------------\n'
  printf '%s - %s <%s>\n\n' "$(LC_ALL=C date -u '+%a %b %e %H:%M:%S UTC %Y' | sed 's/  / /')" \
    "$(git config user.name || echo afonsusousa)" "$(git config user.email || echo afonso.rocha@outlook.com)"
  printf -- '- Update to %s.\n' "$VERSION"
  printf -- '- Update GlobalProtect-openconnect to %s (gpclient, gpservice,\n  gpauth): %s/releases/tag/%s\n' \
    "${NEW_TAG#v}" "$UPSTREAM" "$NEW_TAG"
  [[ -z "$RUST_NOTE" ]] || printf -- '- Build with %s, as upstream does.\n' "$RUST_NOTE"
  printf '\n'
  cat "$CHANGES"
} > "$CHANGES.new"
mv "$CHANGES.new" "$CHANGES"

check

step "Done: $VERSION with GlobalProtect-openconnect ${NEW_TAG#v}"
git -C "$ROOT" --no-pager diff --stat
cat <<EOF

Next:
  1. Read upstream's release notes: $UPSTREAM/releases/tag/$NEW_TAG
  2. Test a real connection: npm run tauri dev (with the matching gpservice installed),
     or upload and install the OBS build.
  3. Edit the changelog entry if needed, then commit:
       git commit -am "Update GlobalProtect-openconnect to $NEW_TAG"
  4. Release: packaging/obs/obs-release.sh --upload home:afonsusousa
EOF
if [[ -n "$RUST_NOTE" ]]; then
  cat <<EOF

Note: upstream now needs Rust $RUST_VERSION. On OBS, an Ubuntu release without
rustc-$RUST_VERSION/cargo-$RUST_VERSION becomes "unresolvable"; drop it or wait for the package.
EOF
fi
