# The narrated tour

The tour on the landing page is built from this directory:

| File | Role |
|---|---|
| `script.md` | The narration (the `>` lines, one block per segment) and what each segment shows. The single source of the spoken text. |
| `segments.ts` | Parses `script.md`; shared paths; finds when a phrase is spoken (cues). |
| `tts.ts` | ElevenLabs text to speech, one MP3 per segment, with the character alignment. |
| `record.ts` | Playwright at 1920×1080 with `recordVideo`, one clip per segment, paced by the narration. |
| `assemble.ts` | ffmpeg: fits each clip to its narration, crossfades, mixes and normalizes the audio, encodes, writes the poster and the captions. |
| `cards/` | The title, end, architecture and deployment cards (HTML in the Tayga fonts and colours). |
| `make-video.sh` | Runs the three steps. |

Output: `site/public/media/tayga-tour.mp4`, `tayga-tour-poster.jpg` and `tayga-tour.vtt`, which
`src/components/landing/Video.astro` picks up at build time. Intermediate files go to
`build/` (ignored by git).

## Voice and model

| Setting | Value |
|---|---|
| Voice | Matilda (`XrExE9yKIg1WjnnlVkGX`), an ElevenLabs premade voice: American English, "Knowledgable, Professional", labelled for informative and educational use |
| Model | `eleven_v4`, the model ElevenLabs recommends for content creation and narration (checked in the ElevenLabs docs on 2026-10-07) |
| Endpoint | `POST /v1/text-to-speech/{voice_id}/with-timestamps`, `output_format=mp3_44100_128`, `voice_settings.speed` 1.05, with `previous_text` and `next_text` for continuity between segments |

Override with `TAYGA_VOICE_ID`, `TAYGA_TTS_MODEL` and `TAYGA_TTS_SPEED`. `tts.ts` only calls the
API for segments whose text, voice, model or speed changed (or those named in `ONLY`).

## Prerequisites

- The Tayga stack with the OpenTelemetry demo running (`make up`), the app at
  `http://localhost:8090` (`TAYGA_URL`). The recording uses live data: real stories, alerts and
  pipeline metrics. No payment error story may exist in the last 15 minutes when segment 3
  starts, so the new one is visibly new.
- Node.js 24 or later (it runs the `.ts` files directly) and the site's dependencies
  (`npm --prefix site ci`, which includes Playwright; run `npx --prefix site playwright install
  chromium` once).
- ffmpeg and ffprobe with libx264 (`brew install ffmpeg`).
- Rust and `make`: segment 3 sets the demo flag with `make flag`, as the e2e tests do.
- `ELEVENLABS_API_KEY` in the environment, for the narration. It is sent only as the
  `xi-api-key` header and never written anywhere.

## Run

```sh
site/video/make-video.sh                 # everything
SKIP_TTS=1 site/video/make-video.sh      # keep the narration
SKIP_RECORD=1 site/video/make-video.sh   # re-assemble only
ONLY=05,06 site/video/make-video.sh      # re-record two segments, then assemble
```

Segment 3 turns on `paymentFailure`, films the Stories page until Tayga writes the story (the
`make e2e` runs saw 45 to 95 seconds) and runs `make flags-reset` in a `finally` block; the
shell script also resets the flags on exit. Segment 4 opens the story segment 3 produced, so
record them together. The wait is shown as a time-lapse, with a badge counting the real seconds
since the flip.

Segment 10 films the docs site's landing page: the script builds the site and serves it with
`astro preview` unless `DOCS_URL` already answers.

## How the timing works

`tts.ts` stores each segment's character alignment. `record.ts` schedules every action against
it (`s.on('Compared with normal')` waits until that phrase is spoken), with 0.8 s of settled
picture before the narration and 1.4 s after. It records how clip time maps to narration time
(`build/clips/NN.json`); `assemble.ts` cuts and retimes each clip by that map, places each MP3 at
its segment's start, joins the clips with 0.5 s crossfades, normalizes to -16 LUFS and encodes
H.264 + AAC at 1080p, two-pass to stay under 29 MB, with `faststart`. The captions are cut at
sentence and clause breaks and timed from the same alignment.
