// @vitest-environment jsdom
/**
 * Pins that the local desktop init releases the links Tauri held back during
 * startup only once the import listeners and every api a link handler uses
 * are registered.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { configuration } from "#lib/configuration.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { IModelApi } from "../shared/model_api";
import type { RawUser } from "../shared/raw_model";
import { configurationDefault } from "../shared/settings_api";
import { ISlicerApi } from "../shared/slicer_api";
import { ITauriImportApi } from "../shared/tauri_import_api";
import { initTauriLocalApis } from "./init";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => vi.fn()),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: async () => "/app-data",
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ setZoom: vi.fn() }),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: async () => null }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({
  page: { url: new URL("http://localhost/") },
}));

function localUser(): RawUser {
  return {
    id: 1,
    username: "local",
    email: "local@noemail.com",
    created_at: "2000-01-01T00:00:00Z",
    permissions: [],
    sync_url: null,
    sync_token: null,
    last_sync: null,
  };
}

describe("initTauriLocalApis", () => {
  afterEach(() => {
    resetContainer();
    Object.assign(configuration, configurationDefault());
    vi.restoreAllMocks();
  });

  it("releases held links once the import listeners and every api are registered", async () => {
    // checkForUpdates logs the update it found.
    vi.spyOn(console, "log").mockImplementation(() => {});
    let atRelease: { listened: number; registered: boolean[] } | undefined;
    mocks.invoke.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_initial_state":
          return {};
        case "get_configuration":
          return configurationDefault();
        case "get_current_user":
          return localUser();
        case "release_pending_links":
          atRelease = {
            listened: mocks.listen.mock.calls.length,
            registered: [IModelApi, ISlicerApi, ITauriImportApi].map(
              (token) => getContainer().optional(token) !== null,
            ),
          };
      }
    });

    await initTauriLocalApis();

    expect(mocks.listen).toHaveBeenCalled();
    expect(atRelease).toEqual({
      listened: mocks.listen.mock.calls.length,
      registered: [true, true, true],
    });
    expect(
      mocks.invoke.mock.calls.filter(
        ([command]) => command === "release_pending_links",
      ),
    ).toHaveLength(1);
  });
});
