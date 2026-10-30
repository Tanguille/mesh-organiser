// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IHostApi, Platform } from "#lib/api/shared/host_api";
import {
  IUserApi,
  IUserManageSelfApi,
  type User,
} from "#lib/api/shared/user_api";
import { currentUser } from "#lib/configuration.svelte";
import CurrentUserEditCard from "./current-user-edit-card.svelte";

function user(id: number, username: string, onlineAccount: boolean): User {
  return {
    id,
    username,
    email: `${username}@example.com`,
    created: new Date(0),
    permissions: { admin: false, sync: true, onlineAccount },
    syncUrl: null,
    syncToken: null,
    lastSync: null,
  };
}

const initialUser = {
  ...currentUser,
  permissions: { ...currentUser.permissions },
};

describe("CurrentUserEditCard", () => {
  afterEach(() => {
    vi.useRealTimers();
    resetContainer();
    Object.assign(currentUser, initialUser);
  });

  it("labels the password confirmation input with 'Verify new password'", () => {
    render(CurrentUserEditCard);

    expect(screen.getByLabelText("Verify new password").id).toBe("password2");
  });

  it("edits the server account, not the local user, in desktop online-account mode", async () => {
    Object.assign(currentUser, user(1, "local", true));
    const editSelf = vi.fn().mockResolvedValue(undefined);
    const container = getContainer();
    container.addSingleton(IHostApi, {
      getPlatform: async () => Platform.TauriOfflineDesktop,
    });
    container.addSingleton(IUserApi, {
      getCurrentUser: async () => user(42, "online", true),
    });
    container.addSingleton(IUserManageSelfApi, { editSelf });

    render(CurrentUserEditCard);
    const username = await screen.findByDisplayValue("online");
    vi.useFakeTimers();
    await fireEvent.input(username, { target: { value: "renamed" } });
    // Past the save debounce.
    await vi.advanceTimersByTimeAsync(1000);

    expect(editSelf).toHaveBeenCalledWith(
      expect.objectContaining({
        username: "renamed",
        email: "online@example.com",
      }),
    );
    expect(currentUser.username).toBe("local");
  });

  it("edits the global current user in web mode", async () => {
    Object.assign(currentUser, user(42, "online", true));
    const editSelf = vi.fn().mockResolvedValue(undefined);
    const getCurrentUser = vi.fn();
    const container = getContainer();
    container.addSingleton(IHostApi, {
      getPlatform: async () => Platform.WebApp,
    });
    container.addSingleton(IUserApi, { getCurrentUser });
    container.addSingleton(IUserManageSelfApi, { editSelf });

    render(CurrentUserEditCard);
    const username = await screen.findByDisplayValue("online");
    vi.useFakeTimers();
    await fireEvent.input(username, { target: { value: "renamed" } });
    await vi.advanceTimersByTimeAsync(1000);

    expect(currentUser.username).toBe("renamed");
    expect(editSelf).toHaveBeenCalledWith(
      expect.objectContaining({ id: 42, username: "renamed" }),
    );
    expect(getCurrentUser).not.toHaveBeenCalled();
  });
});
