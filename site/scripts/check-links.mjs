// Checks every internal link in the built site (dist/): href, src, srcset and poster attributes in
// all HTML pages, including links in custom components that starlight-links-validator does not see.
// A link must resolve to a file under dist/ (with the /tayga/ base), and a #fragment to an id on
// the target page. Run after `npm run build`.
import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { join, relative, posix } from 'node:path';

const BASE = '/tayga/';
const DIST = new URL('../dist/', import.meta.url).pathname;

if (!existsSync(DIST)) {
	console.error('dist/ not found: run `npm run build` first.');
	process.exit(2);
}

const walk = (dir) =>
	readdirSync(dir).flatMap((n) => {
		const p = join(dir, n);
		return statSync(p).isDirectory() ? walk(p) : [p];
	});

const pages = walk(DIST).filter((f) => f.endsWith('.html'));
const idCache = new Map();
const idsOf = (file) => {
	if (!idCache.has(file)) {
		const html = readFileSync(file, 'utf8');
		idCache.set(file, new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
	}
	return idCache.get(file);
};

const attr = /\s(href|src|srcset|poster)="([^"]*)"/g;
const broken = [];
let checked = 0;

for (const page of pages) {
	const html = readFileSync(page, 'utf8');
	const pageUrl = '/' + posix.join(BASE.slice(1), relative(DIST, page).split('\\').join('/'));
	for (const [, name, value] of html.matchAll(attr)) {
		const urls = name === 'srcset' ? value.split(',').map((s) => s.trim().split(/\s+/)[0]) : [value];
		for (const raw of urls) {
			const url = raw.replaceAll('&amp;', '&');
			if (!url || /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(url)) continue; // external, mailto:, data:
			checked++;
			const [pathAndQuery, hash = ''] = url.split('#');
			const path = pathAndQuery.split('?')[0];
			const abs = path === '' ? pageUrl : path.startsWith('/') ? path : posix.join(posix.dirname(pageUrl), path);
			if (!abs.startsWith(BASE)) {
				broken.push({ page: pageUrl, url, why: `outside the ${BASE} base` });
				continue;
			}
			let file = join(DIST, decodeURIComponent(abs.slice(BASE.length)));
			if (abs.endsWith('/')) file = join(file, 'index.html');
			else if (existsSync(file) && statSync(file).isDirectory()) file = join(file, 'index.html');
			if (!existsSync(file)) {
				broken.push({ page: pageUrl, url, why: 'no such file' });
				continue;
			}
			if (hash && file.endsWith('.html') && !idsOf(file).has(decodeURIComponent(hash))) {
				broken.push({ page: pageUrl, url, why: `no element with id "${hash}"` });
			}
		}
	}
}

if (broken.length) {
	for (const b of broken) console.error(`${b.page}: ${b.url} (${b.why})`);
	console.error(`\n${broken.length} broken internal link(s) in ${pages.length} pages.`);
	process.exit(1);
}
console.log(`All ${checked} internal links in ${pages.length} pages resolve.`);
