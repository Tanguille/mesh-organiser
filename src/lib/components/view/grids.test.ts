// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { IModelStreamManager } from "$lib/api/shared/model_api";
import type { IGroupStreamManager } from "$lib/api/shared/group_api";
import ModelGrid from "./model-grid.svelte";
import GroupGrid from "./group-grid.svelte";

// jsdom has no matchMedia; svelte/reactivity touches it at import time and
// IsMobile / IsSplitGridSize (MediaQuery) read it. matches=false: desktop, both panes render.
vi.hoisted(() => {
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
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));

// Empty streams: nothing loaded, so the right pane shows the empty state.
function emptyStream() {
  return {
    setSearchText: vi.fn(),
    setOrderBy: vi.fn(),
    setFileTypes: vi.fn(),
    fetch: vi.fn().mockResolvedValue([]),
    getAll: vi.fn().mockResolvedValue([]),
  };
}

const grids = [
  {
    name: "ModelGrid",
    emptyText: "No model selected",
    mount: (stream: ReturnType<typeof emptyStream>) =>
      render(ModelGrid, { modelStream: stream as IModelStreamManager }),
  },
  {
    name: "GroupGrid",
    emptyText: "No group selected",
    mount: (stream: ReturnType<typeof emptyStream>) =>
      render(GroupGrid, { groupStream: stream as IGroupStreamManager }),
  },
];

describe.each(grids)("$name", ({ emptyText, mount }) => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the dashed empty state when nothing is selected", async () => {
    const stream = emptyStream();
    mount(stream);

    const empty = screen.getByText(emptyText);
    expect(empty.parentElement?.className).toContain("border-dashed");
    await waitFor(() => expect(stream.fetch).toHaveBeenCalled());
  });

  it("applies search text only after the 200ms debounce", async () => {
    const stream = emptyStream();
    mount(stream);
    await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(1));

    vi.useFakeTimers();
    const input = screen.getByPlaceholderText("Search");
    await fireEvent.input(input, { target: { value: "bo" } });
    await fireEvent.input(input, { target: { value: "  bolt  " } });

    vi.advanceTimersByTime(199);
    expect(stream.setSearchText).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    // Debounced: only the last value, trimmed.
    expect(stream.setSearchText).toHaveBeenCalledExactlyOnceWith("bolt");
    vi.useRealTimers();
    await waitFor(() => expect(stream.fetch).toHaveBeenCalledTimes(2));
  });

  it("clears the search filter (null) for whitespace-only input", async () => {
    const stream = emptyStream();
    mount(stream);
    await waitFor(() => expect(stream.fetch).toHaveBeenCalled());

    vi.useFakeTimers();
    await fireEvent.input(screen.getByPlaceholderText("Search"), {
      target: { value: "   " },
    });
    vi.advanceTimersByTime(200);

    expect(stream.setSearchText).toHaveBeenCalledExactlyOnceWith(null);
  });
});
