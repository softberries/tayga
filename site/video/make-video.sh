#!/bin/sh
# Rebuilds the narrated tour: site/public/media/tayga-tour.mp4, tayga-tour-poster.jpg and
# tayga-tour.vtt. See README.md in this directory for the prerequisites.
#
#   site/video/make-video.sh             # narration (changed segments only), recording, assembly
#   SKIP_TTS=1 site/video/make-video.sh  # reuse the narration in build/audio
#   SKIP_RECORD=1 ...                    # reuse the clips in build/clips (assembly only)
#   ONLY=05,06 ...                       # re-record only these segments, then assemble
#
# Segment 3 flips the demo's paymentFailure flag; the flags are reset when the script exits,
# whatever the outcome.
set -eu

VIDEO_DIR="$(cd "$(dirname "$0")" && pwd)"
SITE="$(cd "$VIDEO_DIR/.." && pwd)"
ROOT="$(cd "$SITE/.." && pwd)"
TAYGA_URL="${TAYGA_URL:-http://localhost:8090}"
DOCS_PORT="${DOCS_PORT:-4321}"
export TAYGA_URL

die() { printf 'make-video.sh: %s\n' "$*" >&2; exit 1; }

for cmd in node npm ffmpeg ffprobe make cargo curl; do
	command -v "$cmd" >/dev/null 2>&1 || die "$cmd is required"
done
[ -n "${SKIP_TTS:-}" ] || [ -n "${ELEVENLABS_API_KEY:-}" ] || die "ELEVENLABS_API_KEY is not set (or pass SKIP_TTS=1)"

if [ -z "${SKIP_RECORD:-}" ]; then
	curl -fsS "$TAYGA_URL/api/v1/config" >/dev/null || die "the Tayga app is not reachable at $TAYGA_URL (run make up)"
	# Never leave a failure flag on.
	trap 'make -s -C "$ROOT" flags-reset' EXIT INT TERM
	# `make flag` runs tayga-devtools through cargo; build it first so the flip is instant.
	cargo build -q -p tayga-devtools --manifest-path "$ROOT/Cargo.toml"
fi

[ -n "${SKIP_TTS:-}" ] || node "$VIDEO_DIR/tts.ts"

if [ -z "${SKIP_RECORD:-}" ]; then
	# Segment 10 films the docs site's landing page from a local build.
	PREVIEW_PID=""
	DOCS_URL="${DOCS_URL:-http://localhost:$DOCS_PORT/tayga/}"
	if ! curl -fsS "$DOCS_URL" >/dev/null 2>&1; then
		npm --prefix "$SITE" run build
		(cd "$SITE" && exec npx astro preview --port "$DOCS_PORT") >/dev/null 2>&1 &
		PREVIEW_PID=$!
		trap 'make -s -C "$ROOT" flags-reset; [ -z "$PREVIEW_PID" ] || kill "$PREVIEW_PID" 2>/dev/null || true' EXIT INT TERM
		i=0
		until curl -fsS "$DOCS_URL" >/dev/null 2>&1; do
			i=$((i + 1))
			[ "$i" -lt 60 ] || die "the docs preview did not start at $DOCS_URL"
			sleep 1
		done
	fi
	export DOCS_URL
	(cd "$SITE" && node video/record.ts)
fi

(cd "$SITE" && node video/assemble.ts)
(cd "$SITE" && node video/poster.ts)
ls -l "$SITE/public/media/"
