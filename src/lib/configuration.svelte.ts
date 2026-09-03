import { SvelteDate } from "svelte/reactivity";
import { getContainer } from "./api/dependency_injection";
import {
  configurationDefault,
  ISettingsApi,
  type Configuration,
} from "./api/shared/settings_api";
import { type User } from "./api/shared/user_api";

// TODO: Change this to use the same structure as useSidebar()
export const configuration = $state(configurationDefault());
export const configurationMeta = $state({
  configurationLoaded: false,
  applicationReadOnly: false,
});
export const currentUser = $state<User>({
  id: -1,
  username: "",
  email: "",
  created: new SvelteDate(),
  permissions: {
    admin: false,
    sync: false,
    onlineAccount: false,
  },
  syncUrl: null,
  syncToken: null,
  lastSync: null,
});

export async function updateConfiguration(
  config: Configuration,
): Promise<void> {
  const settingsApi = getContainer().optional<ISettingsApi>(ISettingsApi);

  if (!settingsApi) {
    console.warn("No settings API available to save configuration");
    return;
  }

  await settingsApi.saveConfiguration(config);
}

export const panicState = $state({
  inPanic: false,
  message: "",
});
