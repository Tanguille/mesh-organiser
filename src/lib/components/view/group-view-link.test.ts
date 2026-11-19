// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { createBlobInstance, IBlobApi } from "#lib/api/shared/blob_api";
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
  type IGroupStreamManager,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { createModelInstance, IModelApi } from "#lib/api/shared/model_api";
import { IResourceApi } from "#lib/api/shared/resource_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration, configurationMeta } from "#lib/configuration.svelte";
import { configurationDefault } from "#lib/api/shared/settings_api";

import GroupGrid from "./group-grid.svelte";

vi.hoisted(() => {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
  HTMLElement.prototype.scrollIntoView = () => {};
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
  configurationMeta.applicationReadOnly = false;
});
function setup() {
  const container = getContainer();
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IResourceApi, {
    getResources: vi.fn().mockResolvedValue([]),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const date = "2026-10-24T12:00:00Z";
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl.zip", 1024, date),
      null,
      null,
      date,
      date,
      null,
      [],
      [],
      `model-${id}`,
    ),
  );
  const group = createGroupInstance(
    createGroupMetaInstance(7, "Calibration", date, date, "group-7"),
    models,
    [],
    null,
    [],
  );
  const stream = (items: unknown[]) => ({
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValueOnce(items).mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue(items),
  });
  return { models, group, stream };
}

it.each([0, 1])(
  "shows a View models link for a real group with %s models",
  async (count) => {
    const { group, stream } = setup();
    group.models = group.models.slice(0, count);
    configuration.group_split_view = "no_split";
    render(GroupGrid, { groupStream: stream([group]) as IGroupStreamManager });
    const option = await screen.findByRole("option", { name: /Calibration/ });
    await fireEvent.click(option);
    const link = screen.queryByRole("link", { name: /view models/i });
    expect(link).not.toBeNull();
    expect(link!.getAttribute("href")).toBe("/group/7");
  },
);
