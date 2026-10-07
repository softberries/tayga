/** Site-internal path with the configured base (`/tayga/`), e.g. `href('install/helm/')`. */
export function href(path: string): string {
	const base = import.meta.env.BASE_URL.endsWith('/') ? import.meta.env.BASE_URL : `${import.meta.env.BASE_URL}/`;
	return base + path.replace(/^\//, '');
}

export const GITHUB_URL = 'https://github.com/softberries/tayga';
export const CONTACT_EMAIL = 'hello@softberries.dev';
export const COMPANY_URL = 'https://softberries.dev';
