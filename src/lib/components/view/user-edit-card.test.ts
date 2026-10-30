// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IHostApi, Platform } from "#lib/api/shared/host_api";
import { IAdminUserApi } from "#lib/api/shared/user_api";
import UserEditCard from "./user-edit-card.svelte";

describe("UserEditCard", () => {
  afterEach(() => {
    resetContainer();
    vi.restoreAllMocks();
  });

  it("generates the new user's password with crypto.getRandomValues, not Math.random", async () => {
    getContainer().addSingleton(IAdminUserApi, {
      getAllUsers: vi.fn().mockResolvedValue([]),
    });
    getContainer().addSingleton(IHostApi, {
      getPlatform: vi.fn().mockResolvedValue(Platform.WebApp),
    });
    render(UserEditCard);
    await tick();
    // Spy only after mount: the placeholder user's email still uses Math.random.
    const mathRandom = vi.spyOn(Math, "random");
    const getRandomValues = vi.spyOn(crypto, "getRandomValues");

    await fireEvent.click(
      screen.getByRole("button", { name: /Create new user/ }),
    );

    const password = (await screen.findByLabelText(
      "Password",
    )) as HTMLInputElement;
    expect(password.value).toHaveLength(12);
    expect(getRandomValues).toHaveBeenCalled();
    expect(mathRandom).not.toHaveBeenCalled();
  });
});
