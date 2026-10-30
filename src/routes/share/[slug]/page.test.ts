// @vitest-environment jsdom
/**
 * Behaviour tests for the share page: the loading spinner gives way to an empty
 * state for a share without models, and to an error when the share fails to
 * load, instead of spinning forever.
 */

import { render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IGroupApi } from "#lib/api/shared/group_api";
import { IShareApi, type Share } from "#lib/api/shared/share_api";
import Page from "./+page.svelte";

vi.hoisted(() => {
  // jsdom has no matchMedia; svelte/reactivity touches it at import time.
  window.matchMedia ??= (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    }) as unknown as MediaQueryList;
});

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({
  page: { url: new URL("http://localhost/"), params: { slug: "share-1" } },
}));
// Threlte needs WebGL, which jsdom lacks; these tests never open a model preview.
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

function provideApis(getShare: () => Promise<Share>) {
  const groupApi = { getGroups: vi.fn().mockResolvedValue([]) };
  const container = getContainer();
  container.addSingleton(IShareApi, { getShare: vi.fn(getShare) });
  container.addSingleton(IGroupApi, groupApi);

  return { groupApi };
}

describe("share [slug] +page", () => {
  afterEach(() => {
    resetContainer();
    vi.restoreAllMocks();
  });

  it("shows the empty state for a share without models", async () => {
    const { groupApi } = provideApis(async () => ({
      id: "share-1",
      createdAt: new Date(0),
      shareName: "Empty",
      userName: "bob",
      modelIds: [],
    }));

    render(Page);

    expect(await screen.findByText("This share has no models.")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
    // The server rejects a page size of 0, so an empty share skips the fetch.
    expect(groupApi.getGroups).not.toHaveBeenCalled();
  });

  it("shows an error when the share fails to load", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    provideApis(() => Promise.reject(new Error("offline")));

    render(Page);

    expect(await screen.findByText("Could not load this share.")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
