import { resolve as kitResolve } from "$app/paths";

/**
 * Resolve a route path to an absolute path. Use this instead of $app/paths resolve
 * so that both static and dynamic paths type-check and satisfy svelte/no-navigation-without-resolve.
 */
export function resolve(path: string): string {
  // SvelteKit's resolve() only accepts typed route literals; dynamic paths need a cast
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic route path
  return kitResolve(path as any);
}
