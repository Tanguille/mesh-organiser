/**
 * Pins that desktop online-account mode builds its web APIs for the logged-in
 * server user, not the global current user: on desktop that is the local user,
 * with a local id and the server url as its syncUrl.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { currentUser } from "#lib/configuration.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { IBlobApi, type Blob } from "../shared/blob_api";
import { HttpMethod, IServerRequestApi } from "../shared/server_request_api";
import { IModelApi } from "../shared/model_api";
import { IShareApi } from "../shared/share_api";
import { ISlicerApi } from "../shared/slicer_api";
import { IUserManageSelfApi, type User } from "../shared/user_api";
import { initTauriOnlineAccountApi } from "./init";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => vi.fn()),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({
  page: { url: new URL("http://localhost/") },
}));

const hostUrl = "https://mesh.example.com";

function user(id: number, username: string, syncUrl: string): User {
  return {
    id,
    username,
    email: `${username}@example.com`,
    created: new Date(0),
    permissions: { admin: false, sync: true, onlineAccount: true },
    syncUrl,
    syncToken: null,
    lastSync: null,
  };
}

const localUser = user(1, "local", hostUrl);
const onlineUser = user(42, "online", "server-user-hash");

async function init() {
  const request = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(IServerRequestApi, { request });
  Object.assign(currentUser, localUser);

  await initTauriOnlineAccountApi(onlineUser, hostUrl, "/app-data");

  return { request };
}

describe("initTauriOnlineAccountApi", () => {
  afterEach(() => {
    resetContainer();
    mocks.invoke.mockReset();
    mocks.listen.mockClear();
  });

  it("releases held links once the import listeners and every api are registered", async () => {
    let atRelease: { listened: number; registered: boolean[] } | undefined;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "release_pending_links") {
        atRelease = {
          listened: mocks.listen.mock.calls.length,
          registered: [IModelApi, ISlicerApi, IShareApi].map(
            (token) => getContainer().optional(token) !== null,
          ),
        };
      }
    });

    await init();

    expect(mocks.listen).toHaveBeenCalled();
    expect(atRelease).toEqual({
      listened: mocks.listen.mock.calls.length,
      registered: [true, true, true],
    });
    expect(mocks.invoke.mock.calls).toEqual([["release_pending_links"]]);
  });

  it("builds blob download urls for the online user", async () => {
    await init();
    const blobApi = getContainer().require<IBlobApi>(IBlobApi);

    const url = await blobApi.getBlobDownloadUrl({ sha256: "abc" } as Blob);

    expect(url).toBe(
      `${hostUrl}/api/v1/blobs/abc/download?user_hash=server-user-hash&user_id=42`,
    );
  });

  it("changes the online user's password, not the local user's", async () => {
    const { request } = await init();
    const manageSelfApi =
      getContainer().require<IUserManageSelfApi>(IUserManageSelfApi);

    await manageSelfApi.editSelfPassword("hunter22");

    expect(request.mock.calls).toEqual([
      ["/users/42/password", HttpMethod.PUT, { new_password: "hunter22" }],
    ]);
  });
});
