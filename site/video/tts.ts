/**
 * Narration: one ElevenLabs text-to-speech generation per segment of script.md.
 *
 *   node video/tts.ts                 # generate the segments whose text changed
 *   ONLY=03,07 node video/tts.ts      # regenerate these segments even if unchanged
 *
 * Writes build/audio/NN.mp3 and NN.json (text, voice, model, speed, tempo, duration and the
 * character alignment that record.ts uses for cues and assemble.ts for caption timing). The API
 * key is read from ELEVENLABS_API_KEY and only ever sent as the xi-api-key header.
 *
 * TEMPO speeds the generated speech up further with ffmpeg's atempo (pitch unchanged), and scales
 * the alignment to match. ElevenLabs' original is kept as NN.src.mp3, so a tempo change alone
 * needs no new generation. (eleven_v4 hardly reacts to voice_settings.speed: in a test on
 * 2026-10-07, 0.7 and 1.2 gave clips within 3 % of each other; atempo does the speeding up.)
 */
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { AUDIO, segments } from './segments.ts';

export const VOICE_ID = process.env.TAYGA_VOICE_ID ?? 'XrExE9yKIg1WjnnlVkGX'; // Matilda
export const MODEL_ID = process.env.TAYGA_TTS_MODEL ?? 'eleven_v4';
export const SPEED = Number(process.env.TAYGA_TTS_SPEED ?? '1.2');
export const TEMPO = Number(process.env.TAYGA_TTS_TEMPO ?? '1.14');

const KEY = process.env.ELEVENLABS_API_KEY;
const ONLY = (process.env.ONLY ?? '').split(',').map((s) => s.trim()).filter(Boolean);

mkdirSync(AUDIO, { recursive: true });
const segs = segments();

function duration(file: string): number {
	return Number(
		execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', file], { encoding: 'utf8' }).trim(),
	);
}

interface Alignment {
	characters: string[];
	character_start_times_seconds: number[];
	character_end_times_seconds: number[];
}

/** Applies TEMPO to the source MP3 and writes NN.mp3 and NN.json. */
function finish(id: string, text: string, src: string, raw: Alignment): number {
	const mp3 = `${AUDIO}${id}.mp3`;
	execFileSync('ffmpeg', ['-hide_banner', '-v', 'error', '-y', '-i', src, '-af', `atempo=${TEMPO}`, '-c:a', 'libmp3lame', '-b:a', '128k', '-ar', '44100', mp3]);
	const d = duration(mp3);
	const scale = (xs: number[]) => xs.map((t) => Math.round((t / TEMPO) * 1000) / 1000);
	const alignment = { characters: raw.characters, character_start_times_seconds: scale(raw.character_start_times_seconds), character_end_times_seconds: scale(raw.character_end_times_seconds) };
	writeFileSync(`${AUDIO}${id}.json`, JSON.stringify({ text, voice_id: VOICE_ID, model_id: MODEL_ID, speed: SPEED, tempo: TEMPO, duration: d, alignment, raw_alignment: raw }, null, 1));
	return d;
}

for (const [i, s] of segs.entries()) {
	const json = `${AUDIO}${s.id}.json`;
	const src = `${AUDIO}${s.id}.src.mp3`;
	if (!ONLY.includes(s.id) && existsSync(json) && existsSync(src)) {
		const old = JSON.parse(readFileSync(json, 'utf8'));
		if (old.text === s.text && old.voice_id === VOICE_ID && old.model_id === MODEL_ID && old.speed === SPEED && old.raw_alignment) {
			if (old.tempo === TEMPO) {
				console.log(`${s.id} unchanged (${old.duration.toFixed(1)} s)`);
				continue;
			}
			const d = finish(s.id, s.text, src, old.raw_alignment);
			console.log(`${s.id} re-tempoed to ${TEMPO}: ${d.toFixed(1)} s`);
			continue;
		}
	}
	if (!KEY) throw new Error('ELEVENLABS_API_KEY is not set');
	const body = {
		text: s.text,
		model_id: MODEL_ID,
		voice_settings: { speed: SPEED },
		previous_text: segs[i - 1]?.text ?? null,
		next_text: segs[i + 1]?.text ?? null,
	};
	const r = await fetch(`https://api.elevenlabs.io/v1/text-to-speech/${VOICE_ID}/with-timestamps?output_format=mp3_44100_128`, {
		method: 'POST',
		headers: { 'xi-api-key': KEY, 'content-type': 'application/json', accept: 'application/json' },
		body: JSON.stringify(body),
	});
	if (!r.ok) throw new Error(`segment ${s.id}: HTTP ${r.status} ${(await r.text()).slice(0, 300)}`);
	const j = (await r.json()) as { audio_base64: string; alignment: Alignment };
	writeFileSync(src, Buffer.from(j.audio_base64, 'base64'));
	const d = finish(s.id, s.text, src, j.alignment);
	const words = s.text.split(/\s+/).length;
	console.log(`${s.id} ${s.title}: ${d.toFixed(1)} s, ${words} words, ${((words / d) * 60).toFixed(0)} wpm`);
}
