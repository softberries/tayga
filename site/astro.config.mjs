// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLinksValidator from 'starlight-links-validator';

const GITHUB = 'https://github.com/softberries/tayga';

// Code block surfaces follow the site tokens (src/styles/theme.css): --tg-code-bg per theme.
/** @param {import('@astrojs/starlight/expressive-code').ExpressiveCodeTheme} theme */
function tintCodeTheme(theme) {
	const dark = theme.type === 'dark';
	theme.colors['editor.background'] = dark ? '#0b1428' : '#f5f8fc';
	theme.colors['editorGroupHeader.tabsBackground'] = dark ? '#0e162a' : '#eef2f8';
	theme.colors['tab.activeBackground'] = dark ? '#0b1428' : '#f5f8fc';
	theme.colors['terminal.background'] = dark ? '#0b1428' : '#f5f8fc';
	return theme;
}

export default defineConfig({
	site: 'https://softberries.github.io',
	base: '/tayga/',
	trailingSlash: 'always',
	outDir: './dist',
	// Inline the CSS (about 20 kB) so it does not block the first paint with extra requests.
	build: { inlineStylesheets: 'always' },
	integrations: [
		starlight({
			title: 'Tayga',
			description:
				'Tayga turns OpenTelemetry traces and logs into error stories: root cause, request path, critical path, what differs from normal, and the related logs.',
			logo: { src: './src/assets/logo-mark-72.webp', alt: 'Tayga' },
			favicon: '/favicon-32.png',
			head: [
				{ tag: 'link', attrs: { rel: 'icon', href: '/tayga/favicon-192.png', type: 'image/png', sizes: '192x192' } },
				{ tag: 'link', attrs: { rel: 'apple-touch-icon', href: '/tayga/apple-touch-icon.png' } },
				{ tag: 'meta', attrs: { name: 'theme-color', content: '#0a1020' } },
			],
			social: [
				{ icon: 'github', label: 'GitHub', href: GITHUB },
				{ icon: 'email', label: 'Email Softberries', href: 'mailto:hello@softberries.dev' },
			],
			editLink: { baseUrl: `${GITHUB}/edit/master/site/` },
			lastUpdated: false,
			customCss: ['./src/styles/fonts.css', './src/styles/theme.css'],
			components: {
				Hero: './src/components/Hero.astro',
			},
			expressiveCode: {
				themes: ['github-dark-default', 'github-light-default'],
				useStarlightUiThemeColors: true,
				customizeTheme: tintCodeTheme,
				styleOverrides: {
					borderRadius: '12px',
					borderColor: 'var(--sl-color-hairline-light)',
					codeFontFamily: 'var(--__sl-font-mono)',
					uiFontFamily: 'var(--__sl-font)',
					codeFontSize: '0.8125rem',
					codeLineHeight: '1.65',
					codePaddingBlock: '1rem',
					codePaddingInline: '1.15rem',
					frames: {
						shadowColor: 'transparent',
						editorActiveTabIndicatorTopColor: 'var(--sl-color-accent)',
						frameBoxShadowCssValue: 'var(--tg-shadow-code)',
					},
				},
			},
			plugins: [starlightLinksValidator({ errorOnRelativeLinks: false })],
			sidebar: [
				{
					label: 'Getting started',
					items: [
						{ label: 'What is Tayga', slug: 'getting-started/what-is-tayga' },
						{ label: 'Quickstart', slug: 'getting-started/quickstart' },
						{ label: 'Demo with the OTel demo', slug: 'getting-started/otel-demo' },
					],
				},
				{
					label: 'Install',
					items: [
						{ label: 'Docker Compose', slug: 'install/docker-compose' },
						{ label: 'Helm and Kubernetes', slug: 'install/helm' },
						{ label: 'From source', slug: 'install/from-source' },
						{ label: 'Connect your Collector', slug: 'install/collector' },
						{ label: 'Upgrading', slug: 'install/upgrading' },
						{ label: 'Uninstalling', slug: 'install/uninstalling' },
					],
				},
				{
					label: 'Concepts',
					items: [
						{ label: 'Architecture', slug: 'concepts/architecture' },
						{ label: 'Error stories', slug: 'concepts/error-stories' },
						{ label: 'Root cause and critical path', slug: 'concepts/root-cause-critical-path' },
						{ label: 'Baselines', slug: 'concepts/baselines' },
						{ label: 'Story groups and fingerprints', slug: 'concepts/story-groups' },
						{ label: 'Service map', slug: 'concepts/service-map' },
						{ label: 'Log templates', slug: 'concepts/log-templates' },
						{ label: 'Log alerts', slug: 'concepts/log-alerts' },
						{ label: 'Replicas and partitioning', slug: 'concepts/replicas' },
						{ label: 'The data clock', slug: 'concepts/data-clock' },
					],
				},
				{
					label: 'User guide',
					collapsed: true,
					items: [
						{ label: 'Stories', slug: 'guide/stories' },
						{ label: 'Story detail', slug: 'guide/story-detail' },
						{ label: 'Traces', slug: 'guide/traces' },
						{ label: 'Service map', slug: 'guide/service-map' },
						{ label: 'Logs', slug: 'guide/logs' },
						{ label: 'Alerts', slug: 'guide/alerts' },
						{ label: 'Pipeline', slug: 'guide/pipeline' },
						{ label: 'Command palette', slug: 'guide/command-palette' },
						{ label: 'Theme', slug: 'guide/theme' },
						{ label: 'Login', slug: 'guide/login' },
					],
				},
				{
					label: 'Alerting',
					collapsed: true,
					items: [
						{ label: 'The notifier', slug: 'alerting/notifier' },
						{ label: 'Webhook and Slack formats', slug: 'alerting/formats' },
						{ label: 'Delivery semantics', slug: 'alerting/delivery' },
					],
				},
				{
					label: 'Operations',
					collapsed: true,
					items: [
						{ label: 'Configuration reference', slug: 'operations/configuration' },
						{ label: 'Authentication', slug: 'operations/authentication' },
						{ label: 'Scaling logminer replicas', slug: 'operations/scaling-logminer' },
						{ label: 'Retention and disk', slug: 'operations/retention' },
						{ label: 'Re-mining templates', slug: 'operations/remine' },
						{ label: 'Metrics and Grafana', slug: 'operations/metrics-grafana' },
						{ label: 'Performance tuning', slug: 'operations/performance-tuning' },
						{ label: 'Troubleshooting', slug: 'operations/troubleshooting' },
					],
				},
				{
					label: 'Reference',
					items: [{ label: 'HTTP API', slug: 'reference/api' }],
				},
				{
					label: 'Project',
					items: [
						{ label: 'Performance', slug: 'performance' },
						{ label: 'Comparison', slug: 'comparison' },
						{ label: 'Enterprise', slug: 'enterprise', badge: { text: 'On request', variant: 'tip' } },
						{ label: 'License', slug: 'license' },
						{ label: 'FAQ', slug: 'faq' },
						{ label: 'Changelog', slug: 'changelog' },
						{ label: 'Verified claims', slug: 'verified' },
					],
				},
			],
		}),
	],
});
