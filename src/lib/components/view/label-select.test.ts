// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { LabelMeta } from "$lib/api/shared/label_api";
import LabelSelect from "./label-select.svelte";

function label(id: number, name: string): LabelMeta {
  return {
    id,
    name,
    color: "#ff0000",
    lastModified: new Date(0),
    uniqueGlobalId: `label-${id}`,
  };
}

const available = [
  label(1, "Tools"),
  label(2, "Toys"),
  label(3, "Spare parts"),
];

describe("LabelSelect", () => {
  beforeAll(() => {
    // jsdom lacks pointer capture and scrollIntoView, which bits-ui Select calls.
    Element.prototype.hasPointerCapture ??= () => false;
    Element.prototype.releasePointerCapture ??= () => {};
    Element.prototype.scrollIntoView ??= () => {};
  });

  it("shows the default placeholder when nothing is selected", () => {
    render(LabelSelect, { value: [], availableLabels: available });

    expect(screen.getByText("Select some labels")).toBeTruthy();
  });

  it("shows a custom placeholder when nothing is selected", () => {
    render(LabelSelect, {
      value: [],
      availableLabels: available,
      placeholder: "Pick labels",
    });

    expect(screen.getByText("Pick labels")).toBeTruthy();
  });

  it("renders a badge per selected label instead of the placeholder", () => {
    render(LabelSelect, {
      value: [available[0], available[2]],
      availableLabels: available,
    });

    expect(screen.queryByText("Select some labels")).toBeNull();
    expect(screen.getByText("Tools")).toBeTruthy();
    expect(screen.getByText("Spare parts")).toBeTruthy();
    expect(screen.queryByText("Toys")).toBeNull();
  });

  it("lists every available label when opened and calls onchange on selection", async () => {
    const onchange = vi.fn();
    render(LabelSelect, { value: [], availableLabels: available, onchange });

    await fireEvent.pointerDown(
      screen.getByRole("button", { name: "Select some labels" }),
      {
        button: 0,
        pointerType: "mouse",
      },
    );

    const options = await screen.findAllByRole("option");
    expect(options.map((option) => option.textContent?.trim())).toEqual([
      "Tools",
      "Toys",
      "Spare parts",
    ]);

    await fireEvent.pointerUp(options[1], { button: 0, pointerType: "mouse" });

    expect(onchange).toHaveBeenCalled();
  });
});
