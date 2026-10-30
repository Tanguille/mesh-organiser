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
import { IThreemfApi, type ThreemfMetadata } from "#lib/api/shared/threemf_api";
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

function threemfModel(id: number, name = `Plate ${id}`) {
  return createModelInstance(
    id,
    name,
    createBlobInstance(id, `hash-${id}`, "3mf", 1024, timestamp),
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

function metadata(nozzleDiameter: number): ThreemfMetadata {
  return {
    nozzle_diameter: nozzleDiameter,
    layer_height: null,
    material_type: null,
    supports_enabled: null,
  };
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

  it("keeps the current model's 3MF metadata when the previous model's arrives late", async () => {
    provideApis();
    const pending = new Map<number, (metadata: ThreemfMetadata) => void>();
    getContainer().addSingleton(IThreemfApi, {
      getThreemfMetadata: vi.fn(
        (model: { id: number }) =>
          new Promise<ThreemfMetadata>((resolve) =>
            pending.set(model.id, resolve),
          ),
      ),
    });
    const { rerender, findByText, queryByText } = render(ModelCard, {
      model: threemfModel(1),
    });
    await rerender({ model: threemfModel(2) });

    pending.get(2)!(metadata(0.4));
    expect(await findByText("0.4 mm")).toBeTruthy();
    pending.get(1)!(metadata(0.6));
    // Lets the late response's continuation run and render.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(queryByText("0.6 mm")).toBeNull();
    expect(queryByText("0.4 mm")).toBeTruthy();
  });

  it("does not refetch the 3MF metadata when only the model's name changes", async () => {
    provideApis();
    const getThreemfMetadata = vi.fn().mockResolvedValue(metadata(0.4));
    getContainer().addSingleton(IThreemfApi, { getThreemfMetadata });
    const { rerender, findByText } = render(ModelCard, {
      model: threemfModel(1),
    });
    expect(await findByText("0.4 mm")).toBeTruthy();

    await rerender({ model: threemfModel(1, "Renamed plate") });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(getThreemfMetadata).toHaveBeenCalledTimes(1);
    expect(await findByText("0.4 mm")).toBeTruthy();
  });

  it("logs a failed 3MF metadata fetch and leaves the metadata cleared", async () => {
    provideApis();
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    const failure = new Error("unreadable 3mf");
    getContainer().addSingleton(IThreemfApi, {
      getThreemfMetadata: vi
        .fn()
        .mockResolvedValueOnce(metadata(0.4))
        .mockRejectedValueOnce(failure),
    });
    const { rerender, findByText, queryByText } = render(ModelCard, {
      model: threemfModel(1),
    });
    expect(await findByText("0.4 mm")).toBeTruthy();

    await rerender({ model: threemfModel(2) });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(queryByText("0.4 mm")).toBeNull();
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining("3MF metadata"),
      failure,
    );
    consoleError.mockRestore();
  });
});
