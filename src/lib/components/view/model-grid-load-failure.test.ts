// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { createBlobInstance, IBlobApi } from "#lib/api/shared/blob_api";
import { IGroupApi } from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import {
  createModelInstance,
  IModelApi,
  type IModelStreamManager,
} from "#lib/api/shared/model_api";
import ModelGrid from "./model-grid.svelte";

vi.hoisted(() => {
  window.matchMedia ??= (query: string) =>
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

afterEach(() => {
  cleanup();
  resetContainer();
  vi.restoreAllMocks();
});

it("keeps unloaded models out of bulk actions after a failed first page", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const container = getContainer();
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue(""),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const timestamp = "2026-10-24T12:00:00Z";
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl", 1024, timestamp),
      null,
      null,
      timestamp,
      timestamp,
      null,
      [],
      [],
      `model-${id}`,
    ),
  );
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi
      .fn()
      .mockRejectedValueOnce(new Error("network failure"))
      .mockResolvedValueOnce(models),
    getAll: vi.fn().mockResolvedValue(models),
  };
  render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    default_show_multiselect_all: true,
  });

  const retry = await screen.findByRole("button", {
    name: /failed to load models.*retry/i,
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(screen.queryByText("2 models")).toBeNull();
  expect(screen.getByText("No models to display")).toBeTruthy();

  await fireEvent.click(retry);
  await waitFor(() => expect(screen.getByText("2 models")).toBeTruthy());
});

it("keeps paging when only the bulk prefetch fails", async () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  const container = getContainer();
  container.addSingleton(IGroupApi, {});
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue(""),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  const timestamp = "2026-10-24T12:00:00Z";
  const models = [1, 2].map((id) =>
    createModelInstance(
      id,
      `Part ${id}`,
      createBlobInstance(id, `hash-${id}`, "stl", 1024, timestamp),
      null,
      null,
      timestamp,
      timestamp,
      null,
      [],
      [],
      `model-${id}`,
    ),
  );
  const stream = {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValue([]),
    getAll: vi.fn().mockRejectedValue(new Error("bulk timeout")),
  };
  stream.fetch.mockResolvedValueOnce([models[0]]);
  render(ModelGrid, {
    modelStream: stream as IModelStreamManager,
    default_show_multiselect_all: true,
  });

  await waitFor(() => expect(stream.getAll).toHaveBeenCalled());
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(
    screen.queryByRole("button", { name: /failed to load models.*retry/i }),
  ).toBeNull();
});
