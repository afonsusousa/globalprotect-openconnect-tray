#!/usr/bin/env bash
# Build openSUSE Build Service sources for gptray and, optionally, upload them.
#
#   obs-release.sh [--out DIR] [--upload home:USER[:SUBPROJECT]]
#
# Produces in DIR (default: apps/gptray/.build/obs):
#   gptray-<version>.tar.gz  trimmed sources: gptray, crates/gpapi, crates/common,
#                            the prebuilt frontend (dist/) and a minimal root Cargo.toml
#   vendor.tar.xz            all crates vendored for OBS's offline build
#   gptray.spec              filled in from gptray.spec.in
#   gptray.changes
#
# --upload checks out the package with osc (creating it if needed), replaces its
# files and commits. It uses your own osc credentials (~/.config/osc/oscrc).
set -euo pipefail

usage() { sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

OUT=""
PROJECT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --upload) PROJECT="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) echo "Unknown argument: $1" >&2; usage 1 ;;
  esac
done

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="$(cd "$HERE/../.." && pwd)"
ROOT="$(cd "$APP/../.." && pwd)"
OUT="${OUT:-$APP/.build/obs}"

manifest_version() { # file section
  python3 -I - "$1" "$2" <<'EOF'
import sys, tomllib
data = tomllib.load(open(sys.argv[1], "rb"))
for key in sys.argv[2].split("."):
    data = data[key]
print(data["version"])
EOF
}
VERSION="$(manifest_version "$APP/src-tauri/Cargo.toml" package)"
GP_VERSION="$(manifest_version "$ROOT/Cargo.toml" workspace.package)"
NAME="gptray-$VERSION"
echo "==> gptray $VERSION (for globalprotect-openconnect $GP_VERSION)"

if [[ -n "$(git -C "$ROOT" status --porcelain -- apps/gptray crates/gpapi crates/common)" ]]; then
  echo "warning: uncommitted changes are included in the package" >&2
fi

echo "==> Building the frontend"
(cd "$APP" && npm ci --no-audit --no-fund --loglevel=error && npm run build >/dev/null)

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
SRC="$STAGE/$NAME"
mkdir -p "$SRC"

echo "==> Staging sources"
# Tracked files only (no target/, node_modules/ or local state), plus the built frontend.
(cd "$ROOT" && git ls-files -z -- apps/gptray crates/gpapi crates/common | xargs -0 cp --parents -t "$SRC")
cp -r "$APP/dist" "$SRC/apps/gptray/dist"

# gpapi/common inherit workspace settings; keep a root manifest with just those members,
# so the tarball doesn't need the rest of the repository (or its git-only patches).
python3 -I - "$ROOT/Cargo.toml" "$SRC/Cargo.toml" <<'EOF'
import re, sys
text = open(sys.argv[1]).read()
text = re.sub(r"members\s*=\s*\[[^\]]*\]", 'members = ["crates/gpapi", "crates/common"]', text, count=1)
text = re.sub(r"\n\[patch\.crates-io\].*?(?=\n\[|\Z)", "\n", text, flags=re.S)
open(sys.argv[2], "w").write(text)
EOF

echo "==> Vendoring crates (this takes a while)"
(cd "$SRC/apps/gptray/src-tauri" && cargo vendor --locked --quiet "$STAGE/vendor" >/dev/null)

mkdir -p "$OUT"
rm -f "$OUT"/gptray-*.tar.gz "$OUT/vendor.tar.xz"
tar -C "$STAGE" --owner=0 --group=0 --sort=name -czf "$OUT/$NAME.tar.gz" "$NAME"
XZ_OPT="-T0 -6" tar -C "$STAGE" --owner=0 --group=0 --sort=name -cJf "$OUT/vendor.tar.xz" vendor
sed -e "s/@VERSION@/$VERSION/g" -e "s/@GP_VERSION@/$GP_VERSION/g" "$HERE/gptray.spec.in" > "$OUT/gptray.spec"
cp "$HERE/gptray.changes" "$OUT/gptray.changes"

echo "==> OBS sources in $OUT:"
ls -lh "$OUT"

if [[ -n "$PROJECT" ]]; then
  command -v osc >/dev/null || { echo "osc is not installed (sudo zypper in osc)" >&2; exit 1; }
  WC="$(mktemp -d)"
  trap 'rm -rf "$STAGE" "$WC"' EXIT
  echo "==> Uploading to $PROJECT/gptray"
  if ! osc -q meta pkg "$PROJECT" gptray >/dev/null 2>&1; then
    osc meta pkg "$PROJECT" gptray -F - <<EOF
<package name="gptray" project="$PROJECT">
  <title>gptray</title>
  <description>Tray GUI for GlobalProtect-openconnect</description>
</package>
EOF
  fi
  (cd "$WC" && osc -q checkout "$PROJECT" gptray)
  PKG="$WC/$PROJECT/gptray"
  find "$PKG" -maxdepth 1 -type f ! -name '_*' -delete
  cp "$OUT"/* "$PKG/"
  (cd "$PKG" && osc addremove && osc commit -m "Update to gptray $VERSION")
  echo "==> Committed. Follow the build with: osc results $PROJECT gptray"
fi
