#!/usr/bin/env bash
# Build openSUSE Build Service sources (RPM and Debian/Ubuntu) for
# globalprotect-openconnect-tray and, optionally, upload them.
#
#   obs-release.sh [--out DIR] [--upload home:USER[:SUBPROJECT]]
#
# Produces in DIR (default: .build/obs):
#   globalprotect-openconnect-tray-<version>.tar.gz   the one source tarball:
#       the tray (tracked files) with its prebuilt frontend (dist/),
#       vendor/ + src-tauri/.cargo/config.toml   the tray's crates for offline builds,
#       upstream/globalprotect-openconnect-<gp>.tar.gz
#           upstream GlobalProtect-openconnect at the tag gpapi is pinned to in
#           src-tauri/Cargo.toml, from upstream's own `make tarball OFFLINE=1`
#           (incl. OpenConnect/libxml2 submodules and its vendored crates)
#   globalprotect-openconnect-tray.spec / .changes    RPM (openSUSE)
#   globalprotect-openconnect-tray.dsc, debian.*      Debian/Ubuntu
#
# --upload checks out the package with osc (creating it if needed), replaces its
# files and commits. It uses your own osc credentials (~/.config/osc/oscrc).
set -euo pipefail

PACKAGE=globalprotect-openconnect-tray
UPSTREAM=https://github.com/yuezk/GlobalProtect-openconnect

usage() { sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

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

in_git() { git -C "$APP" rev-parse --is-inside-work-tree >/dev/null 2>&1; }
if in_git && [[ -n "$(git -C "$APP" status --porcelain -- .)" ]]; then
  echo "warning: uncommitted changes are included in the package" >&2
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
SRC="$STAGE/$NAME"
mkdir -p "$SRC/upstream" "$OUT"
rm -f "$OUT"/*.tar.gz "$OUT"/*.tar.xz "$OUT"/*.spec "$OUT"/*.changes "$OUT"/*.dsc "$OUT"/debian.*

echo "==> Upstream: cloning $GP_TAG with submodules"
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$GP_TAG" \
  --recurse-submodules --shallow-submodules "$UPSTREAM" "$STAGE/upstream" 2>&1 \
  | grep -v "is not a commit" || true
# The Rust toolchain upstream builds with (Ubuntu's versioned rustc-X.Y packages).
RUST_VERSION="$(sed -n 's/^RUST_VERSION ?= *//p' "$STAGE/upstream/Makefile")"
echo "==> Upstream: make tarball OFFLINE=1 (vendors its crates; this takes a while)"
make -C "$STAGE/upstream" tarball OFFLINE=1 >/dev/null 2>"$STAGE/upstream-tarball.log" \
  || { cat "$STAGE/upstream-tarball.log" >&2; exit 1; }
mv "$STAGE/upstream/.build/tarball/globalprotect-openconnect-$GP_VERSION.tar.gz" "$SRC/upstream/"

echo "==> Tray: building the frontend"
(cd "$APP" && npm ci --no-audit --no-fund --loglevel=error >/dev/null && npm run build >/dev/null)

echo "==> Tray: staging sources"
if in_git; then
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
mkdir -p "$SRC/src-tauri/.cargo"
(cd "$SRC/src-tauri" && cargo vendor --locked ../vendor > .cargo/config.toml 2>"$STAGE/vendor.log") \
  || { grep -v "^\s*Vendoring" "$STAGE/vendor.log" >&2; exit 1; }
grep -q "vendored-sources" "$SRC/src-tauri/.cargo/config.toml" \
  || { echo "cargo vendor produced no source config" >&2; exit 1; }

echo "==> Packing"
tar -C "$STAGE" --owner=0 --group=0 --sort=name -czf "$OUT/$NAME.tar.gz" "$NAME"

subst() {
  sed -e "s/@VERSION@/$VERSION/g" -e "s/@GP_VERSION@/$GP_VERSION/g" -e "s/@RUST_VERSION@/$RUST_VERSION/g" "$@"
}
# RPM
subst "$HERE/$PACKAGE.spec.in" > "$OUT/$PACKAGE.spec"
cp "$HERE/$PACKAGE.changes" "$OUT/$PACKAGE.changes"
# Debian: OBS turns debian.<name> into debian/<name> and builds from the .dsc.
subst "$HERE/debian/control.in" > "$OUT/debian.control"
subst "$HERE/debian/rules.in" > "$OUT/debian.rules"
cp "$HERE/debian/postrm" "$OUT/debian.postrm"
cp "$HERE/debian/copyright" "$OUT/debian.copyright"
BUILD_DEPENDS="$(sed -n '/^Build-Depends:/,/^[A-Z][a-zA-Z-]*:/p' "$OUT/debian.control" \
  | sed '$d' | sed 's/^Build-Depends://' | tr -d '\n' | sed 's/  */ /g; s/^ //')"
subst "$HERE/debian/dsc.in" | sed "s|@BUILD_DEPENDS@|$BUILD_DEPENDS|" > "$OUT/$PACKAGE.dsc"
python3 -I - "$HERE/$PACKAGE.changes" "$VERSION" > "$OUT/debian.changelog" <<'EOF'
# The newest .changes entry, as a Debian changelog entry.
import email.utils, re, sys
from datetime import datetime
text = open(sys.argv[1]).read()
entry = text.split("-" * 67)[1].strip().splitlines()
date = datetime.strptime(re.sub(r"\s+", " ", entry[0].split(" - ")[0]).replace(" UTC", ""), "%a %b %d %H:%M:%S %Y")
author = entry[0].split(" - ", 1)[1]
body = [("  * " + line[2:]) if line.startswith("- ") else "    " + line.strip() for line in entry[1:] if line.strip()]
print(f"globalprotect-openconnect-tray ({sys.argv[2]}-1) unstable; urgency=medium\n")
print("\n".join(body))
print(f"\n -- {author}  {email.utils.format_datetime(date.replace(tzinfo=__import__('datetime').timezone.utc))}")
EOF

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
