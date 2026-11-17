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
import {
  IGroupApi,
  createGroupInstance,
  createGroupMetaInstance,
} from "#lib/api/shared/group_api";
import { ILabelApi, createLabelMetaInstance } from "#lib/api/shared/label_api";
import { createBlobInstance } from "#lib/api/shared/blob_api";
import { createModelInstance } from "#lib/api/shared/model_api";
import { sidebarState } from "#lib/sidebar_data.svelte";
import { configurationMeta } from "#lib/configuration.svelte";
import {
  state as dragState,
  stopDragging,
} from "#lib/drag-selected-models.svelte";
import DragFixture from "./test/drag-fixture.svelte";

const date = "2026-10-24T12:00:00Z";
const group = createGroupInstance(
  createGroupMetaInstance(7, "Group", date, date, "group-7"),
  [],
  [],
  null,
  [],
);
const label = createLabelMetaInstance(9, "Label", 0, date, "label-9");
const model = createModelInstance(
  1,
  "Part",
  createBlobInstance(1, "hash", "stl.zip", 100, date),
  null,
  null,
  date,
  date,
  null,
  [],
  [],
  "model-1",
);

afterEach(() => {
  cleanup();
  stopDragging();
  resetContainer();
  sidebarState.labels = [];
  configurationMeta.applicationReadOnly = false;
});

function setup(
  groupAction = vi.fn().mockResolvedValue(undefined),
  labelAction = vi.fn().mockResolvedValue(undefined),
  models = [model],
  onActivate = vi.fn(),
) {
  const container = getContainer();
  container.addSingleton(IGroupApi, {
    getGroups: vi.fn().mockResolvedValue([group]),
    addModelsToGroup: groupAction,
  });
  container.addSingleton(ILabelApi, { addLabelToModels: labelAction });
  sidebarState.labels = [
    {
      meta: label,
      children: [],
      effectiveLabels: [],
      hasParent: false,
      modelCount: 0,
      groupCount: 0,
      selfModelCount: 0,
      selfGroupCount: 0,
    },
  ];
  render(DragFixture, { models, onActivate });
  return { groupAction, labelAction };
}

async function startDrag() {
  const source = screen.getByTestId("source");
  await fireEvent.mouseDown(source, { button: 0, clientX: 0, clientY: 0 });
  await fireEvent.mouseMove(source, { clientX: 20, clientY: 0 });
  expect(dragState.dragging).toBe(true);
}

it("moves the selected models to a group when released inside the source grid", async () => {
  const { groupAction } = setup();
  await startDrag();
  await fireEvent.mouseUp(screen.getByTestId("group"));
  await waitFor(() =>
    expect(groupAction).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ id: 7 }),
      expect.arrayContaining([expect.objectContaining({ id: 1 })]),
    ),
  );
  expect(dragState.dragging).toBe(false);
});

it("adds a label when released over a nested SVG path", async () => {
  const { labelAction } = setup();
  await startDrag();
  await fireEvent.mouseUp(screen.getByTestId("label-path"));
  await waitFor(() =>
    expect(labelAction).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ id: 9 }),
      expect.arrayContaining([expect.objectContaining({ id: 1 })]),
    ),
  );
  expect(dragState.dragging).toBe(false);
});

it("cancels dragging with Escape without applying a destination", async () => {
  const { groupAction, labelAction } = setup();
  await startDrag();
  await fireEvent.keyDown(window, { key: "Escape" });
  expect(dragState.dragging).toBe(false);
  await fireEvent.mouseUp(screen.getByTestId("group"));
  expect(groupAction).not.toHaveBeenCalled();
  expect(labelAction).not.toHaveBeenCalled();
});

it("clears an active drag when its root unmounts", async () => {
  setup();
  await startDrag();
  cleanup();
  expect(dragState.dragging).toBe(false);
  expect(dragState.dragging_models).toEqual([]);
});

it("cancels dragging when the pointer leaves the window", async () => {
  const { groupAction, labelAction } = setup();
  await startDrag();
  await fireEvent.mouseOut(document.documentElement, { relatedTarget: null });
  expect(dragState.dragging).toBe(false);
  await fireEvent.mouseUp(screen.getByTestId("group"));
  expect(groupAction).not.toHaveBeenCalled();
  expect(labelAction).not.toHaveBeenCalled();
});

it("clears drag state when the group mutation fails", async () => {
  const groupAction = vi.fn().mockRejectedValue(new Error("offline"));
  setup(groupAction);
  await startDrag();
  await fireEvent.mouseUp(screen.getByTestId("group"));
  await waitFor(() => expect(groupAction).toHaveBeenCalledTimes(1));
  expect(dragState.dragging).toBe(false);
});

it("does not move a model already in the target group", async () => {
  const groupAction = vi.fn().mockResolvedValue(undefined);
  setup(groupAction, vi.fn(), [{ ...model, group: group.meta }]);
  await startDrag();
  await fireEvent.mouseUp(screen.getByTestId("group"));
  expect(groupAction).not.toHaveBeenCalled();
  expect(dragState.dragging).toBe(false);
});

it("does not modify groups or labels in a read-only view", async () => {
  configurationMeta.applicationReadOnly = true;
  const { groupAction, labelAction } = setup();
  await fireEvent.mouseDown(screen.getByTestId("source"), {
    button: 0,
    clientX: 0,
    clientY: 0,
  });
  await fireEvent.mouseMove(screen.getByTestId("source"), { clientX: 20 });
  await fireEvent.mouseUp(screen.getByTestId("group"));
  expect(groupAction).not.toHaveBeenCalled();
  expect(labelAction).not.toHaveBeenCalled();
});

it("does not add a label that the model already has", async () => {
  const labelAction = vi.fn().mockResolvedValue(undefined);
  setup(vi.fn(), labelAction, [{ ...model, labels: [label] }]);
  await startDrag();
  await fireEvent.mouseUp(screen.getByTestId("label-path"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(labelAction).not.toHaveBeenCalled();
});

it("does not activate a source when the release produces a click after dragging", async () => {
  const onActivate = vi.fn();
  setup(vi.fn(), vi.fn(), [model], onActivate);
  await startDrag();
  const source = screen.getByTestId("source");
  await fireEvent.mouseUp(source);
  await fireEvent.click(source);
  expect(onActivate).not.toHaveBeenCalled();
});

it("does not start a drag from empty space around collection items", async () => {
  setup();
  const blank = screen.getByTestId("source").parentElement!;
  await fireEvent.mouseDown(blank, { button: 0, clientX: 0, clientY: 0 });
  await fireEvent.mouseMove(blank, { clientX: 20, clientY: 0 });
  expect(dragState.dragging).toBe(false);
});

it("highlights a valid group and displays the move intent", async () => {
  setup();
  await startDrag();
  const target = screen.getByTestId("group");
  await fireEvent.mouseMove(target, { clientX: 25, clientY: 25 });
  expect(target.classList.contains("ring-2")).toBe(true);
  expect(screen.queryByText(/move to group/i)).not.toBeNull();
});

it("highlights a valid label and displays the add-label intent", async () => {
  setup();
  await startDrag();
  const path = screen.getByTestId("label-path");
  const target = path.closest("[data-drag-type]")!;
  await fireEvent.mouseMove(path, { clientX: 25, clientY: 25 });
  expect(target.classList.contains("ring-2")).toBe(true);
  expect(screen.queryByText(/add label/i)).not.toBeNull();
});

it("does not highlight an already assigned group", async () => {
  setup(vi.fn(), vi.fn(), [{ ...model, group: group.meta }]);
  await startDrag();
  const target = screen.getByTestId("group");
  await fireEvent.mouseMove(target, { clientX: 25, clientY: 25 });
  expect(target.classList.contains("ring-2")).toBe(false);
});

it("does not highlight targets in a read-only view", async () => {
  configurationMeta.applicationReadOnly = true;
  setup();
  await startDrag();
  const target = screen.getByTestId("group");
  await fireEvent.mouseMove(target, { clientX: 25, clientY: 25 });
  expect(target.classList.contains("ring-2")).toBe(false);
});
