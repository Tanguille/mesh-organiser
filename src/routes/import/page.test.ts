// @vitest-environment jsdom
/**
 * Behaviour test for the import page's desktop buttons: a file or folder dialog
 * that rejects must re-enable both buttons. The error itself still escapes to
 * the layout's unhandled-rejection toast.
 */

import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IGroupApi } from "#lib/api/shared/group_api";
import { ITauriImportApi } from "#lib/api/shared/tauri_import_api";
import Page from "./+page.svelte";

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
// Threlte needs WebGL, which jsdom lacks; these tests never open a model preview.
vi.mock("#lib/components/view/three-d-canvas.svelte", () => ({
  default: () => {},
}));

function importButton(name: string) {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

describe("import +page", () => {
  afterEach(() => {
    resetContainer();
    vi.restoreAllMocks();
  });

  it.each([
    ["Import File", "openFilesForImporting"],
    ["Import Folder", "openFolderForImporting"],
  ] as const)(
    "re-enables the import buttons when %s rejects",
    async (buttonName, method) => {
      vi.spyOn(console, "log").mockImplementation(() => {});
      const failure = new Error("dialog failed");
      const tauriImportApi = {
        openFilesForImporting: vi.fn().mockResolvedValue(undefined),
        openFolderForImporting: vi.fn().mockResolvedValue(undefined),
      };
      tauriImportApi[method].mockRejectedValueOnce(failure);
      const container = getContainer();
      container.addSingleton(IGroupApi, {});
      container.addSingleton(ITauriImportApi, tauriImportApi);

      // The click handler's rejection is unhandled by design, so swap
      // vitest's listener for one that records it.
      const vitestListeners = process.listeners("unhandledRejection");
      const unhandledRejection = vi.fn();
      process.removeAllListeners("unhandledRejection");
      process.on("unhandledRejection", unhandledRejection);

      try {
        render(Page);
        await fireEvent.click(importButton(buttonName));
        await waitFor(() =>
          expect(unhandledRejection).toHaveBeenCalledWith(
            failure,
            expect.anything(),
          ),
        );

        expect(importButton("Import File").disabled).toBe(false);
        expect(importButton("Import Folder").disabled).toBe(false);
      } finally {
        process.removeAllListeners("unhandledRejection");
        vitestListeners.forEach((listener) =>
          process.on("unhandledRejection", listener),
        );
      }
    },
  );
});
