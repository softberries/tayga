/**
 * Narration: one ElevenLabs text-to-speech generation per segment of script.md.
 *
 *   node video/tts.ts                 # generate the segments whose text changed
 *   ONLY=03,07 node video/tts.ts      # regenerate these segments even if unchanged
 *
 * Writes build/audio/NN.mp3 and NN.json (text, voice, model, duration and the character
 * alignment that record.ts uses for cues and captions.ts for caption timing). The API key is
 * read from ELEVENLABS_API_KEY and only ever sent as the xi-api-key header.
 */
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { AUDIO, segments } from './segments.ts';

export const VOICE_ID = process.env.TAYGA_VOICE_ID ?? 'XrExE9yKIg1WjnnlVkGX'; // Matilda
export const MODEL_ID = process.env.TAYGA_TTS_MODEL ?? 'eleven_v4';
export const SPEED = Number(process.env.TAYGA_TTS_SPEED ?? '1.05');

const KEY = process.env.ELEVENLABS_API_KEY;
if (!KEY) throw new Error('ELEVENLABS_API_KEY is not set');
const ONLY = (process.env.ONLY ?? '').split(',').map((s) => s.trim()).filter(Boolean);

mkdirSync(AUDIO, { recursive: true });
const segs = segments();

function duration(file: string): number {
	return Number(
		execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', file], { encoding: 'utf8' }).trim(),
	);
}

for (const [i, s] of segs.entries()) {
	const json = `${AUDIO}${s.id}.json`;
	const mp3 = `${AUDIO}${s.id}.mp3`;
	if (!ONLY.includes(s.id) && existsSync(json) && existsSync(mp3)) {
		const old = JSON.parse(readFileSync(json, 'utf8'));
		if (old.text === s.text && old.voice_id === VOICE_ID && old.model_id === MODEL_ID && old.speed === SPEED) {
			console.log(`${s.id} unchanged (${old.duration.toFixed(1)} s)`);
			continue;
		}
	}
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
	const j = (await r.json()) as { audio_base64: string; alignment: unknown };
	writeFileSync(mp3, Buffer.from(j.audio_base64, 'base64'));
	const d = duration(mp3);
	writeFileSync(json, JSON.stringify({ text: s.text, voice_id: VOICE_ID, model_id: MODEL_ID, speed: SPEED, duration: d, alignment: j.alignment }, null, 1));
	const words = s.text.split(/\s+/).length;
	console.log(`${s.id} ${s.title}: ${d.toFixed(1)} s, ${words} words, ${((words / d) * 60).toFixed(0)} wpm`);
}
