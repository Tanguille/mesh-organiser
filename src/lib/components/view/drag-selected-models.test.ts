// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { Model } from "#lib/api/shared/model_api";
import {
  state as dragState,
  stopDragging,
} from "#lib/drag-selected-models.svelte";
import DragSelectedModels from "./drag-selected-models.svelte";

const children = createRawSnippet(() => ({
  render: () => "<span>Part</span>",
}));

describe("DragSelectedModels", () => {
  afterEach(() => {
    stopDragging();
  });

  it("starts dragging on a diagonal move whose axis deltas cancel out", async () => {
    const { container } = render(DragSelectedModels, {
      children,
      models: [{ id: 1 }] as unknown as Model[],
    });
    const area = container.firstElementChild!;

    await fireEvent.mouseDown(area, { button: 0, clientX: 100, clientY: 100 });
    await fireEvent.mouseMove(area, { clientX: 120, clientY: 80 });

    expect(dragState.dragging).toBe(true);
  });
});
