import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { IModelApi } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { createUserInstance } from "../shared/user_api";
import { SyncApi } from "./sync";

const calls = vi.hoisted(() => ({
  invoke: vi.fn(),
  syncModels: vi.fn(),
  syncGroups: vi.fn(),
  syncLabels: vi.fn(),
  syncResources: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: calls.invoke }));
vi.mock("./sync-models", () => ({ syncModels: calls.syncModels }));
vi.mock("./sync-groups", () => ({ syncGroups: calls.syncGroups }));
vi.mock("./sync-labels", () => ({ syncLabels: calls.syncLabels }));
vi.mock("./sync-resources", () => ({ syncResources: calls.syncResources }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));

const onlineUser = createUserInstance(
  1,
  "user",
  "user@example.test",
  "2024-01-01T00:00:00Z",
  [],
  null,
  null,
  null,
);

beforeEach(() => {
  vi.clearAllMocks();
  resetContainer();
  currentUser.lastSync = new Date("2025-01-01T00:00:00Z");
  getContainer().addSingleton(IModelApi, {
    getModels: vi.fn().mockResolvedValue([]),
  });
});

afterEach(() => {
  resetContainer();
  currentUser.lastSync = null;
});

function createSync(capability: unknown | Error) {
  const request = vi.fn(async (path: string) => {
    if (path === "/sync/capabilities") {
      if (capability instanceof Error) throw capability;
      return capability;
    }
    if (path === "/models") return [];
    throw new Error(`Unexpected request: ${path}`);
  });
  const requestApi = { baseUrl: "https://example.test", request };
  return {
    request,
    sync: new SyncApi(
      requestApi as unknown as IServerRequestApi,
      onlineUser,
      requestApi.baseUrl,
    ),
  };
}

describe("sync protocol compatibility", () => {
  it.each([
    ["a missing capability", {}],
    ["a null capability response", null],
    ["a malformed capability response", "supported"],
    ["an unsupported version", { group_deletion_protocol: 2 }],
    ["an old server without the endpoint", new Error("HTTP 404")],
    ["a failed capability lookup", new Error("network unavailable")],
  ])("stops before sync writes for %s", async (_description, response) => {
    const { request, sync } = createSync(response);
    const previousSync = currentUser.lastSync;

    await expect(sync.syncData()).rejects.toThrow(
      /upgrade|protocol|compatib|unavailable|network/i,
    );

    expect(request).toHaveBeenCalledExactlyOnceWith(
      "/sync/capabilities",
      HttpMethod.GET,
      undefined,
      "2",
    );
    expect(calls.syncModels).not.toHaveBeenCalled();
    expect(calls.syncGroups).not.toHaveBeenCalled();
    expect(calls.syncLabels).not.toHaveBeenCalled();
    expect(calls.syncResources).not.toHaveBeenCalled();
    expect(calls.invoke).not.toHaveBeenCalled();
    expect(currentUser.lastSync).toBe(previousSync);
  });

  it("stops before writes when the server supports a protocol the client cannot safely use", async () => {
    const { request, sync } = createSync({ group_deletion_protocol: 1 });
    const previousSync = currentUser.lastSync;

    await expect(sync.syncData()).rejects.toThrow(
      /unavailable|not (?:yet )?available/i,
    );

    expect(
      request.mock.calls.every(([path]) => path === "/sync/capabilities"),
    ).toBe(true);
    expect(calls.syncModels).not.toHaveBeenCalled();
    expect(calls.syncGroups).not.toHaveBeenCalled();
    expect(calls.syncLabels).not.toHaveBeenCalled();
    expect(calls.syncResources).not.toHaveBeenCalled();
    expect(calls.invoke).not.toHaveBeenCalled();
    expect(currentUser.lastSync).toBe(previousSync);
  });
});
