/**
 * Assembles the tour from the recorded clips and the narration:
 *
 *   node video/assemble.ts
 *
 * 1. Each segment's video is cut and retimed to its narration (build/seg/NN.mp4): the pieces in
 *    build/clips/NN.json map narration time to video time, so a segment is exactly its lead-in,
 *    narration and tail; the wait in segment 3 is the one piece that is sped up (a time-lapse).
 * 2. The title and end cards become short clips.
 * 3. Everything is joined with crossfades; each narration is placed at its segment's start, the
 *    mix is loudness-normalized to -16 LUFS (two-pass loudnorm), and the result is encoded as
 *    H.264 + AAC with faststart into site/public/media/tayga-tour.mp4.
 * 4. The captions (tayga-tour.vtt) are timed from the narration's character alignment. The
 *    poster is rendered separately by poster.ts.
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { AUDIO, BUILD, CLIPS, narration, segments } from './segments.ts';

const OUT = fileURLToPath(new URL('../public/media/', import.meta.url));
const SEG = `${BUILD}seg/`;
const FPS = 30;
const XF = 0.5; // crossfade seconds
const TITLE = 4.0;
const END = 6.0;
/** Size budget for the MP4; the video bitrate is derived from it. */
const BUDGET_BYTES = 29 * 1000 * 1000;
const AUDIO_KBPS = 128;

mkdirSync(SEG, { recursive: true });
mkdirSync(OUT, { recursive: true });

function ff(args: string[]): string {
	return execFileSync('ffmpeg', ['-hide_banner', '-v', 'error', '-y', ...args], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] });
}
function probeDuration(file: string): number {
	return Number(execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', file], { encoding: 'utf8' }).trim());
}

interface Piece {
	a0: number;
	a1: number;
	v0: number;
	v1: number;
}
interface ClipMeta {
	lead: number;
	tail: number;
	duration: number;
	pieces: Piece[];
}

const VENC = ['-c:v', 'libx264', '-preset', 'veryfast', '-crf', '12', '-pix_fmt', 'yuv420p', '-r', String(FPS)];

// 1. Segments, fitted to their narration.
const segs = segments();
const clips: { file: string; len: number; audio?: string; lead?: number; id?: string }[] = [];

ff(['-loop', '1', '-t', String(TITLE), '-i', `${CLIPS}title.png`, '-vf', `fps=${FPS},format=yuv420p,fade=t=in:st=0:d=0.8`, ...VENC, `${SEG}title.mp4`]);
clips.push({ file: `${SEG}title.mp4`, len: TITLE });

for (const s of segs) {
	const m = JSON.parse(readFileSync(`${CLIPS}${s.id}.json`, 'utf8')) as ClipMeta;
	const parts = m.pieces.map((p, i) => {
		const factor = (p.a1 - p.a0) / (p.v1 - p.v0);
		return `[0:v]trim=start=${p.v0.toFixed(3)}:end=${p.v1.toFixed(3)},setpts=(PTS-STARTPTS)*${factor.toFixed(6)},fps=${FPS}[p${i}]`;
	});
	const n = m.pieces.length;
	const join = n > 1 ? `${m.pieces.map((_, i) => `[p${i}]`).join('')}concat=n=${n}:v=1:a=0[cat];[cat]` : `[p0]`;
	const len = m.pieces[n - 1].a1 - m.pieces[0].a0;
	const graph = `${parts.join(';')};${join}trim=duration=${len.toFixed(3)},setpts=PTS-STARTPTS,format=yuv420p[v]`;
	const file = `${SEG}${s.id}.mp4`;
	ff(['-i', `${CLIPS}${s.id}.webm`, '-filter_complex', graph, '-map', '[v]', ...VENC, file]);
	const got = probeDuration(file);
	if (Math.abs(got - len) > 0.15) console.warn(`  ! segment ${s.id}: ${got.toFixed(2)} s, expected ${len.toFixed(2)} s`);
	clips.push({ file, len: got, audio: `${AUDIO}${s.id}.mp3`, lead: -m.pieces[0].a0, id: s.id });
	if (n > 1) {
		const sped = m.pieces.map((p) => ((p.v1 - p.v0) / (p.a1 - p.a0)).toFixed(1)).join(' / ');
		console.log(`  ${s.id}: ${got.toFixed(1)} s (piece speeds ${sped}×)`);
	} else console.log(`  ${s.id}: ${got.toFixed(1)} s`);
}

ff(['-loop', '1', '-t', String(END), '-i', `${CLIPS}end.png`, '-vf', `fps=${FPS},format=yuv420p`, ...VENC, `${SEG}end.mp4`]);
clips.push({ file: `${SEG}end.mp4`, len: END });

// 2. Timeline: clip k starts at sum(len before) - k * XF.
const starts: number[] = [];
let acc = 0;
for (const [k, c] of clips.entries()) {
	starts.push(acc - k * XF);
	acc += c.len;
}
const total = acc - (clips.length - 1) * XF;
console.log(`timeline ${total.toFixed(1)} s`);

const narrated = clips.map((c, k) => ({ c, k })).filter((x) => x.c.audio);

let vchain = '';
let prev = '[0:v]';
for (let k = 1; k < clips.length; k++) {
	const out = k === clips.length - 1 ? '[vx]' : `[x${k}]`;
	vchain += `${prev}[${k}:v]xfade=transition=fade:duration=${XF}:offset=${starts[k].toFixed(3)}${out};`;
	prev = out;
}
vchain += `[vx]fade=t=out:st=${(total - 0.8).toFixed(3)}:d=0.8,format=yuv420p[v]`;

const delays = narrated.map(({ c, k }, i) => {
	const ms = Math.round((starts[k] + c.lead!) * 1000);
	return `[${i}:a]aformat=sample_rates=48000:channel_layouts=mono,adelay=${ms}:all=1[a${i}]`;
});
const amix = `${narrated.map((_, i) => `[a${i}]`).join('')}amix=inputs=${narrated.length}:normalize=0:dropout_transition=0,apad=whole_dur=${total.toFixed(3)},atrim=duration=${total.toFixed(3)}`;

// Loudness: measure, then normalize linearly to -16 LUFS.
const mixWav = `${BUILD}mix.wav`;
ff([...narrated.flatMap(({ c }) => ['-i', c.audio!]), '-filter_complex', `${delays.join(';')};${amix}[a]`, '-map', '[a]', '-ar', '48000', '-ac', '1', mixWav]);
const measured = execFileSync('sh', ['-c', `ffmpeg -hide_banner -i "${mixWav}" -af loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json -f null - 2>&1`], { encoding: 'utf8' });
const j = JSON.parse(measured.slice(measured.lastIndexOf('{'), measured.lastIndexOf('}') + 1)) as Record<string, string>;
const norm = `loudnorm=I=-16:TP=-1.5:LRA=11:measured_I=${j.input_i}:measured_TP=${j.input_tp}:measured_LRA=${j.input_lra}:measured_thresh=${j.input_thresh}:offset=${j.target_offset}:linear=true`;
console.log(`loudness before ${j.input_i} LUFS`);

// 3. Final encode, two-pass to the size budget.
const vkbps = Math.floor((BUDGET_BYTES * 8) / total / 1000 - AUDIO_KBPS - 30);
console.log(`video bitrate ${vkbps} kb/s`);
const mp4 = `${OUT}tayga-tour.mp4`;
const graph = `${vchain};[${clips.length}:a]${norm},afade=t=out:st=${(total - 0.8).toFixed(3)}:d=0.8,aresample=48000[a]`;
const x264 = ['-c:v', 'libx264', '-preset', 'slow', '-tune', 'stillimage', '-b:v', `${vkbps}k`, '-maxrate', `${vkbps * 3}k`, '-bufsize', `${vkbps * 6}k`, '-pix_fmt', 'yuv420p', '-r', String(FPS), '-g', String(FPS * 4), '-profile:v', 'high'];
const passlog = `${BUILD}x264pass`;
const finalInputs: string[] = [];
for (const c of clips) finalInputs.push('-i', c.file);
finalInputs.push('-i', mixWav);
ff([...finalInputs, '-filter_complex', graph, '-map', '[v]', '-map', '[a]', ...x264, '-pass', '1', '-passlogfile', passlog, '-an', '-f', 'mp4', '/dev/null']);
ff([...finalInputs, '-filter_complex', graph, '-map', '[v]', '-map', '[a]', ...x264, '-pass', '2', '-passlogfile', passlog, '-c:a', 'aac', '-b:a', `${AUDIO_KBPS}k`, '-ac', '2', '-ar', '48000', '-movflags', '+faststart', mp4]);
console.log(`${mp4}: ${(statSync(mp4).size / 1e6).toFixed(1)} MB, ${probeDuration(mp4).toFixed(1)} s`);


// 4. Captions from the alignment: sentence or clause chunks of at most two short lines.
const SHOW: [RegExp, string][] = [
	[/two point eight microseconds/g, '2.8 µs'],
	[/forty-five to ninety-five seconds/g, '45 to 95 seconds'],
	[/p ninety-nine/g, 'p99'],
	[/twenty-four hour/g, '24-hour'],
	[/five percent/g, '5 %'],
	[/payment failure flag/g, 'paymentFailure flag'],
];
const MAX = 84;
function chunks(text: string): [number, number][] {
	const out: [number, number][] = [];
	let start = 0;
	const re = /[.?!:,]\s|$/g;
	let lastCut = 0;
	let m: RegExpExecArray | null;
	while ((m = re.exec(text))) {
		const end = m.index + (m[0].length ? 1 : 0);
		if (end - start > MAX && lastCut > start) {
			out.push([start, lastCut]);
			start = lastCut + 1;
		}
		if (/[.?!]/.test(m[0]) || end === text.length) {
			if (end - start > MAX) {
				// split a long sentence at the last clause break or space before MAX
				let s0 = start;
				while (end - s0 > MAX) {
					const slice = text.slice(s0, s0 + MAX);
					const cut = Math.max(slice.lastIndexOf(', '), slice.lastIndexOf(': ')) > 30 ? Math.max(slice.lastIndexOf(', '), slice.lastIndexOf(': ')) + 1 : slice.lastIndexOf(' ');
					out.push([s0, s0 + cut]);
					s0 = s0 + cut + 1;
				}
				out.push([s0, end]);
			} else if (end > start) out.push([start, end]);
			start = end + 1;
		}
		lastCut = end;
		if (m[0] === '') break;
	}
	return out.filter(([a, b]) => text.slice(a, b).trim().length);
}
function ts(t: number): string {
	const ms = Math.max(0, Math.round(t * 1000));
	const h = Math.floor(ms / 3600000);
	const mi = Math.floor((ms % 3600000) / 60000);
	const s = Math.floor((ms % 60000) / 1000);
	return `${String(h).padStart(2, '0')}:${String(mi).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;
}
function wrap(t: string): string {
	if (t.length <= 44) return t;
	const mid = t.length / 2;
	let best = -1;
	for (let i = 0; i < t.length; i++) if (t[i] === ' ' && (best < 0 || Math.abs(i - mid) < Math.abs(best - mid))) best = i;
	return best < 0 ? t : `${t.slice(0, best)}\n${t.slice(best + 1)}`;
}
const cues: string[] = ['WEBVTT', ''];
let nCue = 0;
for (const { c, k } of narrated) {
	const n = narration(c.id!);
	const text = n.alignment.characters.join('');
	const st = n.alignment.character_start_times_seconds;
	const en = n.alignment.character_end_times_seconds;
	const off = starts[k] + c.lead!;
	for (const [a, b] of chunks(text)) {
		let shown = text.slice(a, b).trim();
		for (const [re, rep] of SHOW) shown = shown.replace(re, rep);
		const t0 = off + st[a + (text.slice(a, b).length - text.slice(a, b).trimStart().length)];
		const t1 = off + en[Math.min(b, en.length) - 1] + 0.25;
		cues.push(String(++nCue), `${ts(t0)} --> ${ts(t1)}`, wrap(shown), '');
	}
}
writeFileSync(`${OUT}tayga-tour.vtt`, cues.join('\n'));
console.log(`captions: ${nCue} cues`);
writeFileSync(`${BUILD}timeline.json`, JSON.stringify({ total, clips: clips.map((c, k) => ({ id: c.id ?? (k === 0 ? 'title' : 'end'), start: starts[k], len: c.len, narration: c.lead !== undefined ? starts[k] + c.lead : null })) }, null, 1));
