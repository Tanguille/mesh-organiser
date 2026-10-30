// @vitest-environment jsdom
// `.svelte.test.ts` so the share prop can be a deeply reactive `$state` proxy, like the real callers pass.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IShareApi, type Share } from "#lib/api/shared/share_api";
import ShareCard from "./share.svelte";

function makeShare(id: string): Share {
  return {
    id,
    createdAt: new Date(0),
    shareName: `Share ${id}`,
    userName: "admin",
    modelIds: [1, 2],
  };
}

function provideShareApi() {
  const shareApi = {
    getShareLink: vi.fn(
      async (share: Share) => `https://example.com/share/${share.id}`,
    ),
    editShare: vi.fn().mockResolvedValue(undefined),
  };
  getContainer().addSingleton(IShareApi, shareApi);

  return shareApi;
}

describe("Share", () => {
  afterEach(() => {
    resetContainer();
  });

  it("fetches the share link once while the share name is edited", async () => {
    const shareApi = provideShareApi();
    const share = $state(makeShare("a"));
    render(ShareCard, { share });

    const nameInput = screen.getByLabelText("Share name");
    await fireEvent.input(nameInput, { target: { value: "Renamed" } });
    await fireEvent.input(nameInput, { target: { value: "Renamed again" } });
    await tick();

    expect(share.shareName).toBe("Renamed again");
    expect(shareApi.getShareLink).toHaveBeenCalledTimes(1);
  });

  it("fetches the share link again for a share with a different id", async () => {
    const shareApi = provideShareApi();
    const first = $state(makeShare("a"));
    const second = $state(makeShare("b"));
    const { rerender } = render(ShareCard, { share: first });

    rerender({ share: second });
    await tick();

    expect(shareApi.getShareLink).toHaveBeenCalledTimes(2);
    expect(shareApi.getShareLink).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "b" }),
    );
  });
});
