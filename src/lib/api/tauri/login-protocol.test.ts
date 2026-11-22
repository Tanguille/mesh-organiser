// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { configuration } from "#lib/configuration.svelte";
import { resetContainer } from "../dependency_injection";
import { configurationDefault } from "../shared/settings_api";
import { initTauriLocalApis } from "./init";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  fetch: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({
  appDataDir: async () => "/app-data",
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ setZoom: vi.fn() }),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: async () => null }));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));

afterEach(() => {
  resetContainer();
  Object.assign(configuration, configurationDefault());
  vi.restoreAllMocks();
  mocks.fetch.mockReset();
  mocks.invoke.mockReset();
});

it("advertises the group deletion protocol when logging in with a sync token", async () => {
  vi.spyOn(console, "log").mockImplementation(() => {});
  const user = {
    id: 1,
    username: "local",
    email: "local@noemail.com",
    created_at: "2000-01-01T00:00:00Z",
    permissions: [],
    sync_url: "https://example.test",
    sync_token: "test-token",
    last_sync: null,
  };
  mocks.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_initial_state":
        return {};
      case "get_configuration":
        return configurationDefault();
      case "get_current_user":
        return user;
    }
  });
  mocks.fetch.mockImplementation(async (url: string) =>
    url.endsWith("/users/me")
      ? new Response(JSON.stringify(user), { status: 200 })
      : new Response(null, { status: 204 }),
  );

  await initTauriLocalApis();

  const login = mocks.fetch.mock.calls.find(([url]) =>
    url.endsWith("/login/token"),
  );
  expect(login).toBeDefined();
  expect(login![0]).toBe("https://example.test/api/v1/login/token");
  expect(login![1].method).toBe("POST");
  expect(JSON.parse(login![1].body)).toEqual({
    token: "test-token",
  });
});
