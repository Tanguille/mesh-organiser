// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import { IGroupApi } from "#lib/api/shared/group_api";
import { ILabelApi } from "#lib/api/shared/label_api";
import { IModelApi, createModelInstance } from "#lib/api/shared/model_api";
import { configurationDefault } from "#lib/api/shared/settings_api";
import { ISlicerApi } from "#lib/api/shared/slicer_api";
import { configuration } from "#lib/configuration.svelte";
import ModelCard from "./model.svelte";

vi.hoisted(() => {
  // jsdom has no matchMedia; svelte/reactivity touches it at import time.
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
// Threlte needs WebGL, which jsdom lacks; the test only cares whether the preview toggle stays on.
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

const timestamp = "2026-10-29T12:00:00Z";

function provideApis() {
  const container = getContainer();
  container.addSingleton(IModelApi, {});
  container.addSingleton(IGroupApi, {});
  container.addSingleton(ILabelApi, {});
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue("data:image/png;base64,"),
  });
  container.addSingleton(ISlicerApi, {
    availableSlicers: vi.fn().mockResolvedValue([]),
  });
}

describe("Model", () => {
  afterEach(() => {
    resetContainer();
    Object.assign(configuration, configurationDefault());
  });

  it("keeps a manually enabled 3D preview when an unrelated setting changes", async () => {
    provideApis();
    // 100 MB is over the default STL preview limit, so the preview starts off.
    const model = createModelInstance(
      1,
      "Benchy",
      createBlobInstance(1, "hash-1", "stl", 100 * 1024 * 1024, timestamp),
      null,
      null,
      timestamp,
      timestamp,
      null,
      [],
      [],
      "model-1",
    );
    const { container } = render(ModelCard, { model });
    const previewToggle = container
      .querySelector(".lucide-box")!
      .closest("button")!;
    expect(previewToggle.getAttribute("aria-pressed")).toBe("false");

    await fireEvent.click(previewToggle);
    expect(previewToggle.getAttribute("aria-pressed")).toBe("true");

    configuration.collapse_sidebar = !configuration.collapse_sidebar;
    await tick();

    expect(previewToggle.getAttribute("aria-pressed")).toBe("true");
  });
});
