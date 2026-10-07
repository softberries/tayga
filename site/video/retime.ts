/**
 * Re-fits recorded clips to a new narration of the same text, without filming them again:
 *
 *   ONLY=01,03 node video/retime.ts      (from site/)
 *
 * record.ts stores, with each clip, the alignment of the narration it was filmed against
 * (`recorded`). When the narration is regenerated (a new speed or tempo, the same words), this
 * scales the filmed narration span to the new duration and rewrites the clip's `pieces`
 * (narration time to video time) for assemble.ts. The
 * lead-in and the tail keep their real length. The time-lapse in segment 3 stays a time-lapse.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { CLIPS, narration } from './segments.ts';
import type { Alignment } from './segments.ts';

interface Piece {
	a0: number;
	a1: number;
	v0: number;
	v1: number;
}
interface Recorded {
	text: string;
	duration: number;
	alignment: Alignment;
	pieces: Piece[];
}

const ONLY = (process.env.ONLY ?? '').split(',').map((s) => s.trim()).filter(Boolean);
if (!ONLY.length) throw new Error('name the clips to retime, e.g. ONLY=01,03');

type Pt = [number, number]; // [new narration time, old narration time]

function interp(xs: number[], ys: number[], x: number): number {
	for (let i = 1; i < xs.length; i++) if (x <= xs[i] || i === xs.length - 1) return ys[i - 1] + ((ys[i] - ys[i - 1]) * (x - xs[i - 1])) / (xs[i] - xs[i - 1]);
	return ys[0];
}

for (const id of ONLY) {
	const file = `${CLIPS}${id}.json`;
	const meta = JSON.parse(readFileSync(file, 'utf8')) as { lead: number; tail: number; duration: number; pieces: Piece[]; recorded?: Recorded };
	const rec = meta.recorded;
	if (!rec) throw new Error(`clip ${id} has no recorded narration: film it again with record.ts`);
	const now = narration(id);
	if (now.text !== rec.text) throw new Error(`clip ${id}: the narration text changed since filming; film it again`);
	const { lead, tail } = meta;

	// One rate for the whole narration. A word-by-word map follows the alignment's jitter (the
	// video would lurch between 0.4x and 4x); one rate keeps every cue that record.ts schedules
	// within about 0.4 s of its words (measured on 2026-10-07), and the rings start 0.25 s early.
	const map: Pt[] = [[0, 0], [now.duration, rec.duration]];
	const W: Pt[] = [[-lead, -lead], ...map, [now.duration + tail, rec.duration + tail]];
	const wn = W.map((p) => p[0]);
	const wo = W.map((p) => p[1]);

	// Old narration time to video time, from the pieces filmed.
	const vo = [rec.pieces[0].a0, ...rec.pieces.map((p) => p.a1)];
	const vv = [rec.pieces[0].v0, ...rec.pieces.map((p) => p.v1)];

	// Breakpoints: the map's own, and where the map reaches a boundary between filmed pieces.
	const cuts = new Set(wn.map((x) => +x.toFixed(4)));
	for (const o of vo.slice(1, -1)) cuts.add(+interp(wo, wn, o).toFixed(4));
	const ts = [...cuts].sort((x, y) => x - y);
	const pieces: Piece[] = [];
	for (let i = 1; i < ts.length; i++) {
		const [a0, a1] = [ts[i - 1], ts[i]];
		if (a1 - a0 < 0.02) continue;
		const v0 = interp(vo, vv, interp(wn, wo, a0));
		const v1 = interp(vo, vv, interp(wn, wo, a1));
		pieces.push({ a0: pieces.length ? pieces[pieces.length - 1].a1 : a0, a1, v0: pieces.length ? pieces[pieces.length - 1].v1 : v0, v1 });
	}
	writeFileSync(file, JSON.stringify({ ...meta, duration: now.duration, pieces }, null, 1));
	const rates = pieces.map((p) => (p.v1 - p.v0) / (p.a1 - p.a0));
	console.log(`  ${id}: ${rec.duration.toFixed(1)} s -> ${now.duration.toFixed(1)} s, ${pieces.length} pieces, video speed ${Math.min(...rates).toFixed(2)} to ${Math.max(...rates).toFixed(2)}x`);
}
