// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import AddLabelPopover from "./add-label-popover.svelte";

const children = createRawSnippet(() => ({ render: () => "<span>Add</span>" }));

describe("AddLabelPopover", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("submits a six-digit hex colour when Math.random returns a small value", async () => {
    // 0.0001 * 0xffffff = 1677 = 0x68d, three hex digits without padding.
    vi.spyOn(Math, "random").mockReturnValue(0.0001);
    const onsubmit = vi.fn().mockResolvedValue(undefined);
    render(AddLabelPopover, { children, onsubmit });

    await fireEvent.click(screen.getByText("Add"));
    await fireEvent.click(
      await screen.findByRole("button", { name: "Random" }),
    );
    // The colour input sanitises invalid values to #000000, so assert on the submitted state instead.
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(onsubmit).toHaveBeenCalledExactlyOnceWith(
      "New label",
      expect.stringMatching(/^#[0-9a-f]{6}$/),
    );
  });
});
