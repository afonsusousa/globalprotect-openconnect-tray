#!/usr/bin/env bash
# Build openSUSE Build Service sources for gptray and, optionally, upload them.
#
#   obs-release.sh [--out DIR] [--upload home:USER[:SUBPROJECT]]
#
# Works from a standalone gptray checkout; nothing outside this folder is needed.
# Produces in DIR (default: .build/obs inside the gptray folder):
#   gptray-<version>.tar.gz  gptray sources, the prebuilt frontend (dist/) and a
#                            .cargo/config.toml that points cargo at vendor/
#   vendor.tar.xz            all crates (incl. the pinned gpapi/common git deps)
#   gptray.spec              filled in from gptray.spec.in
#   gptray.changes
#
# --upload checks out the package with osc (creating it if needed), replaces its
# files and commits. It uses your own osc credentials (~/.config/osc/oscrc).
set -euo pipefail

usage() { sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

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
OUT="${OUT:-$APP/.build/obs}"

read -r VERSION GP_VERSION < <(python3 -I - "$APP/src-tauri/Cargo.toml" <<'EOF'
import sys, tomllib
manifest = tomllib.load(open(sys.argv[1], "rb"))
tag = manifest["dependencies"]["gpapi"]["tag"]
assert manifest["dependencies"]["common"]["tag"] == tag, "gpapi and common must use the same tag"
print(manifest["package"]["version"], tag.removeprefix("v"))
EOF
)
NAME="gptray-$VERSION"
echo "==> gptray $VERSION (for globalprotect-openconnect $GP_VERSION)"

if git -C "$APP" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
  && [[ -n "$(git -C "$APP" status --porcelain -- .)" ]]; then
  echo "warning: uncommitted changes are included in the package" >&2
fi

echo "==> Building the frontend"
(cd "$APP" && npm ci --no-audit --no-fund --loglevel=error && npm run build >/dev/null)

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
SRC="$STAGE/$NAME"
mkdir -p "$SRC"

echo "==> Staging sources"
tar -C "$APP" \
  --exclude=./node_modules --exclude=./.build --exclude=./dist \
  --exclude=./src-tauri/target --exclude=./src-tauri/gen --exclude=./src-tauri/.cargo \
  -cf - . | tar -C "$SRC" -xf -
cp -r "$APP/dist" "$SRC/dist"

echo "==> Vendoring crates (this takes a while)"
# OBS extracts vendor.tar.xz next to src-tauri/, so "../vendor" resolves from there.
mkdir -p "$SRC/src-tauri/.cargo"
(cd "$SRC/src-tauri" && cargo vendor --locked ../vendor > .cargo/config.toml 2>/dev/null)
grep -q "vendored-sources" "$SRC/src-tauri/.cargo/config.toml" || { echo "cargo vendor produced no source config" >&2; exit 1; }
mv "$SRC/vendor" "$STAGE/vendor"

mkdir -p "$OUT"
rm -f "$OUT"/gptray-*.tar.gz "$OUT/vendor.tar.xz"
tar -C "$STAGE" --owner=0 --group=0 --sort=name -czf "$OUT/$NAME.tar.gz" "$NAME"
# Lay vendor/ out inside the source directory when extracted with %setup -a1.
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
