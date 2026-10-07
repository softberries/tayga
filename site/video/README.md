# The narrated tour

The tour on the landing page is built from this directory:

| File | Role |
|---|---|
| `script.md` | The narration (the `>` lines, one block per segment) and what each segment shows. The single source of the spoken text. |
| `segments.ts` | Parses `script.md`; shared paths; finds when a phrase is spoken (cues). |
| `tts.ts` | ElevenLabs text to speech, one MP3 per segment, with the character alignment. |
| `record.ts` | Playwright at 1920×1080 with `recordVideo`, one clip per segment, paced by the narration. |
| `retime.ts` | Re-fits recorded clips to a regenerated narration of the same text, without filming again. |
| `assemble.ts` | ffmpeg: fits each clip to its narration, crossfades, mixes and normalizes the audio, encodes, writes the captions. |
| `poster.ts` | Renders `cards/poster.html` to the landing page's poster and to `docs/assets/tour-poster.jpg`, the README's poster with a play button drawn in. Runs without the app: `cd site && node video/poster.ts`. |
| `cards/` | The title, end, architecture and deployment cards, and the poster (HTML in the Tayga fonts and colours). The logo is `ui/public/logo.jpg`, the dog portrait (1024 px), cropped in `base.css` to the square of the app's rail logo (`logo-mark.png` is that crop at 136 px). |
| `make-video.sh` | Runs the three steps. |

Output: `site/public/media/tayga-tour.mp4`, `tayga-tour-poster.jpg` and `tayga-tour.vtt`, which
`src/components/landing/Video.astro` picks up at build time. Intermediate files go to
`build/` (ignored by git).

## Voice and model

| Setting | Value |
|---|---|
| Voice | Matilda (`XrExE9yKIg1WjnnlVkGX`), an ElevenLabs premade voice: American English, "Knowledgable, Professional", labelled for informative and educational use |
| Model | `eleven_v4`, the model ElevenLabs recommends for content creation and narration (checked in the ElevenLabs docs on 2026-10-07) |
| Endpoint | `POST /v1/text-to-speech/{voice_id}/with-timestamps`, `output_format=mp3_44100_128`, `voice_settings.speed` 1.2 (the maximum; the documented range is 0.7 to 1.2), with `previous_text` and `next_text` for continuity between segments |
| Tempo | ffmpeg `atempo=1.14` on each generated MP3 (pitch unchanged), alignment scaled to match, so the narration runs 1.15× as fast as the earlier render at speed 1.05. `eleven_v4` barely reacts to `speed`: on 2026-10-07 the same sentence came out at 9.28 s with 0.7 and 9.52 s with 1.2, and the whole narration at 1.2 was only 1 % shorter than at 1.05. |
| Result | Narration 187.8 s (segments 1 to 9: 179.2 s, against 205.8 s at speed 1.05 and no tempo: 1.15× as fast); video 3:34 (214.3 s), 27.9 MB, -16.3 LUFS. A speech-to-text pass (ElevenLabs `scribe_v2`) returned every word of the script. |

Override with `TAYGA_VOICE_ID`, `TAYGA_TTS_MODEL`, `TAYGA_TTS_SPEED` and `TAYGA_TTS_TEMPO`. `tts.ts`
only calls the API for segments whose text, voice, model or speed changed (or those named in
`ONLY`); it keeps ElevenLabs' original as `NN.src.mp3`, so a tempo change alone re-applies
`atempo` without a new generation.

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

After a new narration of the same text (another speed or tempo), the clips need not be filmed
again: `cd site && ONLY=01,03,04 node video/retime.ts && node video/assemble.ts`. `retime.ts`
scales each clip's narration span by one rate to the new duration (a word-by-word map follows
the alignment's jitter and makes the picture lurch); the lead-in, the tail and segment 3's
time-lapse are kept. With the 2026-10-07 narration every cue stayed within 0.35 s of its words.
A clip whose card or text changed has to be filmed again.

Segment 3 turns on `paymentFailure`, films the Stories page until Tayga writes the story (the
`make e2e` runs saw 45 to 95 seconds) and runs `make flags-reset` in a `finally` block; the
shell script also resets the flags on exit. Segment 4 opens the story segment 3 produced, so
record them together. The wait is shown as a time-lapse, with a badge counting the real seconds
since the flip.

Segment 10 films the closing section of the docs site's landing page: the script builds the site
and serves it with `astro preview` unless `DOCS_URL` already answers. The video has no
Enterprise segment: the landing page's Enterprise section and the header's email link are hidden
while filming (the site keeps them), and the end card shows only the docs and code links.

## How the timing works

`tts.ts` stores each segment's character alignment. `record.ts` schedules every action against
it (`s.on('Compared with normal')` waits until that phrase is spoken), with 0.8 s of settled
picture before the narration and 1.4 s after. It records how clip time maps to narration time
(`build/clips/NN.json`); `assemble.ts` cuts and retimes each clip by that map, places each MP3 at
its segment's start, joins the clips with 0.5 s crossfades, normalizes to -16 LUFS and encodes
H.264 + AAC at 1080p, two-pass to stay under 29 MB, with `faststart`. The captions are cut at
sentence and clause breaks and timed from the same alignment.
