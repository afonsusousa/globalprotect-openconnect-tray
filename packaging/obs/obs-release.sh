#!/usr/bin/env bash
# Build openSUSE Build Service sources for globalprotect-openconnect-tray and,
# optionally, upload them.
#
#   obs-release.sh [--out DIR] [--upload home:USER[:SUBPROJECT]]
#
# Produces in DIR (default: .build/obs):
#   globalprotect-openconnect-tray-<version>.tar.gz
#                       the tray sources, prebuilt frontend (dist/) and a
#                       .cargo/config.toml pointing cargo at vendor/
#   vendor.tar.xz       the tray's crates, vendored for OBS's offline build
#   globalprotect-openconnect-<gp>.tar.gz
#                       upstream GlobalProtect-openconnect at the tag gpapi is pinned
#                       to in src-tauri/Cargo.toml, from upstream's own
#                       `make tarball OFFLINE=1` (incl. submodules and vendored crates)
#   globalprotect-openconnect-tray.spec / .changes
#
# --upload checks out the package with osc (creating it if needed), replaces its
# files and commits. It uses your own osc credentials (~/.config/osc/oscrc).
set -euo pipefail

PACKAGE=globalprotect-openconnect-tray
UPSTREAM=https://github.com/yuezk/GlobalProtect-openconnect

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

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

read -r VERSION GP_TAG < <(python3 -I - "$APP/src-tauri/Cargo.toml" <<'EOF'
import sys, tomllib
manifest = tomllib.load(open(sys.argv[1], "rb"))
tag = manifest["dependencies"]["gpapi"]["tag"]
assert manifest["dependencies"]["common"]["tag"] == tag, "gpapi and common must use the same tag"
print(manifest["package"]["version"], tag)
EOF
)
GP_VERSION="${GP_TAG#v}"
NAME="$PACKAGE-$VERSION"
echo "==> $PACKAGE $VERSION with GlobalProtect-openconnect $GP_VERSION"

if git -C "$APP" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
  && [[ -n "$(git -C "$APP" status --porcelain -- .)" ]]; then
  echo "warning: uncommitted changes are included in the package" >&2
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT"
rm -f "$OUT"/*.tar.gz "$OUT"/*.tar.xz "$OUT"/*.spec "$OUT"/*.changes

echo "==> Upstream: cloning $GP_TAG with submodules"
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$GP_TAG" \
  --recurse-submodules --shallow-submodules "$UPSTREAM" "$STAGE/upstream" 2>&1 \
  | grep -v "is not a commit" || true
echo "==> Upstream: make tarball OFFLINE=1 (vendors its crates; this takes a while)"
make -C "$STAGE/upstream" tarball OFFLINE=1 >/dev/null 2>"$STAGE/upstream-tarball.log" \
  || { cat "$STAGE/upstream-tarball.log" >&2; exit 1; }
cp "$STAGE/upstream/.build/tarball/globalprotect-openconnect-$GP_VERSION.tar.gz" "$OUT/"

echo "==> Tray: building the frontend"
(cd "$APP" && npm ci --no-audit --no-fund --loglevel=error >/dev/null && npm run build >/dev/null)

echo "==> Tray: staging sources"
SRC="$STAGE/$NAME"
mkdir -p "$SRC"
if git -C "$APP" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  # Exactly the tracked files (working-tree contents), nothing local or generated.
  (cd "$APP" && git ls-files -z -- . | xargs -0 cp --parents -t "$SRC")
else
  tar -C "$APP" \
    --exclude=./.git --exclude=./.claude --exclude=./node_modules --exclude=./.build --exclude=./dist \
    --exclude=./target --exclude=./src-tauri/target --exclude=./src-tauri/gen --exclude=./src-tauri/.cargo \
    -cf - . | tar -C "$SRC" -xf -
fi
cp -r "$APP/dist" "$SRC/dist"

echo "==> Tray: vendoring crates"
# OBS extracts vendor.tar.xz next to src-tauri/, so "../vendor" resolves from there.
mkdir -p "$SRC/src-tauri/.cargo"
(cd "$SRC/src-tauri" && cargo vendor --locked ../vendor > .cargo/config.toml 2>"$STAGE/vendor.log") \
  || { grep -v "^\s*Vendoring" "$STAGE/vendor.log" >&2; exit 1; }
grep -q "vendored-sources" "$SRC/src-tauri/.cargo/config.toml" \
  || { echo "cargo vendor produced no source config" >&2; exit 1; }
mv "$SRC/vendor" "$STAGE/vendor"

tar -C "$STAGE" --owner=0 --group=0 --sort=name -czf "$OUT/$NAME.tar.gz" "$NAME"
XZ_OPT="-T0 -6" tar -C "$STAGE" --owner=0 --group=0 --sort=name -cJf "$OUT/vendor.tar.xz" vendor
sed -e "s/@VERSION@/$VERSION/g" -e "s/@GP_VERSION@/$GP_VERSION/g" \
  "$HERE/$PACKAGE.spec.in" > "$OUT/$PACKAGE.spec"
cp "$HERE/$PACKAGE.changes" "$OUT/$PACKAGE.changes"

echo "==> OBS sources in $OUT:"
ls -lh "$OUT"

if [[ -n "$PROJECT" ]]; then
  command -v osc >/dev/null || { echo "osc is not installed (sudo zypper in osc)" >&2; exit 1; }
  WC="$(mktemp -d)"
  trap 'rm -rf "$STAGE" "$WC"' EXIT
  echo "==> Uploading to $PROJECT/$PACKAGE"
  if ! osc -q meta pkg "$PROJECT" "$PACKAGE" >/dev/null 2>&1; then
    osc meta pkg "$PROJECT" "$PACKAGE" -F - <<EOF
<package name="$PACKAGE" project="$PROJECT">
  <title>$PACKAGE</title>
  <description>GlobalProtect VPN client (open-source GlobalProtect-openconnect components built from source) with a tray GUI. VIBE CODED: AI-written with minimal human review; use at your own risk.</description>
</package>
EOF
  fi
  (cd "$WC" && osc -q checkout "$PROJECT" "$PACKAGE")
  PKG="$WC/$PROJECT/$PACKAGE"
  find "$PKG" -maxdepth 1 -type f ! -name '_*' -delete
  cp "$OUT"/* "$PKG/"
  (cd "$PKG" && osc addremove && osc commit -m "Update to $PACKAGE $VERSION (GlobalProtect-openconnect $GP_VERSION)")
  echo "==> Committed. Follow the build with: osc results $PROJECT $PACKAGE"
fi
