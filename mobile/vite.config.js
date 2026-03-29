import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'path';

/** @type {import('vite').UserConfig} */
const config = {
	plugins: [svelte()],
	resolve: {
		alias: {
			'$lib': resolve('./src/lib'),
			'$lib/*': resolve('./src/lib/*')
		}
	},
	test: {
		include: ['src/**/*.{test,spec}.{js,ts}']
	}
};

export default config;