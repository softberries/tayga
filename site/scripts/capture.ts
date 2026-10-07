/**
 * Screenshot capture for the docs site: `npm --prefix site run capture`.
 *
 * Drives the live Tayga app (TAYGA_URL, default http://localhost:8090) with Playwright and
 * writes optimized WebP images plus `manifest.json` to `src/assets/screens/`. Every shot is
 * taken at deviceScaleFactor 2 and scaled down only as far as needed to stay near 200 KB.
 *
 * Real data: run it after `make e2e` (or after flipping the e2e failure flags and waiting for
 * stories), then `make flags-reset`. The script only reads from the app. It never saves a
 * setting: the silence toggle is flipped in the form and the shot is taken before Save.
 * The login page, the empty states and the error banner are captured with mocked API
 * responses (Playwright routing), and the manifest says so.
 *
 * Options (environment):
 *   TAYGA_URL      app base URL
 *   ONLY           comma-separated substrings; only shots whose name contains one are taken
 *   THEMES         `dark,light` (default both)
 *   OUT_DIR        write somewhere else (for trial runs)
 *   KEEP_MANIFEST  `1` merges into the existing manifest instead of replacing it (with ONLY)
 */
import { chromium } from 'playwright';
import type { Browser, BrowserContext, Locator, Page } from 'playwright';
import sharp from 'sharp';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const BASE = (process.env.TAYGA_URL ?? 'http://localhost:8090').replace(/\/$/, '');
const API = `${BASE}/api/v1`;
const OUT = (process.env.OUT_DIR ? `${process.env.OUT_DIR.replace(/\/$/, '')}/` : fileURLToPath(new URL('../src/assets/screens/', import.meta.url)));
const ONLY = (process.env.ONLY ?? '').split(',').map((s) => s.trim()).filter(Boolean);
const THEMES = (process.env.THEMES ?? 'dark,light').split(',').map((s) => s.trim()) as Theme[];
const DESKTOP = { width: 1440, height: 900 };
const MOBILE = { width: 390, height: 844 };
const DPR = 2;
/** Target size per image; larger shots step quality down, then resolution. */
const BUDGET = 200 * 1024;
/** The site's existing imports (PNG, same names): the hero and landing images. */
const PNG_ALIASES: Record<string, string> = {
	'stories-home-dark': 'stories-dark.png',
	'story-detail-dark': 'story-dark.png',
	'map-overview-dark': 'map-dark.png',
	'hero-story-dark': 'hero-dark.png',
};

type Theme = 'dark' | 'light';
type Kind = 'page' | 'fullpage' | 'fragment' | 'mobile';

interface Callout {
	n: number;
	label: string;
	target: (p: Page) => Locator;
	/** With a left gutter: put this badge in a right gutter instead. */
	right?: boolean;
}

interface Entry {
	file: string;
	page: string;
	kind: Kind;
	route: string;
	state: string;
	theme: Theme;
	description: string;
	callouts: { n: number; label: string }[];
	width: number;
	height: number;
	cssWidth: number;
	cssHeight: number;
	bytes: number;
	data: 'live' | 'mocked';
	capturedAt: string;
}

interface Data {
	hero: { storyId: string; traceId: string; fingerprint: string; rcSpanId: string | null; summary: string };
	slow: { storyId: string; summary: string } | null;
	homeGroup: string;
	/** A second, different group for the selected-group shots. */
	altGroup: string;
	template: { id: string; service: string; text: string };
	silenceTemplate: string | null;
	mapService: string;
	alertsSince: string;
}

const manifest: Entry[] = [];

// ---------------------------------------------------------------------------------------------
// Data discovery: pick the most telling story, template and alerts from the live API.

async function getJson<T>(path: string): Promise<T> {
	const r = await fetch(`${API}${path}`);
	if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
	return (await r.json()) as T;
}

interface GroupJ {
	fingerprint: string;
	kind: string;
	rc_service: string;
	stories: number;
	sample_story_id: string;
	summary: string;
}
interface StoryJ {
	story_id?: string;
	trace_id: string;
	kind: string;
	summary: string;
	path_services: string[];
	span_count: number;
	root_cause: { span_id?: string; service: string };
	baseline_diff: { new_ops: unknown[]; missing_ops: unknown[]; slower_ops: unknown[] } | null;
	critical_path: { top: unknown[] };
	also_failed: unknown[];
}
interface AlertJ {
	kind: string;
	template_id: string;
	service: string;
	template: string;
	example_traces: { story_id: string | null }[];
}
interface TemplateJ {
	template_id: string;
	service: string;
	template: string;
	count: number;
}

/** Infrastructure noise (flagd stream timeouts) is real but not what the docs should lead with. */
const NOISE = /flagd|EventStream/;

function storyScore(s: StoryJ, logs: number): number {
	const d = s.baseline_diff;
	const diff = d ? d.new_ops.length + d.missing_ops.length + d.slower_ops.length : 0;
	return (
		Math.min(s.path_services.length, 6) * 4 +
		Math.min(s.critical_path.top.length, 5) * 2 +
		(diff > 0 ? 6 : 0) +
		(logs > 0 ? 6 : 0) +
		Math.min(s.span_count, 40) / 8 +
		s.also_failed.length
	);
}

async function discover(): Promise<Data> {
	const groups = await getJson<GroupJ[]>('/story-groups?since=1h');
	const errors = groups.filter((g) => g.kind === 'error' && !NOISE.test(g.summary));
	const slows = groups.filter((g) => g.kind === 'slow');
	let best: { s: StoryJ; id: string; g: GroupJ; score: number } | null = null;
	for (const g of errors.slice(0, 12)) {
		const s = await getJson<StoryJ>(`/stories/${g.sample_story_id}`);
		const t = await getJson<{ logs: unknown[] }>(`/traces/${s.trace_id}`).catch(() => ({ logs: [] }));
		const score = storyScore(s, t.logs.length);
		if (!best || score > best.score) best = { s, id: g.sample_story_id, g, score };
	}
	if (!best) throw new Error('no error stories in the last hour: run `make e2e` (or flip a failure flag) first');
	let slow: Data['slow'] = null;
	let slowScore = -1;
	for (const g of slows.slice(0, 10)) {
		const s = await getJson<StoryJ>(`/stories/${g.sample_story_id}`);
		const score = storyScore(s, 0) + (s.baseline_diff?.slower_ops.length ?? 0) * 3;
		if (score > slowScore) {
			slowScore = score;
			slow = { storyId: g.sample_story_id, summary: s.summary };
		}
	}
	const alerts = await getJson<AlertJ[]>('/log-alerts?since=24h');
	const kinds = new Set(alerts.map((a) => a.kind));
	const alertsSince = kinds.size >= 3 ? '24h' : '7d';
	// The template: the spike with linked stories, else the busiest error-ish template.
	const spike =
		alerts.find((a) => a.kind === 'spike' && /payment/i.test(a.service) && a.example_traces.some((t) => t.story_id)) ??
		alerts.find((a) => a.kind === 'spike' && a.example_traces.some((t) => t.story_id)) ??
		alerts.find((a) => a.kind === 'spike');
	let template: Data['template'];
	if (spike) template = { id: spike.template_id, service: spike.service, text: spike.template };
	else {
		const ts = await getJson<TemplateJ[]>('/log-templates?since=1h');
		const t = ts.find((x) => /fail|error/i.test(x.template)) ?? ts[0];
		if (!t) throw new Error('no log templates');
		template = { id: t.template_id, service: t.service, text: t.template };
	}
	const silence = alerts.find((a) => a.kind === 'silence');
	const rcRow = best.s.root_cause.span_id ?? null;
	const mapService = best.s.path_services.includes('checkout') ? 'checkout' : best.s.root_cause.service;
	return {
		hero: { storyId: best.id, traceId: best.s.trace_id, fingerprint: best.g.fingerprint, rcSpanId: rcRow, summary: best.s.summary },
		slow,
		homeGroup: best.g.fingerprint,
		altGroup: (errors.find((g) => g.fingerprint !== best!.g.fingerprint && g.rc_service !== best!.g.rc_service) ?? slows[0] ?? best.g).fingerprint,
		template,
		silenceTemplate: silence?.template_id ?? null,
		mapService,
		alertsSince,
	};
}

// ---------------------------------------------------------------------------------------------
// Browser helpers.

/** Turns transitions and CSS animations off and hides the caret, so shots are deterministic. */
const STILL_CSS = `
*, *::before, *::after { transition: none !important; animation-delay: 0s !important; animation-duration: 0s !important; caret-color: transparent !important; }
::-webkit-scrollbar { width: 0 !important; height: 0 !important; }
`;

async function newContext(browser: Browser, theme: Theme, viewport = DESKTOP): Promise<BrowserContext> {
	const ctx = await browser.newContext({
		viewport,
		deviceScaleFactor: DPR,
		reducedMotion: 'reduce',
		colorScheme: theme,
		locale: 'en-GB',
		timezoneId: 'UTC',
		isMobile: viewport.width < 600,
		hasTouch: viewport.width < 600,
	});
	await ctx.addInitScript(
		([t, css]) => {
			try {
				localStorage.setItem('tayga-theme', t);
				localStorage.setItem('tayga-live', 'on');
			} catch {
				/* storage blocked */
			}
			const add = () => {
				const s = document.createElement('style');
				s.dataset.capture = 'still';
				s.textContent = css;
				document.head.appendChild(s);
			};
			if (document.head) add();
			else document.addEventListener('DOMContentLoaded', add);
		},
		[theme, STILL_CSS] as const,
	);
	return ctx;
}

/** Waits until nothing is loading: no busy regions, fonts loaded, charts drawn. */
async function settle(page: Page, extra = 900): Promise<void> {
	await page.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
	await page
		.waitForFunction(
			() =>
				document.querySelectorAll('[aria-busy="true"], [aria-label^="Loading"], [aria-label^="Laying out"]').length === 0,
			undefined,
			{ timeout: 20_000 },
		)
		.catch(() => console.warn(`  ! still loading on ${page.url()}`));
	await page.evaluate(() => document.fonts.ready);
	await page.waitForTimeout(extra);
}

async function open(page: Page, route: string, extra?: number): Promise<void> {
	await page.goto(`${BASE}${route}`, { waitUntil: 'domcontentloaded' });
	await settle(page, extra);
}

/** A Card panel by its uppercase PanelTitle. */
function panel(page: Page, title: string | RegExp): Locator {
	return page
		.locator('[data-variant="panel"]')
		.filter({ has: page.getByRole('heading', { name: title, exact: typeof title === 'string' }) })
		.first();
}

// ---------------------------------------------------------------------------------------------
// Callouts: numbered badges composited in a gutter beside (left) or above (top) the fragment,
// each joined by a leader line to an outline round its target, so no content is covered.

interface Box {
	n: number;
	right?: boolean;
	x: number;
	y: number;
	w: number;
	h: number;
}

async function calloutBoxes(page: Page, items: Callout[], origin: { x: number; y: number }): Promise<Box[]> {
	const out: Box[] = [];
	for (const c of items) {
		const b = await c
			.target(page)
			.first()
			.boundingBox({ timeout: 4000 })
			.catch(() => null);
		if (!b) {
			console.warn(`  ! callout ${c.n} (${c.label}) not found; skipped`);
			continue;
		}
		out.push({ n: c.n, right: c.right, x: b.x - origin.x, y: b.y - origin.y, w: b.width, h: b.height });
	}
	return out;
}

async function themeColors(page: Page): Promise<{ ground: string; accent: string; badge: string; onBadge: string; ring: string }> {
	return page.evaluate(() => {
		const cs = getComputedStyle(document.documentElement);
		const v = (n: string) => cs.getPropertyValue(n).trim();
		return { ground: v('--tg-ground'), accent: v('--tg-accent'), badge: v('--tg-accent-strong'), onBadge: v('--tg-on-accent'), ring: v('--tg-panel') };
	});
}

const GUTTER = 40;

/** Composites the gutter, badges, leader lines and target outlines onto a fragment (CSS px in, DPR out). */
async function annotate(
	png: Buffer,
	cssW: number,
	cssH: number,
	boxes: Box[],
	gutter: 'left' | 'top',
	c: Awaited<ReturnType<typeof themeColors>>,
): Promise<{ png: Buffer; cssW: number; cssH: number }> {
	const gx = gutter === 'left' ? GUTTER : 0;
	const gy = gutter === 'top' ? GUTTER : 0;
	const gr = gutter === 'left' && boxes.some((b) => b.right) ? GUTTER : 0;
	const W = cssW + gx + gr;
	let H = cssH + gy;
	let maxCy = 0;
	const R = 11;
	// Badge centres, spread so they never overlap.
	const sorted = [...boxes].sort((a, b) => (gutter === 'left' ? a.y + a.h / 2 - (b.y + b.h / 2) : a.x + a.w / 2 - (b.x + b.w / 2)));
	let prev = -Infinity;
	let prevR = -Infinity;
	const parts: string[] = [];
	for (const b of sorted) {
		const tx = b.x + gx;
		const ty = b.y + gy;
		let cx: number;
		let cy: number;
		let lx: number;
		let ly: number;
		if (gutter === 'left' && b.right) {
			cx = W - GUTTER / 2;
			cy = Math.min(Math.max(ty + Math.min(b.h / 2, 14), R + 2), H - R - 2);
			cy = Math.max(cy, prevR + 2 * R + 4);
			prevR = cy;
			maxCy = Math.max(maxCy, cy);
			lx = tx + b.w + 3;
			ly = Math.min(Math.max(cy, ty), ty + b.h);
		} else if (gutter === 'left') {
			cx = GUTTER / 2;
			cy = Math.min(Math.max(ty + Math.min(b.h / 2, 14), R + 2), H - R - 2);
			cy = Math.max(cy, prev + 2 * R + 4);
			prev = cy;
			maxCy = Math.max(maxCy, cy);
			lx = tx - 3;
			ly = Math.min(Math.max(cy, ty), ty + b.h);
		} else {
			cy = GUTTER / 2;
			cx = Math.min(Math.max(tx + Math.min(b.w / 2, 18), R + 2), W - R - 2);
			cx = Math.max(cx, prev + 2 * R + 4);
			prev = cx;
			ly = ty - 3;
			lx = Math.min(Math.max(cx, tx), tx + b.w);
		}
		parts.push(
			`<rect x="${tx - 3}" y="${ty - 3}" width="${b.w + 6}" height="${b.h + 6}" rx="7" fill="none" stroke="${c.accent}" stroke-width="1.5" stroke-dasharray="4 3" opacity="0.9"/>`,
			`<path d="M${cx} ${cy} L${lx} ${ly}" stroke="${c.accent}" stroke-width="1.25" opacity="0.8"/>`,
			`<circle cx="${cx}" cy="${cy}" r="${R}" fill="${c.badge}" stroke="${c.ground}" stroke-width="2"/>`,
			`<text x="${cx}" y="${cy + 4.2}" text-anchor="middle" font-family="Sora, Helvetica, Arial, sans-serif" font-weight="700" font-size="12" fill="${c.onBadge}">${b.n}</text>`,
		);
	}
	// Badges pushed down past the bottom grow the canvas.
	H = Math.max(H, Math.ceil(maxCy + R + 3));
	const svg = Buffer.from(
		`<svg xmlns="http://www.w3.org/2000/svg" width="${W * DPR}" height="${H * DPR}" viewBox="0 0 ${W} ${H}">${parts.join('')}</svg>`,
	);
	const out = await sharp({ create: { width: W * DPR, height: H * DPR, channels: 4, background: c.ground } })
		.composite([
			{ input: png, left: gx * DPR, top: gy * DPR },
			{ input: svg, left: 0, top: 0 },
		])
		.png()
		.toBuffer();
	return { png: out, cssW: W, cssH: H };
}

/** An invisible fixed box covering several elements, to shoot them as one fragment. */
async function union(page: Page, locs: Locator[], grow = 0): Promise<Locator> {
	const bs = (await Promise.all(locs.map((l) => l.first().boundingBox()))).filter((b) => b !== null);
	const x = Math.min(...bs.map((b) => b.x)) - grow;
	const y = Math.min(...bs.map((b) => b.y)) - grow;
	const r = Math.max(...bs.map((b) => b.x + b.width)) + grow;
	const btm = Math.max(...bs.map((b) => b.y + b.height)) + grow;
	await page.evaluate(
		(q) => {
			document.querySelector('[data-capture="union"]')?.remove();
			const n = document.createElement('div');
			n.dataset.capture = 'union';
			n.style.cssText = `position:absolute;left:${q.x + window.scrollX}px;top:${q.y + window.scrollY}px;width:${q.w}px;height:${q.h}px;pointer-events:none;`;
			document.body.appendChild(n);
		},
		{ x, y, w: r - x, h: btm - y },
	);
	return page.locator('[data-capture="union"]');
}

// ---------------------------------------------------------------------------------------------
// Encoding and the manifest.

async function encode(png: Buffer): Promise<{ buf: Buffer; w: number; h: number }> {
	const meta = await sharp(png).metadata();
	const W = meta.width ?? 0;
	let last: { buf: Buffer; w: number; h: number } | null = null;
	for (const scale of [1, 0.85, 0.75, 0.65]) {
		const w = Math.round(W * scale);
		for (const quality of [85, 80, 74, 68]) {
			const out = await sharp(png)
				.resize({ width: w, kernel: 'lanczos3' })
				.webp({ quality, smartSubsample: true, effort: 6 })
				.toBuffer({ resolveWithObject: true });
			last = { buf: out.data, w: out.info.width, h: out.info.height };
			if (out.data.length <= BUDGET) return last;
		}
	}
	return last!;
}

interface Meta {
	page: string;
	kind: Kind;
	route: string;
	state: string;
	description: string;
	callouts?: Callout[];
	/** Where the callout badges go. */
	gutter?: 'left' | 'top';
	data?: 'live' | 'mocked';
}

function wanted(name: string): boolean {
	return ONLY.length === 0 || ONLY.some((o) => name.includes(o));
}

async function save(name: string, theme: Theme, png: Buffer, cssW: number, cssH: number, m: Meta): Promise<void> {
	const { buf, w, h } = await encode(png);
	const file = `${name}-${theme}.webp`;
	await writeFile(`${OUT}${file}`, buf);
	const alias = PNG_ALIASES[`${name}-${theme}`];
	if (alias) {
		const p = await sharp(png).png({ compressionLevel: 9, palette: true, quality: 95, effort: 10 }).toBuffer();
		await writeFile(`${OUT}${alias}`, p);
	}
	manifest.push({
		file,
		page: m.page,
		kind: m.kind,
		route: m.route,
		state: m.state,
		theme,
		description: m.description,
		callouts: (m.callouts ?? []).map(({ n, label }) => ({ n, label })),
		width: w,
		height: h,
		cssWidth: cssW,
		cssHeight: cssH,
		bytes: buf.length,
		data: m.data ?? 'live',
		capturedAt: new Date().toISOString(),
	});
	console.log(`  ${file}  ${w}x${h}  ${(buf.length / 1024).toFixed(0)} KB${alias ? `  (+ ${alias})` : ''}`);
}

/** Viewport shot of the current page. */
async function shotPage(page: Page, name: string, theme: Theme, m: Meta): Promise<void> {
	const vp = page.viewportSize()!;
	const png = await page.screenshot({ type: 'png' });
	await save(name, theme, png, vp.width, vp.height, m);
}

/** Whole scrolling page. */
async function shotFull(page: Page, name: string, theme: Theme, m: Meta): Promise<void> {
	const png = await page.screenshot({ type: 'png', fullPage: true });
	const meta = await sharp(png).metadata();
	await save(name, theme, png, Math.round((meta.width ?? 0) / DPR), Math.round((meta.height ?? 0) / DPR), m);
}

/**
 * Element shot with some margin, after scrolling it below the sticky header. A target taller
 * than the viewport grows the viewport for the shot.
 */
async function shotEl(page: Page, target: Locator, name: string, theme: Theme, m: Meta, pad = 14): Promise<void> {
	if (m.callouts?.length) pad = Math.min(pad, 8);
	const el = target.first();
	await el.waitFor({ state: 'visible', timeout: 15_000 });
	const vp = page.viewportSize()!;
	let b = await el.boundingBox();
	if (!b) throw new Error(`${name}: not visible`);
	const header = await page.evaluate(() => {
		const h = document.querySelector('header');
		const s = h ? getComputedStyle(h).position : '';
		return h && (s === 'sticky' || s === 'fixed') ? h.getBoundingClientRect().height : 0;
	});
	const room = vp.height - header - pad * 2 - 8;
	let grown = false;
	if (b.height > room) {
		await page.setViewportSize({ width: vp.width, height: Math.ceil(b.height + header + pad * 2 + 40) });
		grown = true;
		await page.waitForTimeout(400);
	}
	await el.evaluate((node, top) => {
		const r = node.getBoundingClientRect();
		const y = window.scrollY + r.top - top;
		window.scrollTo({ top: Math.max(0, y), behavior: 'instant' as ScrollBehavior });
	}, header + pad + 8);
	await page.waitForTimeout(250);
	b = (await el.boundingBox())!;
	const cur = page.viewportSize()!;
	const x = Math.max(0, b.x - pad);
	const y = Math.max(0, b.y - pad);
	const clip = {
		x,
		y,
		width: Math.min(cur.width - x, b.width + pad * 2),
		height: Math.min(cur.height - y, b.height + pad * 2),
	};
	let png = await page.screenshot({ type: 'png', clip });
	let cssW = Math.round(clip.width);
	let cssH = Math.round(clip.height);
	if (m.callouts?.length) {
		const boxes = await calloutBoxes(page, m.callouts, clip);
		const a = await annotate(png, cssW, cssH, boxes, m.gutter ?? 'left', await themeColors(page));
		png = a.png;
		cssW = a.cssW;
		cssH = a.cssH;
	}
	if (grown) {
		await page.setViewportSize(vp);
		await page.waitForTimeout(300);
	}
	await page.evaluate(() => window.scrollTo({ top: 0, behavior: 'instant' as ScrollBehavior }));
	await save(name, theme, png, cssW, cssH, m);
}

async function hasScroll(page: Page): Promise<boolean> {
	return page.evaluate(() => document.documentElement.scrollHeight > window.innerHeight + 4);
}

// ---------------------------------------------------------------------------------------------
// The shots. Each scene opens one route and takes its page and fragment shots.

interface Scene {
	name: string;
	run: (page: Page, theme: Theme, d: Data, ctx: BrowserContext) => Promise<void>;
	mobile?: boolean;
}

/** A page shot plus, when the page scrolls, a full-page one. */
async function pageAndFull(page: Page, name: string, theme: Theme, m: Omit<Meta, 'kind'>): Promise<void> {
	if (wanted(name)) await shotPage(page, name, theme, { ...m, kind: 'page' });
	if (wanted(`${name}-full`) && (await hasScroll(page)))
		await shotFull(page, `${name}-full`, theme, { ...m, kind: 'fullpage', description: `${m.description} (whole page)` });
}

async function frag(page: Page, target: Locator, name: string, theme: Theme, m: Omit<Meta, 'kind'>, pad?: number): Promise<void> {
	if (!wanted(name)) return;
	try {
		await shotEl(page, target, name, theme, { ...m, kind: 'fragment' }, pad);
	} catch (e) {
		console.warn(`  ! ${name}-${theme}: ${(e as Error).message.split('\n')[0]}`);
	}
}

const header = (p: Page) => p.locator('header').first();

const scenes: Scene[] = [
	{
		name: 'shell',
		async run(page, theme) {
			await open(page, '/');
			await frag(page, header(page), 'shell-header', theme, {
				page: 'shell',
				route: '/',
				state: 'header on the Stories page',
				description: 'The header shown on every page: page title, command palette trigger, time range, live toggle and theme switch.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Page title and breadcrumb', target: (p) => p.getByRole('navigation', { name: 'Breadcrumb' }) },
					{ n: 2, label: 'Command palette (⌘K / Ctrl+K)', target: (p) => p.locator('header button').filter({ hasText: 'Jump to' }) },
					{ n: 3, label: 'Time range presets', target: (p) => p.locator('[aria-label="Time range"]') },
					{ n: 4, label: 'Custom time range', target: (p) => p.getByRole('button', { name: /Custom time range/ }) },
					{ n: 5, label: 'Live: refresh every 10 s', target: (p) => p.getByRole('button', { name: 'Live' }) },
					{ n: 6, label: 'Theme switch (light, dark, system)', target: (p) => p.getByRole('button', { name: /^Theme:/ }) },
				],
			}, 10);
			const railBox = await union(page, [page.getByRole('navigation', { name: 'Main' }).locator('img'), page.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(4)], 4);
			await frag(page, railBox, 'shell-rail', theme, {
				page: 'shell',
				route: '/',
				state: 'navigation rail, Stories active',
				description: 'The navigation rail: Stories, Traces, Service map, Logs and Pipeline.',
				callouts: [
					{ n: 1, label: 'Stories', target: (p) => p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(0) },
					{ n: 2, label: 'Traces', target: (p) => p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(1) },
					{ n: 3, label: 'Service map', target: (p) => p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(2) },
					{ n: 4, label: 'Logs', target: (p) => p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(3) },
					{ n: 5, label: 'Pipeline', target: (p) => p.getByRole('navigation', { name: 'Main' }).getByRole('link').nth(4) },
				],
			}, 36);
			// Theme switch: the button in its current mode, then the custom range popover.
			await frag(page, page.getByRole('button', { name: /^Theme:/ }), 'shell-theme-switch', theme, {
				page: 'theme',
				route: '/',
				state: `theme switch showing ${theme}`,
				description: `The theme switch in ${theme} mode. Each click cycles light, dark, system.`,
			}, 10);
			if (wanted('shell-custom-range')) {
				await page.getByRole('button', { name: /Custom time range/ }).click();
				await page.waitForTimeout(500);
				await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
				await frag(page, page.locator('[data-radix-popper-content-wrapper]').first(), 'shell-custom-range', theme, {
					page: 'shell',
					route: '/',
					state: 'custom time range popover open',
					description: 'The custom time range popover: a length and an end time, for looking at a past window.',
				}, 0);
				await page.keyboard.press('Escape');
			}
			if (wanted('shell-shortcuts')) {
				await page.locator('body').click({ position: { x: 700, y: 870 } });
				await page.keyboard.press('?');
				await page.getByRole('dialog').waitFor();
				await page.waitForTimeout(400);
				await frag(page, page.getByRole('dialog'), 'shell-shortcuts', theme, {
					page: 'shell',
					route: '/',
					state: 'keyboard shortcuts dialog (press ?)',
					description: 'The keyboard shortcuts dialog: ⌘K for the palette, g then a letter to go to a page, ? for this list.',
				}, 3);
				await page.keyboard.press('Escape');
			}
		},
	},
	{
		name: 'stories',
		async run(page, theme, d) {
			await open(page, '/');
			await pageAndFull(page, 'stories-home', theme, {
				page: 'stories',
				route: '/',
				state: 'last hour, top group selected',
				description: 'Stories home: KPI tiles, the story groups table, the inspector for the selected group, the mini service map and recent log alerts.',
			});
			await frag(page, page.getByRole('list', { name: 'Summary' }), 'stories-kpis', theme, {
				page: 'stories',
				route: '/',
				state: 'last hour',
				description: 'KPI tiles: error stories, slow stories, active log alerts and spans per second, with the change against the previous window.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Error stories in the range, with a sparkline', target: (p) => p.getByRole('listitem').filter({ hasText: /Error stories/i }) },
					{ n: 2, label: 'Slow stories in the range', target: (p) => p.getByRole('listitem').filter({ hasText: /Slow stories/i }) },
					{ n: 3, label: 'Log alerts still firing', target: (p) => p.getByRole('listitem').filter({ hasText: /log alerts/i }) },
					{ n: 4, label: 'Ingest rate and pipeline lag', target: (p) => p.getByRole('listitem').filter({ hasText: /Spans/i }) },
				],
			});
			await frag(page, page.getByLabel('Search story groups', { exact: true }).locator('xpath=../..'), 'stories-filters', theme, {
				page: 'stories',
				route: '/',
				state: 'no filters',
				description: 'The filter bar above the story groups: kind, root-cause service, endpoint and a text search.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Kind: all, error or slow', target: (p) => p.getByRole('button', { name: /Kind filter/ }) },
					{ n: 2, label: 'Filter by root-cause service', target: (p) => p.getByRole('button', { name: /service/ }).filter({ hasText: '+' }).first() },
					{ n: 3, label: 'Filter by endpoint', target: (p) => p.getByRole('button', { name: /endpoint/ }).filter({ hasText: '+' }).first() },
					{ n: 4, label: 'Search the group summaries', target: (p) => p.getByLabel('Search story groups', { exact: true }) },
				],
			}, 10);
			const row = page.locator(`[role="row"][data-fingerprint="${d.homeGroup}"]`);
			await frag(page, row, 'stories-group-row', theme, {
				page: 'stories',
				route: '/',
				state: 'one story group row',
				description: 'A story group row: kind, summary with the root-cause detail and endpoint, trend over the range, story count and last seen.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Kind badge (error or slow)', target: (p) => p.locator(`[data-fingerprint="${d.homeGroup}"] [role="gridcell"]`).nth(1).locator('span').filter({ hasText: /^(error|slow)$/ }).first() },
					{ n: 2, label: 'Summary, root cause and endpoint', target: (p) => p.locator(`[data-fingerprint="${d.homeGroup}"] [role="gridcell"]`).nth(1)  },
					{ n: 3, label: 'Stories over the range', target: (p) => p.locator(`[data-fingerprint="${d.homeGroup}"] [role="gridcell"]`).nth(2) },
					{ n: 4, label: 'Stories in the range', target: (p) => p.locator(`[data-fingerprint="${d.homeGroup}"] [role="gridcell"]`).nth(3) },
					{ n: 5, label: 'Last seen', target: (p) => p.locator(`[data-fingerprint="${d.homeGroup}"] [role="gridcell"]`).nth(4) },
				],
			}, 16);
			await frag(page, panel(page, 'Service map'), 'stories-minimap', theme, {
				page: 'stories',
				route: '/',
				state: 'last hour',
				description: 'The mini service map on the Stories page; services with errors are marked. Click it to open the full map.',
			});
			await frag(page, panel(page, 'Log alerts'), 'stories-alerts-panel', theme, {
				page: 'stories',
				route: '/',
				state: 'last hour',
				description: 'Recent log alerts on the Stories page, with a link to the Alerts page.',
			});
			// A selected group: a second group (not the top one) in the inspector.
			await open(page, `/?group=${d.altGroup}`);
			await page.locator(`[role="row"][data-fingerprint="${d.altGroup}"]`).click();
			await page.mouse.move(1, 450);
			await settle(page, 800);
			await pageAndFull(page, 'stories-group-selected', theme, {
				page: 'stories',
				route: `/?group=${d.altGroup}`,
				state: 'a group selected; inspector shows its sample story',
				description: 'Stories home with a group selected: the inspector shows the sample story, its request path, a waterfall summary and the comparison with normal.',
			});
			await frag(page, page.getByRole('complementary', { name: 'Selected story' }).or(page.locator('[aria-label="Selected story"]')), 'stories-inspector', theme, {
				page: 'stories',
				route: `/?group=${d.altGroup}`,
				state: 'a group selected',
				description: 'The inspector (group detail): sample story summary, request path, a waterfall summary, comparison with normal and the button to open the story.',
				callouts: [
					{ n: 1, label: 'Story kind, trace and duration', target: (p) => p.locator('[aria-label="Selected story"] h2, [aria-label="Selected story"] h3').first().locator('xpath=preceding-sibling::*[1]') },
					{ n: 2, label: 'Group summary', target: (p) => p.locator('[aria-label="Selected story"] h2, [aria-label="Selected story"] h3').first() },
					{ n: 3, label: 'Request path; the failing service is red', target: (p) => p.locator('[aria-label="Selected story"] [aria-label="Request path"]') },
					{ n: 4, label: 'Waterfall summary', target: (p) => p.locator('[aria-label="Selected story"] [role="group"]').first() },
					{ n: 5, label: 'Compared with normal', target: (p) => p.locator('[aria-label="Selected story"] section[aria-label="Compared with normal"]') },
					{ n: 6, label: 'Open the story', target: (p) => p.locator('[aria-label="Selected story"]').getByRole('link', { name: /Open story/ }) },
				],
			}, 30);
			// Filtered: error stories only.
			await open(page, '/?kind=error');
			await pageAndFull(page, 'stories-filtered-errors', theme, {
				page: 'stories',
				route: '/?kind=error',
				state: 'kind filter: error',
				description: 'Stories home filtered to error stories.',
			});
		},
	},
	{
		name: 'stories-states',
		async run(page, theme) {
			// Empty and error states: mocked API responses.
			if (wanted('stories-empty')) {
				await page.route('**/api/v1/story-groups?*', (r) => r.fulfill({ json: [] }));
				await open(page, '/');
				await frag(page, page.locator('section[aria-label="Story groups"] [data-variant="panel"]').first(), 'stories-empty', theme, {
					page: 'stories',
					route: '/',
					state: 'empty: no stories in the window (mocked empty response)',
					description: 'The empty state when nothing failed or ran slow in the range, with a hint to widen it.',
					data: 'mocked',
				});
				await page.unroute('**/api/v1/story-groups?*');
			}
			if (wanted('stories-error')) {
				await page.route('**/api/v1/story-groups?*', (r) => r.fulfill({ status: 503, json: { error: 'clickhouse unavailable' } }));
				await open(page, '/', 2500);
				await frag(page, page.locator('section[aria-label="Story groups"] [data-variant="panel"]').first(), 'stories-error', theme, {
					page: 'stories',
					route: '/',
					state: 'error: story groups failed to load (mocked 503)',
					description: 'The error banner when a panel cannot load, with a Try again button.',
					data: 'mocked',
				});
				await page.unroute('**/api/v1/story-groups?*');
			}
			if (wanted('stories-filter-empty')) {
				await open(page, '/?q=no-such-group-zzz');
				await frag(page, page.locator('section[aria-label="Story groups"] [data-variant="panel"]').first(), 'stories-filter-empty', theme, {
					page: 'stories',
					route: '/?q=no-such-group-zzz',
					state: 'empty: the filters match no group',
					description: 'The empty state when the filters match no group, with a way to clear them.',
				});
			}
			if (wanted('story-not-found')) {
				await open(page, '/stories/00000000000000000000000000000000');
				await pageAndFull(page, 'story-not-found', theme, {
					page: 'story',
					route: '/stories/00000000000000000000000000000000',
					state: 'story not found (404)',
					description: 'The page for a story that has expired or never existed.',
				});
			}
		},
	},
	{
		name: 'story',
		async run(page, theme, d) {
			const route = `/stories/${d.hero.storyId}`;
			await open(page, route, 1500);
			await pageAndFull(page, 'story-detail', theme, {
				page: 'story',
				route,
				state: 'error story; waterfall zoomed to the critical path',
				description: `Story detail for "${d.hero.summary}": root cause, request path, group trend, comparison with normal, the waterfall, logs and related alerts.`,
			});
			if (wanted('hero-story')) {
				// The hero: header card and the waterfall filtered to the critical path in one frame.
				await open(page, `${route}?only=errors`, 1500);
				await page.evaluate(() => {
					const wf = [...document.querySelectorAll('[data-variant="panel"]')].find((e) => e.querySelector('h2')?.textContent === 'Waterfall');
					const row = wf?.previousElementSibling as HTMLElement | null;
					if (row) row.style.display = 'none';
				});
				await page.waitForTimeout(500);
				await shotPage(page, 'hero-story', theme, {
					page: 'story',
					kind: 'page',
					route: `${route}?only=errors`,
					state: 'error story; waterfall filtered to error spans (the failing chain with its critical-path timing); the trend and comparison row hidden so the waterfall is in view',
					description: 'Hero: an error story with its root cause, request path and the waterfall filtered to the failing chain, ending in the marked root-cause span.',
				});
				await open(page, route, 1500);
			}
			const header = page.locator('[data-variant="panel"]').filter({ has: page.locator('[aria-label="Request path"]') }).first();
			await frag(page, header, 'story-root-cause', theme, {
				page: 'story',
				route,
				state: 'error story header',
				description: 'The root-cause card: kind, endpoint, the story summary, the root-cause service and span, the request path and key facts.',
				callouts: [
					{ n: 1, label: 'Kind and endpoint', target: () => header.locator('span').filter({ hasText: /story$/ }).first() },
					{ n: 2, label: 'Open in Jaeger / open the trace in Tayga', target: () => header.getByRole('link', { name: /Open trace/ }), right: true },
					{ n: 3, label: 'Summary', target: () => header.locator('h2').first() },
					{ n: 4, label: 'Root cause: service, span and exception', target: () => header.locator('p').first() },
					{ n: 5, label: 'Request path, root-cause service highlighted', target: () => header.locator('[aria-label="Request path"]') },
					{ n: 6, label: 'Time, root span duration, spans, services, trace id', target: () => header.locator('dl') },
				],
			}, 30);
			await frag(page, panel(page, 'Group trend'), 'story-group-trend', theme, {
				page: 'story',
				route,
				state: 'error story',
				description: 'Group trend: how many stories of this group occurred over the range.',
			});
			await frag(page, panel(page, 'Compared with normal'), 'story-baseline', theme, {
				page: 'story',
				route,
				state: 'error story',
				description: 'Compared with normal: operations slower than the baseline, new or missing operations, other failed spans and where the time went on the critical path.',
			});
			const wf = panel(page, 'Waterfall');
			await frag(page, wf, 'story-waterfall', theme, {
				page: 'story',
				route,
				state: 'zoomed to the critical path',
				description: 'The waterfall: span search, error and critical-path filters, the trace overview (drag to zoom) and the span rows; the root-cause row is marked.',
				callouts: [
					{ n: 1, label: 'Search spans', target: (p) => p.getByLabel('Search spans', { exact: true }) },
					{ n: 2, label: 'Show errors / critical path only', target: () => wf.getByText('All spans', { exact: true }).locator('..') },
					{ n: 3, label: 'Trace overview: drag to zoom', target: (p) => p.getByRole('group', { name: /Trace overview/ }) },
					{ n: 4, label: 'Root-cause span', target: (p) => p.locator('[data-root-cause="true"]').first() },
				],
			}, 30);
			await frag(page, page.locator('[data-root-cause="true"]').first(), 'story-waterfall-row', theme, {
				page: 'story',
				route,
				state: 'root-cause row',
				description: 'A waterfall row: indentation by depth, service and span name, the error mark, and the bar on the time axis. The root-cause row is highlighted.',
			}, 8);
			await frag(page, panel(page, 'Logs'), 'story-logs', theme, {
				page: 'story',
				route,
				state: 'logs of the trace',
				description: 'Logs of the story trace: filter by service and severity; each line links to its span and its log template.',
				
				callouts: [
					{ n: 1, label: 'Filter by service', target: (p) => p.getByRole('group', { name: 'Filter logs by service' }) },
					{ n: 2, label: 'Log table: time, service, span, severity, body, template', target: (p) => p.getByRole('table', { name: 'Logs' }) },
				],
			}, 30);
			await frag(page, panel(page, 'Related log alerts'), 'story-related-alerts', theme, {
				page: 'story',
				route,
				state: 'alerts on templates seen in this trace',
				description: 'Related log alerts: alerts on the log templates this trace emitted.',
			});
			// Span drawer: open the root-cause span.
			if (wanted('story-span-drawer')) {
				await page.locator('[data-root-cause="true"]').first().click();
				await settle(page, 700);
				await shotPage(page, 'story-span-drawer', theme, {
					page: 'story',
					kind: 'page',
					route: `${route}?span=…`,
					state: 'root-cause span drawer open',
					description: 'The span drawer for the root-cause span: attributes, events, the exception and its logs.',
				});
				await frag(page, page.getByRole('dialog').first(), 'story-span-drawer-panel', theme, {
					page: 'story',
					route: `${route}?span=…`,
					state: 'root-cause span drawer open',
					description: 'The span drawer: span name, timing, tabs for attributes, events and logs, and copy buttons.',
				}, 0);
				await page.keyboard.press('Escape');
			}
			if (d.slow && wanted('story-slow')) {
				const r2 = `/stories/${d.slow.storyId}`;
				await open(page, r2, 1500);
				await pageAndFull(page, 'story-slow', theme, {
					page: 'story',
					route: r2,
					state: 'slow story',
					description: `A slow story ("${d.slow.summary}"): the critical path shows where the time went and the comparison with the endpoint's baseline.`,
				});
				await frag(page, panel(page, 'Compared with normal'), 'story-slow-baseline', theme, {
					page: 'story',
					route: r2,
					state: 'slow story',
					description: 'Compared with normal on a slow story: slower operations against their p95 and the critical path breakdown.',
				});
			}
		},
	},
	{
		name: 'traces',
		async run(page, theme, d) {
			await open(page, '/traces', 1500);
			await pageAndFull(page, 'traces-search', theme, {
				page: 'traces',
				route: '/traces',
				state: 'last hour, no filters (newest 500)',
				description: 'Traces search: filters, the duration-over-time scatter and the trace list.',
			});
			const filters = page.locator('[data-variant="panel"]').first();
			await frag(page, filters, 'traces-filters', theme, {
				page: 'traces',
				route: '/traces',
				state: 'no filters',
				description: 'Trace filters: service, endpoint, duration bounds, errors only, and opening a trace by id.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Service', target: (p) => p.getByRole('button', { name: /^Service/ }).first() },
					{ n: 2, label: 'Endpoint', target: (p) => p.getByRole('button', { name: /^Endpoint/ }).first() },
					{ n: 3, label: 'Duration bounds (ms)', target: (p) => p.getByRole('group', { name: 'Duration (ms)' }) },
					{ n: 4, label: 'Errors only', target: (p) => p.getByRole('button', { name: /Errors only/ }) },
					{ n: 5, label: 'Open a trace by id', target: (p) => p.getByRole('search', { name: 'Open a trace by id' }) },
				],
			}, 10);
			const svcRoute = `/traces?service=${encodeURIComponent(d.mapService)}&touched=1`;
			await open(page, svcRoute, 1500);
			await pageAndFull(page, 'traces-service', theme, {
				page: 'traces',
				route: svcRoute,
				state: `service filter: ${d.mapService}, anywhere in the trace`,
				description: `Traces search filtered to ${d.mapService}: error and slow stories stand out on the scatter, and the list links each to its story.`,
			});
			await frag(page, panel(page, 'Duration over time'), 'traces-scatter', theme, {
				page: 'traces',
				route: svcRoute,
				state: `last hour, service ${d.mapService}`,
				description: 'Duration over time: each dot is a trace; stories are coloured. Drag to select, click a dot to open it. Linear or log scale.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Legend: other traces, slow stories, errors', target: (p) => panel(p, 'Duration over time').getByRole('list', { name: 'Legend' }) },
					{ n: 2, label: 'Linear or log scale', target: (p) => p.locator('[aria-label="Duration scale"]') },
				],
			}, 16);
			await frag(page, panel(page, 'Traces'), 'traces-table', theme, {
				page: 'traces',
				route: svcRoute,
				state: `last hour, service ${d.mapService}`,
				description: 'The trace list: start, endpoint, duration, spans, errors and a link to the story when the trace is one.',
			});
			await open(page, '/traces?errors=1', 1500);
			await pageAndFull(page, 'traces-errors', theme, {
				page: 'traces',
				route: '/traces?errors=1',
				state: 'errors only',
				description: 'Traces search with "Errors only": failing traces and their stories.',
			});
			const route = `/traces/${d.hero.traceId}`;
			await open(page, route, 1500);
			await pageAndFull(page, 'trace-view', theme, {
				page: 'traces',
				route,
				state: 'the hero story trace',
				description: 'Trace view: summary stats, the link to its story and the full waterfall; a span’s logs open in the span drawer.',
			});
			await frag(page, page.locator('[data-variant="panel"]').first(), 'trace-header', theme, {
				page: 'traces',
				route,
				state: 'the hero story trace',
				description: 'Trace header: endpoint, root span duration, trace window, spans, services, errors, logs and start time, with the story link.',
			}, 10);
			await frag(page, page.getByRole('link', { name: 'Story for this trace' }).or(page.locator('[aria-label="Story for this trace"]')), 'trace-story-link', theme, {
				page: 'traces',
				route,
				state: 'the hero story trace',
				description: 'The banner linking a trace to its story.',
			}, 10);
			await open(page, '/traces/00000000000000000000000000000001');
			await frag(page, page.locator('main [data-variant="panel"]').first().or(page.locator('[data-variant="panel"]').first()), 'trace-not-found', theme, {
				page: 'traces',
				route: '/traces/00000000000000000000000000000001',
				state: 'trace not found',
				description: 'The state for a trace id that is not stored.',
			});
		},
	},
	{
		name: 'map',
		async run(page, theme, d) {
			await open(page, '/map', 1500);
			await pageAndFull(page, 'map-overview', theme, {
				page: 'map',
				route: '/map',
				state: 'last hour, infrastructure hidden',
				description: 'Service map: services as cards coloured by health, calls as edges sized by rate and coloured by error rate.',
			});
			const toolbar = page.locator('[data-variant="panel"]').filter({ has: page.getByRole('heading', { name: 'Service map' }) }).first();
			await frag(page, toolbar, 'map-toolbar', theme, {
				page: 'map',
				route: '/map',
				state: 'infrastructure hidden',
				description: 'Map toolbar: summary, service search and the infrastructure toggle.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Services, health and hidden infrastructure', target: () => toolbar.locator('h2').locator('xpath=following-sibling::*[1]') },
					{ n: 2, label: 'Find a service', target: () => toolbar.locator('input').first() },
					{ n: 3, label: 'Show infrastructure services', target: () => toolbar.getByRole('switch').first() },
				],
			}, 10);
			await frag(page, page.getByRole('list', { name: 'Legend' }).or(page.locator('[aria-label="Legend"]')).first(), 'map-legend', theme, {
				page: 'map',
				route: '/map',
				state: 'legend',
				description: 'Map legend: node health, failing and erroring edges, line width by calls per minute.',
			}, 8);
			// Hover an edge to show its label.
			if (wanted('map-edge')) {
				const edge = page.locator('.react-flow__edge').filter({ has: page.locator('path') }).nth(2);
				const path = edge.locator('path').last();
				const box = await path.boundingBox();
				if (box) {
					await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
					await page.waitForTimeout(250);
					const pts = await path.evaluate((el) => {
						const p = el as SVGPathElement;
						const l = p.getTotalLength();
						const pt = p.getPointAtLength(l / 2);
						const m = p.getScreenCTM()!;
						return { x: pt.x * m.a + m.e, y: pt.y * m.d + m.f };
					});
					await page.mouse.move(pts.x, pts.y);
					await page.waitForTimeout(600);
					const label = page.locator('.tg-map-label').first();
					if (await label.isVisible()) {
						const lb = (await label.boundingBox())!;
						await page.evaluate(
							(r) => {
								const n = document.createElement('div');
								n.dataset.capture = 'clipbox';
								n.style.cssText = `position:fixed;left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;pointer-events:none;`;
								document.body.appendChild(n);
							},
							{ x: Math.min(lb.x, pts.x) - 140, y: Math.min(lb.y, pts.y) - 90, w: lb.width + 280, h: lb.height + 180 },
						);
						await frag(page, page.locator('[data-capture="clipbox"]'), 'map-edge-hover', theme, {
							page: 'map',
							route: '/map',
							state: 'hovering an edge',
							description: 'Hovering an edge shows its calls per minute, error rate and average latency; clicking pins the label.',
						}, 0);
						await page.evaluate(() => document.querySelector('[data-capture="clipbox"]')?.remove());
					} else console.warn('  ! map-edge-hover: no label on hover');
				}
				await page.mouse.move(5, 5);
			}
			const node = page.locator('.react-flow__node').filter({ hasText: d.mapService }).first();
			await frag(page, node, 'map-node', theme, {
				page: 'map',
				route: '/map',
				state: 'one service node',
				description: 'A service node: health ring, name, and badges for calls to infrastructure services.',
			}, 12);
			await open(page, '/map?infra=1', 1500);
			await pageAndFull(page, 'map-infra', theme, {
				page: 'map',
				route: '/map?infra=1',
				state: 'infrastructure shown',
				description: 'Service map with infrastructure services (such as flagd) drawn as nodes.',
			});
			const r3 = `/map?service=${encodeURIComponent(d.mapService)}`;
			await open(page, r3, 1800);
			await pageAndFull(page, 'map-service-selected', theme, {
				page: 'map',
				route: r3,
				state: `${d.mapService} selected; drawer open`,
				description: `Service map with ${d.mapService} selected: its neighbours are highlighted and the drawer shows rate, errors, p99, calls in and out, and its stories.`,
			});
			await frag(page, page.getByRole('dialog').first(), 'map-drawer', theme, {
				page: 'map',
				route: r3,
				state: `${d.mapService} selected`,
				description: 'The service drawer: RED metrics (rate, errors, p99), callers and callees, and recent stories for the service.',
			}, 0);
			await open(page, '/map?q=no-such-service', 1500);
			await frag(page, page.locator('[data-variant="panel"]').first(), 'map-search-empty', theme, {
				page: 'map',
				route: '/map?q=no-such-service',
				state: 'search with no match',
				description: 'The map toolbar when the search matches no service.',
			}, 10);
		},
	},
	{
		name: 'logs',
		async run(page, theme, d) {
			await open(page, '/logs/templates', 1200);
			await pageAndFull(page, 'logs-templates', theme, {
				page: 'logs',
				route: '/logs/templates',
				state: 'last hour, all services',
				description: 'Log templates: every template mined from the logs, with service, hits, trend, first seen and alert status.',
			});
			const card = page.locator('[data-variant="panel"]').first();
			await frag(page, card.locator('> div').first(), 'logs-templates-filters', theme, {
				page: 'logs',
				route: '/logs/templates',
				state: 'no filters',
				description: 'Template filters: service and a text search.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Filter by service', target: (p) => p.getByRole('button', { name: /^Service/ }).first() },
					{ n: 2, label: 'Search template text', target: (p) => p.getByLabel('Search templates', { exact: true }) },
				],
			}, 10);
			await frag(page, page.locator('[role="row"][data-template-id]').first(), 'logs-template-row', theme, {
				page: 'logs',
				route: '/logs/templates',
				state: 'top template',
				description: 'A template row: service, the template with masked variables (<*>), hits, trend, first seen and status.',
			}, 8);
			const tr = `/logs/templates?q=${encodeURIComponent(d.template.text.slice(0, 24))}`;
			await open(page, tr, 1200);
			await frag(page, page.locator('[data-variant="panel"]').first(), 'logs-templates-search', theme, {
				page: 'logs',
				route: tr,
				state: 'text search',
				description: 'Template search narrowed to one template, with its alert status.',
			});
			const route = `/logs/templates/${d.template.id}`;
			await open(page, route, 1500);
			await pageAndFull(page, 'logs-template-detail', theme, {
				page: 'logs',
				route,
				state: `template "${d.template.text.slice(0, 60)}"`,
				description: 'Template detail: the template text and stats, hits over time, a sample, the latest hits with links to traces and stories, the alerts on it and the silence setting.',
			});
			const head = page.locator('[data-variant="panel"]').first();
			await frag(page, head, 'logs-template-header', theme, {
				page: 'logs',
				route,
				state: 'template header',
				description: 'Template header: service, template text, hits in the range, first and last seen, number of alerts.',
			});
			await frag(page, panel(page, /^Hits per/), 'logs-template-hits', theme, {
				page: 'logs',
				route,
				state: 'hits chart',
				description: 'Hits per minute for the template over the range.',
			});
			await frag(page, panel(page, 'Sample'), 'logs-template-sample', theme, {
				page: 'logs',
				route,
				state: 'sample and latest hits',
				description: 'A sample log line for the template and its latest hits, each linked to its trace and story.',
			});
			await frag(page, panel(page, /^Latest \d+ hits/), 'logs-template-recent', theme, {
				page: 'logs',
				route,
				state: 'latest hits',
				description: 'The latest hits of the template: time, body, and links to the trace and the story.',
			});
			await frag(page, panel(page, 'Alerts'), 'logs-template-alerts', theme, {
				page: 'logs',
				route,
				state: 'alerts on this template',
				description: 'Alerts raised on this template.',
			});
			const sil = page.locator('[data-variant="panel"]').filter({ has: page.getByRole('switch') }).last();
			await frag(page, sil, 'logs-silence-off', theme, {
				page: 'logs',
				route,
				state: 'alert when silent: off',
				description: 'The silence control: alert when the template has not been seen for a number of minutes.',
				gutter: 'top',
					callouts: [
					{ n: 1, label: 'Alert when silent (switch)', target: () => sil.getByRole('switch') },
					{ n: 2, label: 'Silent minutes (1–1440)', target: (p) => p.getByLabel('Silent minutes') },
				],
			}, 20);
			if (wanted('logs-silence-on')) {
				const sw = sil.getByRole('switch');
				const was = await sw.getAttribute('aria-checked');
				if (was !== 'true') {
					await sw.click(); // a draft only: the shot is taken before Save and the page is reloaded after
					await page.waitForTimeout(300);
				}
				await frag(page, sil, 'logs-silence-on', theme, {
					page: 'logs',
					route,
					state: 'alert when silent: switched on, not yet saved',
					description: 'The silence control switched on: set the minutes and press Save to apply.',
					gutter: 'top',
					callouts: [
						{ n: 1, label: 'Alert when silent (switch)', target: () => sil.getByRole('switch') },
						{ n: 2, label: 'Silent minutes', target: (p) => p.getByLabel('Silent minutes') },
						{ n: 3, label: 'Save', target: () => sil.getByRole('button', { name: /Save/ }) },
					],
				}, 20);
				await page.reload();
			}
			await open(page, '/logs/templates/1', 1000);
			await frag(page, page.locator('[data-variant="panel"]').first(), 'logs-template-not-found', theme, {
				page: 'logs',
				route: '/logs/templates/1',
				state: 'template not found',
				description: 'The state for a template id that does not exist.',
			});
		},
	},
	{
		name: 'alerts',
		async run(page, theme, d) {
			const since = d.alertsSince === '1h' ? '' : `since=${d.alertsSince}`;
			const route = `/logs/alerts${since ? `?${since}` : ''}`;
			await open(page, route, 1500);
			await pageAndFull(page, 'alerts', theme, {
				page: 'alerts',
				route,
				state: `last ${d.alertsSince}, all kinds`,
				description: 'Log alerts: filters, alerts over time by kind, and the list of new, spike and silence alerts with example traces.',
			});
			await frag(page, page.locator('[data-variant="panel"]').first(), 'alerts-filters', theme, {
				page: 'alerts',
				route,
				state: 'no filters',
				description: 'Alert filters: kind, service, active only, and a link to the templates.',
				gutter: 'top',
				callouts: [
					{ n: 1, label: 'Kind: all, new, spike, silence', target: (p) => p.locator('[aria-label="Alert kind"]') },
					{ n: 2, label: 'Service', target: (p) => p.getByRole('button', { name: /^Service/ }).first() },
					{ n: 3, label: 'Active only', target: (p) => p.getByRole('button', { name: /Active only/ }) },
					{ n: 4, label: 'Browse templates', target: (p) => p.getByRole('link', { name: 'Browse templates' }) },
				],
			}, 10);
			await frag(page, panel(page, /^Alerts per/), 'alerts-timeline', theme, {
				page: 'alerts',
				route,
				state: `last ${d.alertsSince}`,
				description: 'Alerts over time, stacked by kind.',
			});
			for (const kind of ['new', 'spike', 'silence'] as const) {
				const r = `/logs/alerts?${since ? `${since}&` : ''}kind=${kind}`;
				await open(page, r, 1200);
				if (wanted(`alerts-${kind}`)) await shotPage(page, `alerts-${kind}`, theme, {
					page: 'alerts',
					kind: 'page',
					route: r,
					state: `kind: ${kind}`,
					description: `Log alerts filtered to ${kind} alerts.`,
				});
				const row = page.locator('table tbody tr').first();
				if ((await row.count()) === 0) {
					console.warn(`  ! no ${kind} alerts in ${d.alertsSince}`);
					continue;
				}
				await frag(page, row, `alerts-card-${kind}`, theme, {
					page: 'alerts',
					route: r,
					state: `one ${kind} alert`,
					description: {
						new: 'A new-template alert: a template that first appeared after its service was established.',
						spike: 'A spike alert: the window count, the peak and the baseline, with example traces linked to stories.',
						silence: 'A silence alert: a watched template that has not been seen for its set minutes.',
					}[kind],
				}, 10);
			}
		},
	},
	{
		name: 'pipeline',
		async run(page, theme) {
			await open(page, '/pipeline', 2000);
			await pageAndFull(page, 'pipeline', theme, {
				page: 'pipeline',
				route: '/pipeline',
				state: 'last hour',
				description: 'Pipeline: job status, throughput charts per stage and consumer lag.',
			});
			await frag(page, page.getByRole('list', { name: 'Job status' }), 'pipeline-status', theme, {
				page: 'pipeline',
				route: '/pipeline',
				state: 'all jobs up',
				description: 'The status strip: each job (ingest, writer, assembler, logminer, notifier, api) and when it was last scraped.',
			}, 10);
			await frag(page, panel(page, /^Ingest records/i), 'pipeline-chart-ingest', theme, {
				page: 'pipeline',
				route: '/pipeline',
				state: 'last hour',
				description: 'Ingest records per second by kind (traces, logs).',
			});
			await frag(page, panel(page, /^Assembler output/i), 'pipeline-chart-assembler', theme, {
				page: 'pipeline',
				route: '/pipeline',
				state: 'last hour',
				description: 'Assembler output: closed traces, error stories and slow stories per second.',
			});
			await frag(page, panel(page, 'Consumer lag'), 'pipeline-lag', theme, {
				page: 'pipeline',
				route: '/pipeline',
				state: 'current lag',
				description: 'Consumer lag per consumer group and topic.',
			});
		},
	},
	{
		name: 'palette',
		async run(page, theme, d) {
			await open(page, '/');
			const isMac = process.platform === 'darwin';
			const openPalette = async () => {
				await page.locator('header button').filter({ hasText: 'Jump to' }).click();
				await page.getByRole('dialog').waitFor();
				await page.waitForTimeout(500);
			};
			void isMac;
			await openPalette();
			if (wanted('palette-open')) await shotPage(page, 'palette-open', theme, {
				page: 'palette',
				kind: 'page',
				route: '/',
				state: 'palette open, empty query',
				description: 'The command palette (⌘K / Ctrl+K): recent items, pages and actions.',
			});
			await frag(page, page.getByRole('dialog').first(), 'palette', theme, {
				page: 'palette',
				route: '/',
				state: 'empty query',
				description: 'The command palette: type to search services, story groups, templates or a trace id.',
				callouts: [
					{ n: 1, label: 'Search input', target: (p) => p.getByRole('dialog').getByRole('combobox').first() },
					{ n: 2, label: 'Pages', target: (p) => p.locator('[cmdk-group-heading]').filter({ hasText: /^Pages$/ }) },
					{ n: 3, label: 'Actions', target: (p) => p.locator('[cmdk-group-heading]').filter({ hasText: /^Actions$/ }) },
				],
			}, 0);
			const term = d.mapService;
			await page.keyboard.type(term, { delay: 30 });
			await settle(page, 900);
			await frag(page, page.getByRole('dialog').first(), 'palette-results', theme, {
				page: 'palette',
				route: '/',
				state: `query "${term}"`,
				description: `Palette results for "${term}": the service, story groups and log templates that match.`,
			}, 0);
			await page.keyboard.press('Escape');
		},
	},
	{
		name: 'login',
		async run(page, theme) {
			// Auth is off on the shared stack: mock the config and session instead of changing it.
			await page.route('**/api/v1/config', (r) =>
				r.fulfill({ json: { jaeger_url: null, grafana_url: null, auth_enabled: true, infra_services: ['flagd'] } }),
			);
			await page.route('**/api/v1/auth/me', (r) => r.fulfill({ status: 401, json: { error: 'unauthorized' } }));
			await page.route('**/api/v1/auth/login', (r) => r.fulfill({ status: 401, json: { error: 'unauthorized' } }));
			await open(page, '/login', 800);
			if (wanted('login')) await shotPage(page, 'login', theme, {
				page: 'login',
				kind: 'page',
				route: '/login',
				state: 'auth on (mocked config), signed out',
				description: 'The sign-in page shown when authentication is enabled.',
				data: 'mocked',
			});
			if (wanted('login-error')) {
				await page.getByLabel('Username').fill('admin');
				await page.getByLabel('Password').fill('wrong-password');
				await page.getByRole('button', { name: 'Sign in' }).click();
				await page.waitForTimeout(800);
				await frag(page, page.locator('form').first().locator('xpath=ancestor::*[@data-variant="panel"][1]').or(page.locator('form').first()), 'login-error', theme, {
					page: 'login',
					route: '/login',
					state: 'wrong credentials (mocked 401)',
					description: 'The sign-in form after wrong credentials: the username stays, the password is cleared.',
					data: 'mocked',
				}, 16);
			}
			await page.unrouteAll();
		},
	},
	// Mobile: 390 px wide.
	{
		name: 'mobile',
		mobile: true,
		async run(page, theme, d) {
			const shots: [string, string, string][] = [
				['mobile-stories', '/', 'Stories home on a phone: KPI tiles, the group list and the inspector stack vertically.'],
				['mobile-story', `/stories/${d.hero.storyId}`, 'Story detail on a phone.'],
				['mobile-map', '/map', 'Service map on a phone.'],
				['mobile-alerts', `/logs/alerts?since=${d.alertsSince}`, 'Log alerts on a phone.'],
			];
			for (const [name, route, description] of shots) {
				if (!wanted(name)) continue;
				await open(page, route, 1500);
				await shotPage(page, name, theme, { page: name.replace('mobile-', ''), kind: 'mobile', route, state: '390 × 844', description });
			}
		},
	},
];

// ---------------------------------------------------------------------------------------------

async function main(): Promise<void> {
	await mkdir(OUT, { recursive: true });
	const d = await discover();
	console.log('data:', JSON.stringify(d, null, 1));
	const browser = await chromium.launch();
	try {
		// Both themes of a scene back to back, so the data in a light/dark pair matches.
		for (const scene of scenes) {
			for (const theme of THEMES) {
				const vp = scene.mobile ? MOBILE : DESKTOP;
				const ctx = await newContext(browser, theme, vp);
				const page = await ctx.newPage();
				page.setDefaultTimeout(8000);
				console.log(`[${theme}] ${scene.name}`);
				try {
					await scene.run(page, theme, d, ctx);
				} catch (e) {
					console.error(`  ! scene ${scene.name} (${theme}) failed: ${(e as Error).message.split('\n')[0]}`);
				}
				await ctx.close();
			}
		}
	} finally {
		await browser.close();
	}
	let entries = manifest;
	if (process.env.KEEP_MANIFEST === '1') {
		const prev = JSON.parse(await readFile(`${OUT}manifest.json`, 'utf8').catch(() => '{"images":[]}')) as { images: Entry[] };
		const fresh = new Set(manifest.map((e) => e.file));
		entries = [...prev.images.filter((e) => !fresh.has(e.file)), ...manifest];
	}
	entries.sort((a, b) => a.file.localeCompare(b.file));
	await writeFile(
		`${OUT}manifest.json`,
		JSON.stringify(
			{
				generatedBy: 'site/scripts/capture.ts',
				app: BASE,
				viewport: { desktop: DESKTOP, mobile: MOBILE, deviceScaleFactor: DPR },
				note: 'width/height are the image pixels; cssWidth/cssHeight the CSS pixels captured (images are at up to 2x). data "mocked" marks shots taken with mocked API responses.',
				aliases: PNG_ALIASES,
				images: entries,
			},
			null,
			'\t',
		) + '\n',
	);
	console.log(`${manifest.length} images written to ${OUT}`);
}

await main();
