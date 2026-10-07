// Renders the README hero banner (index.html) to docs/assets/hero-light.png and hero-dark.png:
// 1280x640 CSS px at device scale 2, then palette-quantised to keep the files small.
// Run from the repository root after `npm --prefix site ci` (it borrows the site's Playwright
// and sharp):  node docs/assets/hero/render.mjs
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.join(here, '../../../site/package.json'));
const { chromium } = require('playwright');
const sharp = require('sharp');

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 640 }, deviceScaleFactor: 2 });
for (const theme of ['light', 'dark']) {
	await page.goto(`${pathToFileURL(path.join(here, 'index.html')).href}?theme=${theme}`);
	await page.evaluate(() => document.fonts.ready);
	await page.waitForFunction(() => {
		const img = document.getElementById('shot');
		return img instanceof HTMLImageElement && img.complete && img.naturalWidth > 0;
	});
	const raw = await page.screenshot({ type: 'png' });
	const out = path.join(here, '..', `hero-${theme}.png`);
	const info = await sharp(raw).png({ palette: true, quality: 92, effort: 10, compressionLevel: 9 }).toFile(out);
	console.log(`${out}: ${info.width}x${info.height}, ${Math.round(info.size / 1024)} KiB`);
}
await browser.close();
