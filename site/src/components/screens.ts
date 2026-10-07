/**
 * Screenshot lookup for the user guide: the images in `src/assets/screens` and their entries in
 * `manifest.json` (written by `scripts/capture.ts`), by base name (`stories-kpis` for
 * `stories-kpis-dark.webp` and `stories-kpis-light.webp`).
 */
import type { ImageMetadata } from 'astro';
import manifest from '../assets/screens/manifest.json';

export interface Callout {
	n: number;
	label: string;
}

export interface ShotEntry {
	file: string;
	page: string;
	kind: string;
	route: string;
	state: string;
	theme: 'dark' | 'light';
	description: string;
	callouts: Callout[];
	width: number;
	height: number;
	cssWidth: number;
	cssHeight: number;
	data: 'live' | 'mocked';
}

const files = import.meta.glob<{ default: ImageMetadata }>('../assets/screens/*.webp', { eager: true });
const entries = new Map((manifest.images as ShotEntry[]).map((e) => [e.file, e]));

export interface Shot {
	entry: ShotEntry;
	image: ImageMetadata;
}

/** Both theme variants of a screenshot; fails the build when either is missing. */
export function shot(name: string): { dark: Shot; light: Shot } {
	const get = (theme: 'dark' | 'light'): Shot => {
		const file = `${name}-${theme}.webp`;
		const entry = entries.get(file);
		const image = files[`../assets/screens/${file}`]?.default;
		if (!entry || !image) throw new Error(`Screenshot "${file}" is not in src/assets/screens/manifest.json`);
		return { entry, image };
	};
	return { dark: get('dark'), light: get('light') };
}

/** Responsive widths for an image shown at most `cssWidth` CSS px wide, from an original `width` px wide. */
export function widthsFor(cssWidth: number, width: number): number[] {
	const candidates = [360, 720, 1080, 1440, 2160, 2880, cssWidth, Math.min(width, cssWidth * 2)];
	return [...new Set(candidates.filter((w) => w <= width && w <= cssWidth * 2))].sort((a, b) => a - b);
}
