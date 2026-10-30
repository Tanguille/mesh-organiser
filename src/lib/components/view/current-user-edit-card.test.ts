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

// Renders the card for `account` on `platform`, renames it, and returns the
// editSelf mock once the save debounce has passed.
async function renameAccount(platform: Platform, account: User) {
  const editSelf = vi.fn().mockResolvedValue(undefined);
  const container = getContainer();
  container.addSingleton(IHostApi, { getPlatform: async () => platform });
  container.addSingleton(IUserApi, { getCurrentUser: async () => account });
  container.addSingleton(IUserManageSelfApi, { editSelf });

  render(CurrentUserEditCard);
  const username = await screen.findByDisplayValue(account.username);
  vi.useFakeTimers();
  await fireEvent.input(username, { target: { value: "renamed" } });
  await vi.advanceTimersByTimeAsync(1000);

  return editSelf;
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

  it("edits the account getCurrentUser returns in web mode", async () => {
    Object.assign(currentUser, user(1, "local", false));

    const editSelf = await renameAccount(
      Platform.WebApp,
      user(42, "online", false),
    );

    expect(editSelf).toHaveBeenCalledWith(
      expect.objectContaining({ id: 42, username: "renamed" }),
    );
    expect(currentUser.username).toBe("local");
  });

  it("edits the account getCurrentUser returns in desktop online-account mode", async () => {
    // On desktop, currentUser is the local user even for an online account.
    Object.assign(currentUser, user(1, "local", true));

    const editSelf = await renameAccount(
      Platform.TauriOfflineDesktop,
      user(42, "online", true),
    );

    expect(editSelf).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 42,
        username: "renamed",
        email: "online@example.com",
      }),
    );
    expect(currentUser.username).toBe("local");
  });
});
