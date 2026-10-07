#!/bin/sh
# Builds the standalone release bundle: OUTDIR/tayga-standalone-VERSION.tar.gz
# (deploy/standalone plus install.sh, with TAYGA_VERSION pinned in .env.example)
# and its .sha256 file. Used by .github/workflows/release.yml.
#
#   sh scripts/make-bundle.sh 0.1.0 dist
set -eu

[ "$#" -eq 2 ] || { echo "usage: make-bundle.sh VERSION OUTDIR" >&2; exit 2; }
VERSION="${1#v}"
OUT="$2"
case "$VERSION" in
'' | *[!0-9A-Za-z.+-]*) echo "make-bundle.sh: bad version: $1" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NAME="tayga-standalone-$VERSION"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT INT TERM

mkdir -p "$STAGE/$NAME" "$OUT"
for f in compose.yaml .env.example notifier.toml otel-collector.yaml README.md; do
	cp "$ROOT/deploy/standalone/$f" "$STAGE/$NAME/$f"
done
cp "$ROOT/scripts/install.sh" "$STAGE/$NAME/install.sh"
chmod 755 "$STAGE/$NAME/install.sh"
sed "s/^TAYGA_VERSION=.*/TAYGA_VERSION=$VERSION/" "$ROOT/deploy/standalone/.env.example" >"$STAGE/$NAME/.env.example"
grep -q "^TAYGA_VERSION=$VERSION\$" "$STAGE/$NAME/.env.example"

tar -C "$STAGE" -czf "$OUT/$NAME.tar.gz" "$NAME"
(
	cd "$OUT"
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$NAME.tar.gz" >"$NAME.tar.gz.sha256"
	else
		shasum -a 256 "$NAME.tar.gz" >"$NAME.tar.gz.sha256"
	fi
)
echo "$OUT/$NAME.tar.gz"
