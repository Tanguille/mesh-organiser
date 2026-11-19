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
import {
  createGroupInstance,
  createGroupMetaInstance,
  IGroupApi,
} from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { createModelInstance, IModelApi } from "#lib/api/shared/model_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configurationMeta } from "#lib/configuration.svelte";
import { createRawSnippet } from "svelte";
import RightClickModels from "../view/right-click-models.svelte";

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
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

const timestamp = "2026-10-29T12:00:00Z";
const destination = createGroupMetaInstance(
  42,
  "Destination",
  timestamp,
  timestamp,
  "group-42",
);
function model(id: number) {
  return createModelInstance(
    id,
    `Part ${id}`,
    createBlobInstance(id, `hash-${id}`, "part.stl", 1024, timestamp),
    null,
    null,
    timestamp,
    timestamp,
    null,
    [],
    [],
    `model-${id}`,
  );
}
function provideApis(move = vi.fn().mockResolvedValue(undefined)) {
  const container = getContainer();
  const groups = [createGroupInstance(destination, [], [], null, [])];
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue(groups),
    addModelsToGroup: move,
  });
  container.addSingleton(IModelApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  return move;
}
afterEach(() => {
  cleanup();
  resetContainer();
  configurationMeta.applicationReadOnly = false;
  vi.restoreAllMocks();
});

async function openMenu(models: ReturnType<typeof model>[]) {
  render(RightClickModels, {
    models,
    children: createRawSnippet(() => ({ render: () => "<span>Models</span>" })),
  });
  await fireEvent.contextMenu(screen.getByText("Models"));
  return await screen.findByRole("menuitem", { name: /move to group/i });
}

it("moves context-selected models to an unrelated existing group", async () => {
  const move = provideApis();
  const parts = [model(1), model(2)];
  await fireEvent.click(await openMenu(parts));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() =>
    expect(move).toHaveBeenCalledWith(destination, [
      expect.objectContaining({ id: 1 }),
      expect.objectContaining({ id: 2 }),
    ]),
  );
  await waitFor(() =>
    expect(parts.map((part) => part.group?.id)).toEqual([42, 42]),
  );
});

it("keeps prior membership and reports failure when a context move fails", async () => {
  const move = provideApis(vi.fn().mockRejectedValue(new Error("offline")));
  vi.spyOn(console, "error").mockImplementation(() => {});
  const part = model(1);
  part.group = createGroupMetaInstance(
    8,
    "Original",
    timestamp,
    timestamp,
    "group-8",
  );
  await fireEvent.click(await openMenu([part]));
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(move).toHaveBeenCalled());
  expect(part.group?.id).toBe(8);
  expect(await screen.findByRole("alert")).toBeTruthy();
});

it("does not offer context group changes in read-only mode", async () => {
  provideApis();
  configurationMeta.applicationReadOnly = true;
  render(RightClickModels, {
    models: [model(1)],
    children: createRawSnippet(() => ({ render: () => "<span>Models</span>" })),
  });
  await fireEvent.contextMenu(screen.getByText("Models"));
  await screen.findByRole("menuitem", { name: /selected/i });
  expect(screen.queryByRole("menuitem", { name: /move to group/i })).toBeNull();
});

it("opens the context group picker with the keyboard", async () => {
  provideApis();
  const item = await openMenu([model(1)]);
  item.focus();
  await fireEvent.keyDown(item, { key: "Enter" });
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "Destination" }),
    ).not.toBeNull(),
  );
});

it("reports the original models after selection changes during a move", async () => {
  let finish!: () => void;
  const move = provideApis(
    vi.fn().mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    ),
  );
  const first = model(1);
  const next = model(2);
  const onModelsMoved = vi.fn();
  const children = createRawSnippet(() => ({
    render: () => "<span>Models</span>",
  }));
  const view = render(RightClickModels, {
    models: [first],
    onModelsMoved,
    children,
  });
  await fireEvent.contextMenu(screen.getByText("Models"));
  await fireEvent.click(
    await screen.findByRole("menuitem", { name: /move to group/i }),
  );
  await fireEvent.click(
    await screen.findByRole("button", { name: "Destination" }),
  );
  await waitFor(() => expect(move).toHaveBeenCalled());
  await view.rerender({ models: [next], onModelsMoved, children });
  finish();
  await waitFor(() =>
    expect(onModelsMoved).toHaveBeenCalledWith(
      [expect.objectContaining({ id: 1 })],
      destination,
    ),
  );
  expect(first.group?.id).toBe(42);
  expect(next.group).toBeNull();
});
