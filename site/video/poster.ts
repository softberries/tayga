/**
 * Renders the tour's posters from cards/poster.html (the title card with room for a play button):
 *   site/public/media/tayga-tour-poster.jpg  1920x1080, no button; the landing page draws its own
 *   docs/assets/tour-poster.jpg              1280x720 with the button drawn in, for the README
 *
 *   node video/poster.ts    (from site/)
 */
import { chromium } from 'playwright';
import { execFileSync } from 'node:child_process';
import { readFileSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { VIDEO_DIR } from './segments.ts';

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const UI_PUBLIC = `${ROOT}ui/public/`;
const OUT = [
	{ query: '', file: `${ROOT}site/public/media/tayga-tour-poster.jpg`, width: 1920 },
	{ query: '?play', file: `${ROOT}docs/assets/tour-poster.jpg`, width: 1280 },
];

const browser = await chromium.launch();
try {
	const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
	await ctx.route('http://cards.local/**', async (route) => {
		const path = new URL(route.request().url()).pathname;
		const file = path.startsWith('/fonts/') || path === '/logo.jpg' ? `${UI_PUBLIC}${path.slice(1)}` : `${VIDEO_DIR}cards${path}`;
		const type = path.endsWith('.css') ? 'text/css' : path.endsWith('.jpg') ? 'image/jpeg' : path.endsWith('.woff2') ? 'font/woff2' : 'text/html';
		await route.fulfill({ body: readFileSync(file), contentType: type });
	});
	const page = await ctx.newPage();
	for (const o of OUT) {
		await page.goto(`http://cards.local/poster.html${o.query}`);
		await page.evaluate(() => document.fonts.ready);
		await page.waitForTimeout(300);
		const png = `${o.file}.png`;
		await page.screenshot({ path: png });
		execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-i', png, '-vf', `scale=${o.width}:-2`, '-q:v', '3', o.file]);
		rmSync(png);
		console.log(`  ${o.file}`);
	}
	await ctx.close();
} finally {
	await browser.close();
}
