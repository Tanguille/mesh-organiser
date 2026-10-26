import { getContainer } from "./api/dependency_injection";
import {
  defaultSidebarState,
  ISidebarStateApi,
} from "./api/shared/sidebar_state_api";

export const sidebarState = $state(defaultSidebarState());

export async function updateSidebarState(): Promise<void> {
  const loadSidebarState =
    getContainer().optional<ISidebarStateApi>(ISidebarStateApi);
  if (!loadSidebarState) {
    return;
  }

  Object.assign(sidebarState, await loadSidebarState());
}
