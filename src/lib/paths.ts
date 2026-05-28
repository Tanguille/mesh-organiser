import { resolve as kitResolve } from "$app/paths";
import { goto } from "$app/navigation";
import { page } from "$app/state";

/**
 * Resolve a route path to an absolute path. Use this instead of $app/paths resolve
 * so that both static and dynamic paths type-check and satisfy svelte/no-navigation-without-resolve.
 */
export function resolve(path: string): string {
  // SvelteKit's resolve() only accepts typed route literals; dynamic paths need a cast
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- dynamic route path
  return kitResolve(path as any);
}

/**
 * Whether the current route's `thisLabelOnly` query param is set, controlling
 * whether label views restrict to the label itself versus its descendants.
 */
export function getThisLabelOnly(): boolean {
  return page.url.searchParams.get("thisLabelOnly") === "true";
}

/**
 * After switching/linking a user, group and label detail routes reference the
 * previous user's data, so navigate away from them before reloading.
 */
export async function redirectAfterUserSwitch(): Promise<void> {
  if (location.href.includes("/group/")) {
    await goto(resolve("/group"));
  }

  if (location.href.includes("/label/")) {
    await goto(resolve("/"));
  }
}
