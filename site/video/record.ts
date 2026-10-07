/**
 * Films the tour, one Playwright `recordVideo` clip per segment of script.md, at 1920x1080.
 *
 *   node video/record.ts                 # every segment, then the title and end cards
 *   ONLY=03,04 node video/record.ts      # just these segments
 *
 * Needs the narration first (tts.ts): every segment is paced by its audio. Actions are
 * scheduled against the narration's character alignment, so a ring appears as its words are
 * spoken. Writes build/clips/NN.webm and NN.json (the pieces that map audio time to video time
 * for make-video.sh), plus build/clips/title.png and end.png.
 *
 * Segment 3 sets the demo's paymentFailure flag (as the e2e crate does, through `make flag`),
 * films the Stories page until Tayga writes the new story, and runs `make flags-reset` in a
 * finally block; make-video.sh also traps EXIT with `make flags-reset`.
 *
 * Environment: TAYGA_URL (default http://localhost:8090), DOCS_URL (the built docs site served
 * by `astro preview`, default http://localhost:4321/tayga/), STORY_TIMEOUT_S (default 300).
 */
import { chromium } from 'playwright';
import type { Browser, BrowserContext, Locator, Page } from 'playwright';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, renameSync, writeFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { CLIPS, VIDEO_DIR, cue, narration, segments } from './segments.ts';
import type { Narration } from './segments.ts';

const BASE = (process.env.TAYGA_URL ?? 'http://localhost:8090').replace(/\/$/, '');
const API = `${BASE}/api/v1`;
const DOCS = process.env.DOCS_URL ?? 'http://localhost:4321/tayga/';
const STORY_TIMEOUT_S = Number(process.env.STORY_TIMEOUT_S ?? '300');
const ONLY = (process.env.ONLY ?? '').split(',').map((s) => s.trim()).filter(Boolean);
const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const UI_PUBLIC = `${ROOT}ui/public/`;
const VIEW = { width: 1920, height: 1080 };
/** Seconds of settled picture before the narration starts, and after it ends. */
const LEAD = 0.8;
const TAIL = 1.4;

mkdirSync(CLIPS, { recursive: true });

// ---------------------------------------------------------------------------------------------
// Live data.

async function getJson<T>(path: string): Promise<T> {
	const r = await fetch(`${API}${path}`);
	if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
	return (await r.json()) as T;
}

interface GroupJ {
	fingerprint: string;
	kind: string;
	summary: string;
	sample_story_id: string;
	rc_service: string;
}

/** The error story with the most spans in the last day: the "scroll through a hundred spans" trace. */
async function bigTrace(): Promise<string> {
	const groups = await getJson<GroupJ[]>('/story-groups?since=24h&kind=error');
	let best = { spans: 0, trace: '' };
	for (const g of groups.slice(0, 30)) {
		const s = await getJson<{ trace_id: string; span_count: number }>(`/stories/${g.sample_story_id}`).catch(() => null);
		if (!s || s.span_count <= best.spans) continue;
		const t = await fetch(`${API}/traces/${s.trace_id}`);
		if (t.ok) best = { spans: s.span_count, trace: s.trace_id };
	}
	if (!best.trace) throw new Error('no stored error trace in the last 24 h');
	console.log(`  big trace ${best.trace} (${best.spans} spans)`);
	return best.trace;
}

function make(...args: string[]): void {
	execFileSync('make', args, { cwd: ROOT, stdio: 'inherit' });
}

// ---------------------------------------------------------------------------------------------
// The overlay: a visible cursor, rings, smooth scrolling, the terminal and the elapsed badge.

const OVERLAY = () => {
	const css = `
.__tg-cursor{position:fixed;left:0;top:0;width:30px;height:30px;z-index:2147483647;pointer-events:none;transform:translate(-4px,-3px);filter:drop-shadow(0 3px 6px rgba(0,0,0,.45));transition:opacity .3s}
.__tg-click{position:fixed;width:44px;height:44px;margin:-22px 0 0 -22px;border-radius:50%;border:3px solid #7cc4ff;z-index:2147483646;pointer-events:none;animation:__tgclick .6s ease-out forwards}
@keyframes __tgclick{from{transform:scale(.3);opacity:1}to{transform:scale(1.6);opacity:0}}
.__tg-ring{position:fixed;z-index:2147483645;pointer-events:none;border:2.5px solid var(--__ring,#7cc4ff);border-radius:12px;box-shadow:0 0 0 5px color-mix(in srgb,var(--__ring,#7cc4ff) 18%,transparent),0 0 30px color-mix(in srgb,var(--__ring,#7cc4ff) 40%,transparent);opacity:0;transform:scale(1.04);transition:opacity .35s ease,transform .35s ease}
.__tg-ring.on{opacity:1;transform:scale(1)}
.__tg-term{position:fixed;right:40px;bottom:40px;width:760px;z-index:2147483640;border:1px solid #2e4266;border-radius:16px;background:rgba(7,12,24,.96);box-shadow:0 24px 60px rgba(0,0,0,.5);font:500 20px/1.5 'JetBrains Mono',ui-monospace,Menlo,monospace;color:#d8e1f0;overflow:hidden;opacity:0;transform:translateY(16px);transition:opacity .4s,transform .4s}
.__tg-term.on{opacity:1;transform:none}
.__tg-term .bar{height:40px;display:flex;align-items:center;gap:9px;padding:0 16px;background:#0e162a;border-bottom:1px solid #1c2944;font:600 16px Sora,sans-serif;color:#8e9bb3}
.__tg-term .bar i{width:12px;height:12px;border-radius:50%;background:#ff6b6b}.__tg-term .bar i:nth-child(2){background:#f4b740}.__tg-term .bar i:nth-child(3){background:#7fd1a8;margin-right:8px}
.__tg-term pre{margin:0;padding:16px 20px;font:inherit;white-space:pre-wrap}
.__tg-term .p{color:#7fd1a8}.__tg-term .o{color:#8e9bb3}
.__tg-badge{position:fixed;left:50%;top:76px;transform:translateX(-50%);z-index:2147483640;display:flex;gap:12px;align-items:center;padding:10px 20px;border-radius:999px;border:1px solid #2e4266;background:rgba(14,22,42,.95);box-shadow:0 12px 30px rgba(0,0,0,.45);font:600 19px Sora,sans-serif;color:#d8e1f0;opacity:0;transition:opacity .4s}
.__tg-badge.on{opacity:1}.__tg-badge b{font-family:'JetBrains Mono',monospace;color:#7cc4ff}.__tg-badge .dot{width:10px;height:10px;border-radius:50%;background:#ff6b6b;box-shadow:0 0 10px #ff6b6b}
.__tg-badge small{font:500 15px Sora,sans-serif;color:#8e9bb3}
`;
	const w = window as unknown as Record<string, unknown>;
	const install = () => {
		if (document.querySelector('.__tg-cursor')) return;
		const st = document.createElement('style');
		st.textContent = css;
		document.head.appendChild(st);
		const c = document.createElement('div');
		c.className = '__tg-cursor';
		c.innerHTML =
			'<svg viewBox="0 0 24 24" width="30" height="30"><path d="M4 2.5 L4 19.5 L8.6 15.4 L11.6 22 L14.6 20.7 L11.7 14.2 L18 14.2 Z" fill="#ffffff" stroke="#0a1020" stroke-width="1.4" stroke-linejoin="round"/></svg>';
		let pos = { x: 960, y: 600 };
		try {
			pos = JSON.parse(sessionStorage.getItem('__tgcur') ?? '') as typeof pos;
		} catch {
			/* first page */
		}
		if ((w.__tgHideCursor as boolean) === true) c.style.opacity = '0';
		c.style.left = `${pos.x}px`;
		c.style.top = `${pos.y}px`;
		document.body.appendChild(c);
		document.addEventListener(
			'mousemove',
			(e) => {
				c.style.left = `${e.clientX}px`;
				c.style.top = `${e.clientY}px`;
				try {
					sessionStorage.setItem('__tgcur', JSON.stringify({ x: e.clientX, y: e.clientY }));
				} catch {
					/* storage blocked */
				}
			},
			true,
		);
		document.addEventListener(
			'mousedown',
			(e) => {
				const k = document.createElement('div');
				k.className = '__tg-click';
				k.style.left = `${e.clientX}px`;
				k.style.top = `${e.clientY}px`;
				document.body.appendChild(k);
				setTimeout(() => k.remove(), 700);
			},
			true,
		);
	};
	if (document.body) install();
	else document.addEventListener('DOMContentLoaded', install);

	w.__tgRing = (el: Element, hold: number, color: string | null, pad: number) => {
		const r = document.createElement('div');
		r.className = '__tg-ring';
		if (color) r.style.setProperty('--__ring', color);
		document.body.appendChild(r);
		let alive = true;
		const tick = () => {
			if (!alive) return;
			const b = el.getBoundingClientRect();
			r.style.left = `${b.left - pad}px`;
			r.style.top = `${b.top - pad}px`;
			r.style.width = `${b.width + pad * 2}px`;
			r.style.height = `${b.height + pad * 2}px`;
			requestAnimationFrame(tick);
		};
		tick();
		requestAnimationFrame(() => requestAnimationFrame(() => r.classList.add('on')));
		setTimeout(() => {
			r.classList.remove('on');
			setTimeout(() => {
				alive = false;
				r.remove();
			}, 450);
		}, hold);
	};
	w.__tgScroll = (target: { el?: Element; y?: number; container?: Element | null }, ms: number, offset: number) =>
		new Promise<void>((done) => {
			const box = target.container ?? null;
			const cur = () => (box ? box.scrollTop : window.scrollY);
			const max = box ? box.scrollHeight - box.clientHeight : document.documentElement.scrollHeight - innerHeight;
			let y = target.y ?? 0;
			if (target.el) {
				const r = target.el.getBoundingClientRect();
				const top = box ? box.getBoundingClientRect().top : 0;
				y = cur() + r.top - top - offset;
			}
			y = Math.max(0, Math.min(max, y));
			const y0 = cur();
			const t0 = performance.now();
			const f = (now: number) => {
				const t = Math.min(1, (now - t0) / ms);
				const e = t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
				const v = y0 + (y - y0) * e;
				if (box) box.scrollTop = v;
				else window.scrollTo(0, v);
				if (t < 1) requestAnimationFrame(f);
				else done();
			};
			requestAnimationFrame(f);
		});
};

// ---------------------------------------------------------------------------------------------
// Segment harness.

type Theme = 'dark' | 'light' | 'system';

interface Piece {
	a0: number;
	a1: number;
	v0: number;
	v1: number;
}

class Seg {
	page!: Page;
	ctx!: BrowserContext;
	n: Narration;
	/** Epoch ms of the video's first frame, and of narration time 0. */
	t0 = 0;
	base = 0;
	cur = { x: 960, y: 600 };
	extra: Record<string, unknown> = {};
	pieces: Piece[] | null = null;

	id: string;
	browser: Browser;

	constructor(id: string, browser: Browser) {
		this.id = id;
		this.browser = browser;
		this.n = narration(id);
	}

	async open(theme: Theme = 'dark', opts: { hideCursor?: boolean; colorScheme?: 'dark' | 'light' } = {}): Promise<Page> {
		this.ctx = await this.browser.newContext({
			viewport: VIEW,
			deviceScaleFactor: 1,
			colorScheme: opts.colorScheme ?? 'dark',
			locale: 'en-GB',
			timezoneId: 'Europe/Warsaw',
			recordVideo: { dir: `${CLIPS}raw-${this.id}/`, size: VIEW },
		});
		await this.ctx.addInitScript(
			([t, hide]) => {
				try {
					localStorage.setItem('tayga-theme', t as string);
					localStorage.setItem('tayga-live', 'on');
					localStorage.setItem('starlight-theme', 'dark');
				} catch {
					/* storage blocked */
				}
				(window as unknown as Record<string, unknown>).__tgHideCursor = hide;
			},
			[theme, opts.hideCursor ?? false] as const,
		);
		await this.ctx.addInitScript(OVERLAY);
		await this.ctx.route('http://cards.local/**', async (route) => {
			const path = new URL(route.request().url()).pathname;
			const file = path.startsWith('/fonts/') || path === '/logo.svg' ? `${UI_PUBLIC}${path.slice(1)}` : `${VIDEO_DIR}cards${path}`;
			const type = path.endsWith('.css') ? 'text/css' : path.endsWith('.svg') ? 'image/svg+xml' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html';
			await route.fulfill({ body: readFileSync(file), contentType: type });
		});
		this.t0 = Date.now();
		this.page = await this.ctx.newPage();
		return this.page;
	}

	async goto(url: string, extra = 900): Promise<void> {
		await this.page.goto(url.startsWith('http') ? url : `${BASE}${url}`, { waitUntil: 'domcontentloaded' });
		await this.settle(extra);
	}

	async settle(extra = 900): Promise<void> {
		const p = this.page;
		await p.waitForLoadState('load').catch(() => {});
		await p
			.waitForFunction(
				() => document.querySelectorAll('[aria-busy="true"], [aria-label^="Loading"], [aria-label^="Laying out"]').length === 0,
				undefined,
				{ timeout: 20_000 },
			)
			.catch(() => console.warn(`  ! still loading on ${p.url()}`));
		await p.evaluate(() => document.fonts.ready);
		await p.waitForTimeout(extra);
	}

	/** Narration time 0 is now plus the lead-in. */
	async start(): Promise<void> {
		this.base = Date.now() + LEAD * 1000;
		await this.at(0);
	}

	cue(phrase: string): number {
		return cue(this.n, phrase);
	}

	/** Waits until narration time `t` seconds. */
	async at(t: number): Promise<void> {
		const ms = this.base + t * 1000 - Date.now();
		if (ms > 0) await this.page.waitForTimeout(ms);
		else if (ms < -1500) console.warn(`  ! segment ${this.id} running ${(-ms / 1000).toFixed(1)} s late at t=${t.toFixed(1)}`);
	}

	async on(phrase: string, early = 0.25): Promise<void> {
		await this.at(Math.max(0, this.cue(phrase) - early));
	}

	async move(x: number, y: number, ms = 750): Promise<void> {
		const from = { ...this.cur };
		const steps = Math.max(10, Math.round(ms / 16));
		for (let i = 1; i <= steps; i++) {
			const t = i / steps;
			const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
			await this.page.mouse.move(from.x + (x - from.x) * e, from.y + (y - from.y) * e);
			await this.page.waitForTimeout(ms / steps - 2);
		}
		this.cur = { x, y };
	}

	async moveTo(loc: Locator, ms = 750, fx = 0.5, fy = 0.5): Promise<void> {
		const b = await loc.first().boundingBox({ timeout: 8000 });
		if (!b) throw new Error(`segment ${this.id}: target not visible`);
		await this.move(b.x + b.width * fx, b.y + b.height * fy, ms);
	}

	async click(loc: Locator, ms = 750): Promise<void> {
		await this.moveTo(loc, ms);
		await this.page.waitForTimeout(180);
		await this.page.mouse.down();
		await this.page.waitForTimeout(90);
		await this.page.mouse.up();
	}

	async ring(loc: Locator, hold = 2600, color: string | null = null, pad = 6): Promise<void> {
		const h = await loc
			.first()
			.elementHandle({ timeout: 8000 })
			.catch(() => null);
		if (!h) {
			console.warn(`  ! segment ${this.id}: ring target not found`);
			return;
		}
		await h.evaluate((el, a) => (window as unknown as { __tgRing: (...x: unknown[]) => void }).__tgRing(el, a.hold, a.color, a.pad), { hold, color, pad });
	}

	async scrollTo(loc: Locator | number, ms = 1200, offset = 140, container?: Locator): Promise<void> {
		const box = container ? await container.first().elementHandle() : null;
		if (typeof loc === 'number') {
			await this.page.evaluate(
				(a) => (window as unknown as { __tgScroll: (...x: unknown[]) => Promise<void> }).__tgScroll({ y: a.y, container: a.box }, a.ms, 0),
				{ y: loc, ms, box },
			);
			return;
		}
		const h = await loc.first().elementHandle({ timeout: 8000 });
		await h!.evaluate(
			(el, a) => (window as unknown as { __tgScroll: (...x: unknown[]) => Promise<void> }).__tgScroll({ el, container: a.box }, a.ms, a.offset),
			{ ms, offset, box },
		);
	}

	/** Wheel scrolling over the pointer: for virtualized lists that own their scrolling. */
	async wheel(dy: number, ms: number): Promise<void> {
		const t0 = Date.now();
		let done = 0;
		while (Date.now() - t0 < ms) {
			const want = (dy * Math.min(1, (Date.now() - t0 + 50) / ms)) - done;
			if (want > 0) {
				await this.page.mouse.wheel(0, want);
				done += want;
			}
			await this.page.waitForTimeout(50);
		}
	}

	async finish(): Promise<void> {
		await this.at(this.n.duration + TAIL + 0.3);
		const video = this.page.video();
		await this.ctx.close();
		const raw = await video!.path();
		renameSync(raw, `${CLIPS}${this.id}.webm`);
		const start = (this.base - this.t0) / 1000;
		const pieces = this.pieces ?? [{ a0: -LEAD, a1: this.n.duration + TAIL, v0: start - LEAD, v1: start + this.n.duration + TAIL }];
		writeFileSync(`${CLIPS}${this.id}.json`, JSON.stringify({ lead: LEAD, tail: TAIL, duration: this.n.duration, pieces, ...this.extra }, null, 1));
		console.log(`  ${this.id}.webm  narration ${this.n.duration.toFixed(1)} s`);
	}
}

const panel = (p: Page, title: string | RegExp) =>
	p.locator('[data-variant="panel"]').filter({ has: p.getByRole('heading', { name: title, exact: typeof title === 'string' }) }).first();

const ERR = '#ff6b6b';

// ---------------------------------------------------------------------------------------------
// The segments.

type Run = (s: Seg) => Promise<void>;

const run: Record<string, Run> = {
	// 1. The problem: the manual way, a big trace scrolled by hand.
	async '01'(s) {
		const trace = await bigTrace();
		await s.open();
		await s.goto('/traces?since=1h', 1500);
		const p = s.page;
		await s.start();
		await s.move(700, 330, 1200);
		await s.move(1300, 300, 1400);
		await s.on('but not why', 0.1);
		const box = p.getByRole('search', { name: 'Open a trace by id' }).locator('input');
		await s.click(box, 700);
		await box.pressSequentially(trace, { delay: 25 });
		await s.on('So someone opens a trace', 0);
		await p.keyboard.press('Enter');
		await s.settle(200);
		await s.move(1000, 640, 500);
		await s.on('scrolls through');
		await s.wheel(2600, 1900);
		await s.on('goes hunting');
		await s.wheel(2400, 1900);
		await s.on('Tayga does that first pass', 0.6);
		await s.click(p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(0), 600);
		await s.settle(300);
		await s.finish();
	},

	// 2. Beside your stack: the architecture card.
	async '02'(s) {
		await s.open('dark', { hideCursor: true });
		await s.goto('http://cards.local/architecture.html', 600);
		const light = (...ids: string[]) => s.page.evaluate((x) => (window as unknown as { light: (...a: string[]) => void }).light(...x), ids);
		await s.start();
		await light('services');
		await s.on('OpenTelemetry Collector');
		await light('services', 'collector');
		await s.on('over OTLP');
		await light('collector', 'otlp', 'ingest');
		await s.on('Redpanda');
		await light('redpanda');
		await s.on('closes each trace');
		await light('assembler');
		await s.on('stores everything');
		await light('writer', 'clickhouse');
		await s.on('analyses every request');
		await light('assembler', 'logminer', 'api', 'notifier');
		await s.on('The core is open source');
		await light('license');
		await s.finish();
	},

	// 3. A live failure: the flag, the wait (time-lapse) and the new story group.
	async '03'(s) {
		const pre = await getJson<GroupJ[]>('/story-groups?since=15m&kind=error&service=payment');
		if (pre.length) throw new Error('a payment error story exists in the last 15 min; wait until it ages out so the new one is visibly new');
		await s.open();
		await s.goto('/?since=15m&kind=error', 1500);
		const p = s.page;
		try {
			await p.evaluate(() => {
				const t = document.createElement('div');
				t.className = '__tg-term';
				t.innerHTML = '<div class="bar"><i></i><i></i><i></i>terminal · tayga</div><pre></pre>';
				document.body.appendChild(t);
				const b = document.createElement('div');
				b.className = '__tg-badge';
				document.body.appendChild(b);
			});
			await s.start();
			await s.move(900, 420, 1200);
			await s.on('The OpenTelemetry demo');
			await p.evaluate(() => {
				const t = document.querySelector('.__tg-term')!;
				t.querySelector('pre')!.innerHTML = '<span class="p">$</span> ';
				t.classList.add('on');
			});
			await s.on('We turn on');
			const cmd = 'make flag NAME=paymentFailure VARIANT=100%';
			for (let i = 1; i <= cmd.length; i++) {
				await p.evaluate((x) => {
					document.querySelector('.__tg-term pre')!.innerHTML = `<span class="p">$</span> ${x}`;
				}, cmd.slice(0, i));
				await p.waitForTimeout(28);
			}
			make('flag', 'NAME=paymentFailure', 'VARIANT=100%');
			const flip = Date.now();
			await p.evaluate((f) => {
				document.querySelector('.__tg-term pre')!.innerHTML += '\n<span class="o">paymentFailure = 100%</span>';
				const b = document.querySelector('.__tg-badge')!;
				b.classList.add('on');
				const tick = () => {
					const sec = Math.floor((Date.now() - f) / 1000);
					b.innerHTML = `<span class="dot"></span>paymentFailure on · <b>${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}</b> <small>real time since the flip · time-lapse</small>`;
				};
				tick();
				(window as unknown as { __tgBadge: number }).__tgBadge = window.setInterval(tick, 200);
			}, flip);
			const flipA = (flip - s.base) / 1000;
			await s.on('and watch the Stories page');
			await s.ring(p.locator('section[aria-label="Story groups"]').first(), 3000);
			await s.move(1150, 330, 900);
			// The wait: Live refreshes every 10 s; the row appears when Tayga writes the story.
			const row = p.locator('[role="row"][data-fingerprint]').filter({ hasText: 'payment charge failed' }).first();
			const deadline = Date.now() + STORY_TIMEOUT_S * 1000;
			while (!(await row.isVisible().catch(() => false))) {
				if (Date.now() > deadline) throw new Error(`no payment story within ${STORY_TIMEOUT_S} s`);
				await p.waitForTimeout(200);
			}
			const appear = Date.now();
			const waited = (appear - flip) / 1000;
			console.log(`  payment story appeared ${waited.toFixed(0)} s after the flip`);
			await p.evaluate((w) => {
				const g = window as unknown as { __tgBadge: number };
				clearInterval(g.__tgBadge);
				const sec = Math.round(w);
				document.querySelector('.__tg-badge')!.innerHTML =
					`<span class="dot"></span>First story <b>${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}</b> <small>after the flip</small>`;
				document.querySelector('.__tg-term')!.classList.remove('on');
			}, waited);
			// From here, narration time follows the appearance: "And here it is" lands just after it.
			const h = s.cue('And here it is');
			const base0 = s.base;
			s.base = appear - (h - 0.6) * 1000;
			const fp = await row.getAttribute('data-fingerprint');
			await s.on('payment charge failed', 0.1);
			await s.ring(row, 3600, ERR, 4);
			await s.on('a new error story group');
			await s.click(row, 800);
			await s.page.waitForTimeout(400);
			const start0 = (base0 - s.t0) / 1000;
			const start1 = (s.base - s.t0) / 1000;
			const a1 = flipA + 2;
			const a2 = h - 0.6 - 2.5;
			s.pieces = [
				{ a0: -LEAD, a1, v0: start0 - LEAD, v1: start0 + a1 },
				{ a0: a1, a1: a2, v0: start0 + a1, v1: start1 + a2 },
				{ a0: a2, a1: s.n.duration + TAIL, v0: start1 + a2, v1: start1 + s.n.duration + TAIL },
			];
			s.extra = { group: fp, waited };
			await s.finish();
		} finally {
			make('flags-reset');
		}
	},

	// 4. The story page.
	async '04'(s) {
		const m = JSON.parse(readFileSync(`${CLIPS}03.json`, 'utf8')) as { group?: string };
		if (!m.group) throw new Error('segment 3 has no story group: record 03 first');
		await s.open();
		await s.goto(`/?since=1h&kind=error&group=${m.group}`, 1500);
		const p = s.page;
		// Select the group from segment 3 (a live refresh can reset the selection from the URL).
		await p.locator(`[role="row"][data-fingerprint="${m.group}"]`).click();
		await s.settle(800);
		const insp = p.locator('[aria-label="Selected story"]');
		await s.start();
		await s.moveTo(insp.locator('h2, h3').first(), 900);
		await s.on('Open the story', 0.1);
		await s.click(insp.getByRole('link', { name: /Open story/ }), 600);
		await s.settle(300);
		const header = p.locator('[data-variant="panel"]').filter({ has: p.locator('[aria-label="Request path"]') }).first();
		await s.on('the root cause');
		await s.ring(header.locator('p').first(), 3200, ERR);
		await s.moveTo(header.locator('p').first(), 700, 0.3);
		await s.on('Then the request path');
		await s.ring(header.locator('[aria-label="Request path"]'), 3000);
		await s.moveTo(header.locator('[aria-label="Request path"]'), 700, 0.8);
		await s.on('Compared with normal');
		await s.ring(panel(p, 'Compared with normal'), 4200);
		await s.moveTo(panel(p, 'Compared with normal'), 800, 0.3, 0.3);
		await s.on('The waterfall');
		await s.scrollTo(panel(p, 'Waterfall'), 1300, 80);
		await s.on('root-cause span selected', 0.4);
		const rc = p.locator('[data-root-cause="true"]').first();
		await s.ring(rc, 2600, ERR, 3);
		await s.moveTo(rc, 700, 0.35);
		await s.on('And below');
		await s.scrollTo(panel(p, 'Logs'), 1300, 80);
		await s.ring(p.getByRole('table', { name: 'Logs' }), 2400);
		await s.on('each linked');
		const tpl = p.getByRole('table', { name: 'Logs' }).locator('a[href*="/logs/templates/"]').first();
		if (await tpl.count()) {
			await s.moveTo(tpl, 700);
			await s.ring(tpl, 2400, null, 4);
		} else console.warn('  ! no template link in the logs table');
		await s.finish();
	},

	// 5. Story groups and the inspector.
	async '05'(s) {
		await s.open();
		await s.goto('/?since=24h', 1500);
		const p = s.page;
		const rows = p.locator('[role="row"][data-fingerprint]');
		const first = rows.first();
		await s.start();
		await s.moveTo(first, 900, 0.3);
		await s.ring(first, 2600);
		await s.on('Numbers, IDs');
		await s.ring(first.locator('[role="gridcell"]').nth(1), 2400);
		await s.on('with a count');
		await s.ring(first.locator('[role="gridcell"]').nth(3), 1800);
		await s.on('and a trend');
		await s.ring(first.locator('[role="gridcell"]').nth(2), 1800);
		await s.on('The inspector');
		await s.ring(p.locator('[aria-label="Selected story"]'), 2600);
		await s.on('triage with the arrow keys', 0.6);
		await s.click(first, 600);
		for (let i = 0; i < 3; i++) {
			await p.waitForTimeout(900);
			await p.keyboard.press('ArrowDown');
		}
		await s.finish();
	},

	// 6. The service map.
	async '06'(s) {
		await s.open();
		await s.goto('/map?since=15m', 2200);
		const p = s.page;
		const node = (name: string) => p.locator('.react-flow__node').filter({ has: p.getByText(name, { exact: true }) }).first();
		const toolbar = p.locator('[data-variant="panel"]').filter({ has: p.getByRole('heading', { name: 'Service map' }) }).first();
		await s.start();
		await s.ring(toolbar.locator('h2').locator('xpath=following-sibling::*[1]'), 2600);
		await s.moveTo(node('frontend-proxy'), 1000);
		await s.on('with its call rate');
		await s.moveTo(node('frontend'), 800);
		await s.ring(node('frontend'), 2400);
		await s.on('A service turns red');
		await s.moveTo(node('payment'), 900);
		await s.ring(node('payment'), 3000, ERR);
		await s.on('and amber');
		await s.moveTo(node('checkout'), 800);
		await s.on('Select one');
		await s.click(node('payment'), 500);
		await p.waitForTimeout(900);
		await s.on('the stories whose');
		await s.ring(p.getByRole('dialog').getByText(/stories with root cause here/i).locator('xpath=..'), 2600);
		await s.finish();
	},

	// 7. Log alerts and a template.
	async '07'(s) {
		await s.open();
		await s.goto('/logs/alerts?since=7d', 1500);
		const p = s.page;
		const table = p.locator('table').first();
		await s.start();
		await s.moveTo(table.locator('tbody tr').first(), 1000, 0.3);
		await s.on('become templates');
		await s.ring(table.locator('tbody tr').first().locator('td').nth(2), 2400);
		await s.on('It alerts on a new template', 0.1);
		await s.ring(table.getByText('new', { exact: true }).first(), 1500);
		await s.on('a spike above');
		await s.ring(table.getByText('spike', { exact: true }).first(), 1500);
		await s.on('and silence');
		await s.ring(table.getByText('silence', { exact: true }).first(), 1900, '#b79cff');
		const pay = table.locator('tbody tr').filter({ hasText: 'Payment request failed' }).first();
		await s.on('Each alert links');
		await s.ring(pay.locator('td').last(), 2200);
		await s.moveTo(pay.locator('td').last().locator('a').first(), 700);
		await s.on('and the notifier', 0.6);
		await s.click(pay.locator('a[href*="/logs/templates/"]').first(), 600);
		await s.settle(300);
		await s.ring(panel(p, /^Hits per/), 2400);
		await s.on('with retries', 0.4);
		const sil = p.locator('[data-variant="panel"]').filter({ has: p.getByRole('switch') }).last();
		await s.scrollTo(sil, 1100, 260);
		await s.ring(sil.getByRole('switch'), 2000, '#b79cff');
		await s.moveTo(sil.getByRole('switch'), 600);
		await s.finish();
	},

	// 8. Pipeline health and the theme switch.
	async '08'(s) {
		await s.open('system');
		await s.goto('/pipeline', 2500);
		const p = s.page;
		await s.start();
		await s.on('The Pipeline page');
		await s.ring(p.getByRole('list', { name: 'Job status' }), 2600);
		await s.moveTo(p.getByRole('list', { name: 'Job status' }), 900, 0.2);
		await s.on('charts every stage');
		await s.scrollTo(panel(p, 'Open traces'), 2000, 120);
		await s.on('reads consumer lag');
		await s.scrollTo(panel(p, 'Consumer lag'), 1400, 120);
		await s.ring(panel(p, 'Consumer lag'), 2600);
		await s.on('Prefer a light theme', 0.8);
		await s.scrollTo(0, 900);
		await s.moveTo(p.getByRole('button', { name: /^Theme:/ }), 700);
		await s.on('One click', 0.1);
		await s.click(p.getByRole('button', { name: /^Theme:/ }), 200);
		await s.finish();
	},

	// 9. Deployment: the terminal card.
	async '09'(s) {
		await s.open('dark', { hideCursor: true });
		await s.goto('http://cards.local/terminal.html', 600);
		const play = (i: number, ms: number) =>
			s.page.evaluate((a) => {
				void (window as unknown as { play: (i: number, ms: number) => Promise<void> }).play(a.i, a.ms);
			}, { i, ms });
		const k = s.cue('On Kubernetes');
		const d = s.cue('Or run it next');
		await s.start();
		await play(0, (k - 0.6) * 1000);
		await s.on('On Kubernetes', 0.2);
		await play(1, (d - k - 0.4) * 1000);
		await s.on('Or run it next', 0.2);
		await play(2, (s.n.duration - d) * 1000);
		await s.finish();
	},

	// 10. Enterprise and the docs site.
	async '10'(s) {
		await s.open();
		await s.goto(DOCS, 1200);
		const p = s.page;
		// The docs site is laid out for reading, not for 1080p video: enlarge it.
		// Zoom the page's own elements, not the overlay, so the cursor and rings stay in viewport px.
		await p.evaluate(() => {
			for (const el of document.body.children) if (!/__tg-/.test(el.className.toString()) && el.tagName !== 'SCRIPT') (el as HTMLElement).style.zoom = '1.4';
		});
		await p.waitForTimeout(300);
		await p.evaluate(() => {
			const e = document.querySelector('.tg-ent')!;
			window.scrollTo(0, window.scrollY + e.getBoundingClientRect().top - 120);
		});
		await p.waitForTimeout(500);
		await s.start();
		await s.moveTo(p.locator('.tg-ent h2'), 900, 0.4);
		await s.on('single sign-on');
		await s.ring(p.locator('.tg-ent__list'), 4200);
		await s.on('Write to');
		await s.moveTo(p.locator('.tg-ent a[href^="mailto:"]'), 700);
		await s.ring(p.locator('.tg-ent a[href^="mailto:"]'), 2600, null, 5);
		await s.on('head to the docs site', 0.5);
		await s.scrollTo(p.locator('.tg-close'), 1400, 140);
		await s.moveTo(p.locator('.tg-close a').first(), 700);
		await s.ring(p.locator('.tg-close__actions'), 3000, null, 8);
		await s.finish();
	},
};

async function cards(browser: Browser): Promise<void> {
	const ctx = await browser.newContext({ viewport: VIEW, deviceScaleFactor: 1 });
	await ctx.route('http://cards.local/**', async (route) => {
		const path = new URL(route.request().url()).pathname;
		const file = path.startsWith('/fonts/') || path === '/logo.svg' ? `${UI_PUBLIC}${path.slice(1)}` : `${VIDEO_DIR}cards${path}`;
		const type = path.endsWith('.css') ? 'text/css' : path.endsWith('.svg') ? 'image/svg+xml' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html';
		await route.fulfill({ body: readFileSync(file), contentType: type });
	});
	const page = await ctx.newPage();
	for (const name of ['title', 'end']) {
		await page.goto(`http://cards.local/${name}.html`);
		await page.evaluate(() => document.fonts.ready);
		await page.waitForTimeout(300);
		await page.screenshot({ path: `${CLIPS}${name}.png` });
		console.log(`  ${name}.png`);
	}
	await ctx.close();
}

const browser = await chromium.launch();
try {
	const ids = segments().map((s) => s.id);
	for (const id of ids) {
		if (ONLY.length && !ONLY.includes(id)) continue;
		console.log(`segment ${id}`);
		await run[id](new Seg(id, browser));
	}
	if (!ONLY.length || ONLY.includes('cards')) await cards(browser);
	if (!existsSync(`${CLIPS}title.png`)) await cards(browser);
} finally {
	await browser.close();
}
