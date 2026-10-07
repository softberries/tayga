/**
 * Shared helpers for the tour video: the segments parsed from script.md, the build paths, and
 * the cue lookup (when a phrase is spoken, from the TTS character alignment).
 */
import { readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const VIDEO_DIR = fileURLToPath(new URL('./', import.meta.url));
export const BUILD = `${VIDEO_DIR}build/`;
export const AUDIO = `${BUILD}audio/`;
export const CLIPS = `${BUILD}clips/`;

export interface Segment {
	n: number;
	id: string;
	title: string;
	text: string;
}

/** The numbered `## N. Title` sections of script.md and their `>` narration lines. */
export function segments(): Segment[] {
	const md = readFileSync(`${VIDEO_DIR}script.md`, 'utf8');
	const out: Segment[] = [];
	let cur: Segment | null = null;
	for (const line of md.split('\n')) {
		const h = /^## (\d+)\. (.+)$/.exec(line);
		if (h) {
			cur = { n: Number(h[1]), id: h[1].padStart(2, '0'), title: h[2].trim(), text: '' };
			out.push(cur);
			continue;
		}
		if (cur && line.startsWith('> ')) cur.text = `${cur.text} ${line.slice(2).trim()}`.trim();
	}
	return out;
}

export interface Alignment {
	characters: string[];
	character_start_times_seconds: number[];
	character_end_times_seconds: number[];
}

export interface Narration {
	text: string;
	voice_id: string;
	model_id: string;
	duration: number;
	alignment: Alignment;
}

export function narration(id: string): Narration {
	const p = `${AUDIO}${id}.json`;
	if (!existsSync(p)) throw new Error(`no narration for segment ${id}: run tts.ts first`);
	return JSON.parse(readFileSync(p, 'utf8')) as Narration;
}

/** Seconds into the segment's audio when `phrase` starts being spoken. */
export function cue(n: Narration, phrase: string): number {
	const text = n.alignment.characters.join('');
	const i = text.indexOf(phrase);
	if (i < 0) throw new Error(`cue "${phrase}" not in: ${text}`);
	return n.alignment.character_start_times_seconds[i];
}
