/**
 * Characterization tests for WebUserAdminApi's edit requests. The self variants
 * target the user the API was constructed with, the admin variants target the
 * user passed in; method, url and body are the contract with the web server.
 */

import { describe, expect, it, vi } from "vitest";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { User } from "../shared/user_api";
import { WebUserAdminApi } from "./user_admin";

function user(id: number, username: string): User {
  return {
    id,
    username,
    email: `${username}@example.com`,
    created: new Date(0),
    permissions: { admin: true, sync: false, onlineAccount: true },
    syncUrl: null,
    syncToken: null,
    lastSync: null,
  };
}

const currentUser = user(1, "admin");
const otherUser = user(7, "bob");

function createApi() {
  const request = vi.fn().mockResolvedValue(undefined);
  const requestApi = { request } as unknown as IServerRequestApi;

  return { request, api: new WebUserAdminApi(requestApi, currentUser) };
}

describe("WebUserAdminApi password edits", () => {
  it("editSelfPassword PUTs the new password for the current user", async () => {
    const { request, api } = createApi();

    await api.editSelfPassword("hunter2");

    expect(request.mock.calls).toEqual([
      ["/users/1/password", HttpMethod.PUT, { new_password: "hunter2" }],
    ]);
  });

  it("editUserPassword PUTs the new password for the given user", async () => {
    const { request, api } = createApi();

    await api.editUserPassword(otherUser, "correct horse");

    expect(request.mock.calls).toEqual([
      ["/users/7/password", HttpMethod.PUT, { new_password: "correct horse" }],
    ]);
  });
});

describe("WebUserAdminApi user edits", () => {
  it("editSelf PUTs name and email for the given user", async () => {
    const { request, api } = createApi();

    await api.editSelf(otherUser);

    expect(request.mock.calls).toEqual([
      [
        "/users/7",
        HttpMethod.PUT,
        { user_name: "bob", user_email: "bob@example.com" },
      ],
    ]);
  });

  it("editUser PUTs name and email, then the permission strings", async () => {
    const { request, api } = createApi();

    await api.editUser(otherUser);

    expect(request.mock.calls).toEqual([
      [
        "/users/7",
        HttpMethod.PUT,
        { user_name: "bob", user_email: "bob@example.com" },
      ],
      [
        "/users/7/permissions",
        HttpMethod.PUT,
        { permissions: ["Admin", "OnlineAccount"] },
      ],
    ]);
  });
});
