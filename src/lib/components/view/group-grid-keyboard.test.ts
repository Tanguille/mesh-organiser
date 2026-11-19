// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import {
  IGroupApi,
  createGroupInstance,
  createGroupMetaInstance,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { IModelApi, createModelInstance } from "#lib/api/shared/model_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configurationDefault } from "#lib/api/shared/settings_api";
import { configuration } from "#lib/configuration.svelte";
import GroupGrid from "./group-grid.svelte";

vi.hoisted(() => {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
});

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

afterEach(() => {
  cleanup();
  resetContainer();
  Object.assign(configuration, configurationDefault());
});

it("selects all groups with Ctrl+A and clears them with Escape", async () => {
  const date = "2026-10-24T12:00:00Z";
  const groups = [1, 2].map((id) => {
    const meta = createGroupMetaInstance(
      id,
      `Group ${id}`,
      date,
      date,
      `group-${id}`,
    );
    const model = createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 100, date),
      null,
      null,
      date,
      date,
      meta,
      [],
      [],
      `model-${id}`,
    );
    return createGroupInstance(meta, [model], [], null, []);
  });
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  configuration.group_split_view = "no_split";
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(groups).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(groups),
  };
  render(GroupGrid, { groupStream: stream as IGroupStreamManager });
  const first = await screen.findByRole("option", { name: /Group 1/ });
  const second = screen.getByRole("option", { name: /Group 2/ });
  first.focus();
  await fireEvent.keyDown(first, { key: "a", ctrlKey: true });
  expect(first.getAttribute("aria-selected")).toBe("true");
  expect(second.getAttribute("aria-selected")).toBe("true");
  await fireEvent.keyDown(first, { key: "Escape" });
  expect(first.getAttribute("aria-selected")).toBe("false");
  expect(second.getAttribute("aria-selected")).toBe("false");
});
