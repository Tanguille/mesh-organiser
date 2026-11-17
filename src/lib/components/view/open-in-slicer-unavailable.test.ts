// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { createBlobInstance } from "#lib/api/shared/blob_api";
import { createModelInstance } from "#lib/api/shared/model_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import OpenInSlicerButton from "./open-in-slicer-button.svelte";

vi.hoisted(() => {
  window.matchMedia ??= (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
});
afterEach(() => {
  cleanup();
  resetContainer();
});
function model() {
  const timestamp = "2026-10-24";
  return createModelInstance(
    1,
    "Part",
    createBlobInstance(1, "hash", "part.stl", 1024, timestamp),
    null,
    null,
    timestamp,
    timestamp,
    null,
    [],
    [],
    "model-1",
  );
}
it("omits the slicer action when its platform API is unavailable", () => {
  render(OpenInSlicerButton, { models: model() });
  expect(screen.queryByRole("button", { name: /open in slicer/i })).toBeNull();
});
it("offers the slicer action when its platform API is available", () => {
  getContainer().addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
  render(OpenInSlicerButton, { models: model() });
  expect(screen.getByRole("button", { name: /open in slicer/i })).toBeTruthy();
});
