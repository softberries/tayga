#!/bin/sh
# Tayga installer: runs the standalone Docker Compose stack (Tayga, ClickHouse, Redpanda).
#
#   curl -fsSL https://raw.githubusercontent.com/softberries/tayga/master/scripts/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version 0.1.0 --dir ~/tayga
#
# Re-running it is safe: it updates the settings it manages in <dir>/.env and
# runs `docker compose up -d` again. Run with --help for the options.
set -eu

REPO="softberries/tayga"
VERSION=""
DIR="${TAYGA_DIR:-$HOME/tayga}"
PROJECT=""
OFFSET=""
BIND=""
LOCAL=0
UNINSTALL=0
PURGE=0
WAIT_SECS=300

usage() {
	cat <<'EOF'
Usage: install.sh [options]

Installs and starts Tayga with Docker Compose, waits until it is healthy, and
prints the URLs and an OpenTelemetry Collector snippet.

Options:
  --version V     Release to install (0.1.0 or v0.1.0). Default: the latest release.
  --dir DIR       Install directory. Default: $TAYGA_DIR, else ~/tayga.
  --project NAME  Docker Compose project name. Default: tayga (kept in DIR/.env).
  --ports N       Add N to every published host port (8090, 4317, 4318),
                  for example --ports 10000 gives 18090, 14317, 14318.
  --bind ADDR     Address the published ports bind to. Default: 127.0.0.1
                  (this machine only). 0.0.0.0 exposes them on every interface.
  --local         Use this checkout's deploy/standalone and a locally built
                  image (tayga:local, built from docker/Dockerfile if missing)
                  instead of downloading a release.
  --uninstall     Stop and remove the stack's containers and network. Keeps the
                  data volumes and DIR, so a later install picks the data up again.
  --purge         With --uninstall: also delete the data volumes and DIR.
  -h, --help      Show this help.

Environment:
  TAYGA_DIR            Default for --dir.
  TAYGA_IMAGE          With --local: the image to use (default tayga:local).
  TAYGA_DOWNLOAD_BASE  Base URL of the release downloads (a mirror), default
                       https://github.com/softberries/tayga/releases/download;
                       the bundle is fetched from <base>/v<version>/.
EOF
}

say() { printf '%s\n' "$*"; }
err() { printf 'install.sh: %s\n' "$*" >&2; }
die() { err "$*"; exit 1; }

need_arg() { [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 needs a value"; }

while [ "$#" -gt 0 ]; do
	case "$1" in
	--version) need_arg "$@"; VERSION="$2"; shift 2 ;;
	--dir) need_arg "$@"; DIR="$2"; shift 2 ;;
	--project) need_arg "$@"; PROJECT="$2"; shift 2 ;;
	--ports) need_arg "$@"; OFFSET="$2"; shift 2 ;;
	--bind) need_arg "$@"; BIND="$2"; shift 2 ;;
	--local) LOCAL=1; shift ;;
	--uninstall) UNINSTALL=1; shift ;;
	--purge) PURGE=1; shift ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; die "unknown option: $1" ;;
	esac
done

[ "$PURGE" -eq 0 ] || [ "$UNINSTALL" -eq 1 ] || die "--purge only works with --uninstall"
case "$OFFSET" in
'' ) ;;
*[!0-9]*) die "--ports takes a non-negative number, got: $OFFSET" ;;
esac
case "$PROJECT" in
'' ) ;;
*[!a-z0-9_-]*) die "--project may only contain a-z, 0-9, _ and -" ;;
esac

# --- Docker ----------------------------------------------------------------

command -v docker >/dev/null 2>&1 || die "Docker is required: https://docs.docker.com/get-docker/"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon (is it running, and may this user use it?)"
docker compose version >/dev/null 2>&1 || die "Docker Compose v2 is required (the 'docker compose' plugin)"

# --- .env helpers ----------------------------------------------------------

# env_get KEY: the value of KEY in $DIR/.env, without surrounding quotes.
env_get() {
	[ -f "$DIR/.env" ] || return 0
	sed -n "s/^$1=//p" "$DIR/.env" | tail -n 1 | sed "s/^'\(.*\)'\$/\1/; s/^\"\(.*\)\"\$/\1/"
}

# env_set KEY VALUE: replaces or appends KEY=VALUE in $DIR/.env.
env_set() {
	tmp="$DIR/.env.tmp.$$"
	if [ -f "$DIR/.env" ] && grep -q "^$1=" "$DIR/.env"; then
		awk -v k="$1" -v v="$2" 'index($0, k "=") == 1 { print k "=" v; next } { print }' "$DIR/.env" >"$tmp"
	else
		{ [ -f "$DIR/.env" ] && cat "$DIR/.env"; printf '%s=%s\n' "$1" "$2"; } >"$tmp"
	fi
	mv "$tmp" "$DIR/.env"
}

# Without -f, compose also loads a compose.override.yaml the user keeps in $DIR.
compose() {
	docker compose --project-directory "$DIR" -p "$PROJECT" "$@"
}

# --- Uninstall -------------------------------------------------------------

if [ "$UNINSTALL" -eq 1 ]; then
	[ -f "$DIR/compose.yaml" ] || die "no Tayga install in $DIR (compose.yaml not found)"
	[ -n "$PROJECT" ] || PROJECT="$(env_get COMPOSE_PROJECT_NAME)"
	[ -n "$PROJECT" ] || PROJECT=tayga
	if [ "$PURGE" -eq 1 ]; then
		say "Removing the $PROJECT stack and its data volumes..."
		compose down --volumes --remove-orphans
		rm -rf "$DIR"
		say "Removed the stack, its volumes and $DIR."
	else
		say "Removing the $PROJECT stack (data volumes are kept)..."
		compose down --remove-orphans
		say "Removed the containers and network. Data volumes and $DIR are kept;"
		say "run again with --uninstall --purge to delete them."
	fi
	exit 0
fi

# --- Bundle ----------------------------------------------------------------

download() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL "$1" -o "$2"
	elif command -v wget >/dev/null 2>&1; then
		wget -q "$1" -O "$2"
	else
		die "curl or wget is required to download a release"
	fi
}

# The tag GitHub's /releases/latest redirects to.
latest_version() {
	command -v curl >/dev/null 2>&1 || die "curl is required to find the latest release; pass --version"
	url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" ||
		die "no release found at https://github.com/$REPO/releases (or GitHub is unreachable); pass --version, or use --local in a checkout"
	tag="${url##*/}"
	case "$tag" in
	v[0-9]*) printf '%s\n' "${tag#v}" ;;
	*) die "no release found at https://github.com/$REPO/releases (use --local in a checkout)" ;;
	esac
}

sha256_of() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d' ' -f1
	fi
}

# compose.yaml is managed by this script (customize through .env or a
# compose.override.yaml); files the user may have edited are never overwritten.
install_file() {
	if [ -f "$DIR/$2" ]; then
		if ! cmp -s "$1" "$DIR/$2"; then
			cp "$1" "$DIR/$2.new"
			say "Keeping your $DIR/$2 (this version's copy is $DIR/$2.new)."
		fi
	else
		cp "$1" "$DIR/$2"
	fi
}

if [ "$LOCAL" -eq 1 ]; then
	case "$0" in
	*/*) SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)" ;;
	*) die "--local needs the script run from a checkout (sh scripts/install.sh --local)" ;;
	esac
	ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
	SRC="$ROOT/deploy/standalone"
	[ -f "$SRC/compose.yaml" ] && [ -f "$ROOT/docker/Dockerfile" ] ||
		die "--local: $ROOT is not a Tayga checkout"
	IMAGE="${TAYGA_IMAGE:-tayga:local}"
	mkdir -p "$DIR"
	if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
		say "Building $IMAGE from $ROOT (the first build takes several minutes)..."
		docker build -f "$ROOT/docker/Dockerfile" -t "$IMAGE" "$ROOT"
	fi
	cp "$SRC/compose.yaml" "$DIR/compose.yaml"
	for f in notifier.toml otel-collector.yaml; do install_file "$SRC/$f" "$f"; done
	cp "$SRC/.env.example" "$DIR/.env.example"
	cp "$SRC/README.md" "$DIR/README.md"
	cp "$SCRIPT_DIR/install.sh" "$DIR/install.sh"
	[ -f "$DIR/.env" ] || cp "$SRC/.env.example" "$DIR/.env"
	env_set TAYGA_IMAGE "$IMAGE"
	VERSION="local"
else
	[ -n "$VERSION" ] || VERSION="$(latest_version)"
	VERSION="${VERSION#v}"
	NAME="tayga-standalone-$VERSION"
	BASE="${TAYGA_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download}/v$VERSION"
	TMP="$(mktemp -d)"
	trap 'rm -rf "$TMP"' EXIT INT TERM
	say "Downloading $NAME.tar.gz..."
	download "$BASE/$NAME.tar.gz" "$TMP/$NAME.tar.gz" || die "download failed: $BASE/$NAME.tar.gz"
	if download "$BASE/$NAME.tar.gz.sha256" "$TMP/$NAME.tar.gz.sha256" 2>/dev/null; then
		want="$(cut -d' ' -f1 <"$TMP/$NAME.tar.gz.sha256")"
		got="$(sha256_of "$TMP/$NAME.tar.gz")"
		if [ -z "$got" ]; then
			err "no sha256sum or shasum found; skipping the checksum check"
		elif [ "$want" != "$got" ]; then
			die "checksum mismatch for $NAME.tar.gz (expected $want, got $got)"
		fi
	fi
	tar -xzf "$TMP/$NAME.tar.gz" -C "$TMP"
	mkdir -p "$DIR"
	SRC="$TMP/$NAME"
	[ -f "$SRC/compose.yaml" ] || die "unexpected bundle layout in $NAME.tar.gz"
	cp "$SRC/compose.yaml" "$DIR/compose.yaml"
	for f in notifier.toml otel-collector.yaml; do install_file "$SRC/$f" "$f"; done
	cp "$SRC/.env.example" "$DIR/.env.example"
	cp "$SRC/README.md" "$DIR/README.md"
	cp "$SRC/install.sh" "$DIR/install.sh"
	[ -f "$DIR/.env" ] || cp "$SRC/.env.example" "$DIR/.env"
	env_set TAYGA_VERSION "$VERSION"
	# A release install uses the published image; drop a TAYGA_IMAGE left by --local.
	if [ -n "$(env_get TAYGA_IMAGE)" ]; then env_set TAYGA_IMAGE ""; fi
fi

# --- Settings --------------------------------------------------------------

if [ -n "$PROJECT" ]; then env_set COMPOSE_PROJECT_NAME "$PROJECT"; fi
PROJECT="$(env_get COMPOSE_PROJECT_NAME)"
[ -n "$PROJECT" ] || { PROJECT=tayga; env_set COMPOSE_PROJECT_NAME "$PROJECT"; }
if [ -n "$BIND" ]; then env_set TAYGA_BIND "$BIND"; fi
if [ -n "$OFFSET" ]; then
	env_set TAYGA_HTTP_PORT "$((8090 + OFFSET))"
	env_set TAYGA_OTLP_GRPC_PORT "$((4317 + OFFSET))"
	env_set TAYGA_OTLP_HTTP_PORT "$((4318 + OFFSET))"
fi
HTTP_PORT="$(env_get TAYGA_HTTP_PORT)"; HTTP_PORT="${HTTP_PORT:-8090}"
GRPC_PORT="$(env_get TAYGA_OTLP_GRPC_PORT)"; GRPC_PORT="${GRPC_PORT:-4317}"
OTLP_HTTP_PORT="$(env_get TAYGA_OTLP_HTTP_PORT)"; OTLP_HTTP_PORT="${OTLP_HTTP_PORT:-4318}"
BIND="$(env_get TAYGA_BIND)"; BIND="${BIND:-127.0.0.1}"
for p in "$HTTP_PORT" "$GRPC_PORT" "$OTLP_HTTP_PORT"; do
	[ "$p" -ge 1 ] && [ "$p" -le 65535 ] || die "port $p is out of range (check --ports)"
done
env_set TAYGA_PUBLIC_URL "http://localhost:$HTTP_PORT"

# --- Free ports ------------------------------------------------------------

# port_busy PORT: true when something listens on PORT on this host.
port_busy() {
	if command -v nc >/dev/null 2>&1; then
		nc -z 127.0.0.1 "$1" >/dev/null 2>&1
	elif command -v lsof >/dev/null 2>&1; then
		lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
	elif command -v ss >/dev/null 2>&1; then
		ss -ltn "sport = :$1" 2>/dev/null | grep -q LISTEN
	else
		return 1
	fi
}

# Ports this project already publishes are ours (a re-run); check only the others.
running="$(compose ps -q 2>/dev/null || true)"
if [ -z "$running" ]; then
	for p in "$HTTP_PORT" "$GRPC_PORT" "$OTLP_HTTP_PORT"; do
		if port_busy "$p"; then
			die "port $p is in use. Free it, or move Tayga's ports with --ports N (adds N to 8090, 4317 and 4318)"
		fi
	done
fi

# --- Start -----------------------------------------------------------------

say "Starting Tayga ($VERSION) as compose project $PROJECT in $DIR..."
compose up -d --remove-orphans

say "Waiting for tayga-api to become healthy (up to ${WAIT_SECS}s)..."
waited=0
while :; do
	cid="$(compose ps -q tayga-api 2>/dev/null || true)"
	status=""
	[ -z "$cid" ] || status="$(docker inspect -f '{{.State.Health.Status}}' "$cid" 2>/dev/null || true)"
	[ "$status" = healthy ] && break
	if [ "$waited" -ge "$WAIT_SECS" ]; then
		compose ps -a >&2 || true
		die "tayga-api is not healthy after ${WAIT_SECS}s (status: ${status:-not running}). Logs: docker compose -p $PROJECT --project-directory $DIR logs"
	fi
	sleep 5
	waited=$((waited + 5))
done

HOST="localhost"
[ "$BIND" = "127.0.0.1" ] || [ "$BIND" = "0.0.0.0" ] || HOST="$BIND"

cat <<EOF

Tayga is running.

  Web app and API:  http://$HOST:$HTTP_PORT
  OTLP gRPC:        $HOST:$GRPC_PORT
  OTLP HTTP:        http://$HOST:$OTLP_HTTP_PORT  (/v1/traces, /v1/logs)
  Install dir:      $DIR   (settings in .env)

Send traces and logs to Tayga from an OpenTelemetry Collector by adding this
exporter to your traces and logs pipelines (full example: $DIR/otel-collector.yaml):

  exporters:
    otlp_grpc/tayga:
      endpoint: $HOST:$GRPC_PORT
      tls:
        insecure: true

Manage the stack:
  docker compose --project-directory $DIR -p $PROJECT ps
  docker compose --project-directory $DIR -p $PROJECT logs -f tayga-api
  sh $DIR/install.sh --dir $DIR --uninstall           # keeps the data
  sh $DIR/install.sh --dir $DIR --uninstall --purge   # deletes the data too
EOF
