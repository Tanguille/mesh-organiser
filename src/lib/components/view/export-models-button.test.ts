// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "$lib/api/dependency_injection";
import { ILocalApi } from "$lib/api/shared/local_api";
import { IDownloadApi } from "$lib/api/shared/download_api";
import type { Model } from "$lib/api/shared/model_api";
import ExportModelsButton from "./export-models-button.svelte";

const models = [{ id: 1 }, { id: 2 }] as unknown as Model[];

function provideLocalApi() {
  const openInFolder = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(ILocalApi, { openInFolder });

  return openInFolder;
}

function provideDownloadApi() {
  const downloadApi = {
    downloadModel: vi.fn().mockResolvedValue(undefined),
    downloadModelsAsZip: vi.fn().mockResolvedValue(undefined),
  };
  getContainer().addSingleton(IDownloadApi, downloadApi);

  return downloadApi;
}

async function openMenu() {
  // The chevron trigger is the only other button next to "Open in folder".
  const trigger = screen
    .getAllByRole("button")
    .find((button) => !button.textContent?.includes("Open in folder"))!;
  await fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" });
}

describe("ExportModelsButton", () => {
  afterEach(() => {
    resetContainer();
  });

  describe("with a local API", () => {
    it("renders the 'Open in folder' button and no download button", () => {
      provideLocalApi();

      render(ExportModelsButton, { models, class: "" });

      expect(
        screen.getByRole("button", { name: "Open in folder" }),
      ).toBeTruthy();
      expect(screen.queryByText(/Download/)).toBeNull();
    });

    it("'Open in folder' calls openInFolder(models, false)", async () => {
      const openInFolder = provideLocalApi();
      render(ExportModelsButton, { models, class: "" });

      await fireEvent.click(
        screen.getByRole("button", { name: "Open in folder" }),
      );

      expect(openInFolder).toHaveBeenCalledWith(models, false);
    });

    it("menu item 'Export as individual models' calls openInFolder(models, false)", async () => {
      const openInFolder = provideLocalApi();
      render(ExportModelsButton, { models, class: "" });

      await openMenu();
      await fireEvent.click(
        await screen.findByText("Export as individual models"),
      );

      expect(openInFolder).toHaveBeenCalledWith(models, false);
    });

    it("menu item 'Export as .zip file' calls openInFolder(models, true)", async () => {
      const openInFolder = provideLocalApi();
      render(ExportModelsButton, { models, class: "" });

      await openMenu();
      await fireEvent.click(await screen.findByText("Export as .zip file"));

      expect(openInFolder).toHaveBeenCalledWith(models, true);
    });

    it("prefers the local API when a download API is also available", () => {
      provideLocalApi();
      provideDownloadApi();

      render(ExportModelsButton, { models, class: "" });

      expect(
        screen.getByRole("button", { name: "Open in folder" }),
      ).toBeTruthy();
      expect(screen.queryByText(/Download/)).toBeNull();
    });
  });

  // Today the download fallback lives in resource-grid.svelte / multi-model.svelte
  // (`{:else if downloadApi}` -> "Download model(s)" AsyncButton -> downloadModels()),
  // and ExportModelsButton throws because it `require`s ILocalApi. Once the fallback
  // is folded into ExportModelsButton, these flip: change `it.fails` to `it`.
  describe("with only a download API (fallback, not yet in this component)", () => {
    it.fails("renders a download button and no 'Open in folder' button", () => {
      provideDownloadApi();

      render(ExportModelsButton, { models, class: "" });

      expect(screen.getByRole("button", { name: /Download/ })).toBeTruthy();
      expect(
        screen.queryByRole("button", { name: "Open in folder" }),
      ).toBeNull();
    });

    it.fails(
      "download button calls downloadModelsAsZip for multiple models",
      async () => {
        const downloadApi = provideDownloadApi();
        render(ExportModelsButton, { models, class: "" });

        await fireEvent.click(screen.getByRole("button", { name: /Download/ }));

        expect(downloadApi.downloadModelsAsZip).toHaveBeenCalledWith(models);
      },
    );

    it.fails(
      "download button calls downloadModel for a single model",
      async () => {
        const downloadApi = provideDownloadApi();
        render(ExportModelsButton, { models: [models[0]], class: "" });

        await fireEvent.click(screen.getByRole("button", { name: /Download/ }));

        expect(downloadApi.downloadModel).toHaveBeenCalledWith(models[0]);
      },
    );

    it.fails("renders nothing without a local or download API", () => {
      const { container } = render(ExportModelsButton, { models, class: "" });

      expect(container.querySelector("button")).toBeNull();
    });
  });
});
