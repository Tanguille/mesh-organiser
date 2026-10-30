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
import { IUserManageSelfApi, type User } from "../shared/user_api";
import { initTauriOnlineAccountApi } from "./init";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));
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
  });

  it("builds blob download urls for the online user", async () => {
    await init();
    const blobApi = getContainer().require<IBlobApi>(IBlobApi);

    const url = await blobApi.getBlobDownloadUrl({ sha256: "abc" } as Blob);

    expect(url).toBe(
      `${hostUrl}/api/v1/blobs/abc/download?user_hash=server-user-hash&user_id=42`,
    );
  });

  it("edits the online user's details and password, not the local user's", async () => {
    const { request } = await init();
    const manageSelfApi =
      getContainer().require<IUserManageSelfApi>(IUserManageSelfApi);

    // The settings card passes the global current user, here the local user.
    await manageSelfApi.editSelf({ ...currentUser, username: "renamed" });
    await manageSelfApi.editSelfPassword("hunter22");

    expect(request.mock.calls).toEqual([
      [
        "/users/42",
        HttpMethod.PUT,
        { user_name: "renamed", user_email: "local@example.com" },
      ],
      ["/users/42/password", HttpMethod.PUT, { new_password: "hunter22" }],
    ]);
  });
});
