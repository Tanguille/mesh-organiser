// @vitest-environment jsdom
/**
 * Pins the import progress bar's value, which reads 0 rather than NaN while the
 * backend has not reported the model total yet.
 */

import { render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { importState, resetImportState } from "#lib/import.svelte";
import { ImportStatus } from "#lib/api/shared/tauri_import_api";
import ImportProgressIndicator from "./tauri-import-progress-indicator.svelte";

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));

function progressValue() {
  return screen.getByRole("progressbar").getAttribute("aria-valuenow");
}

describe("ImportProgressIndicator", () => {
  afterEach(() => {
    vi.spyOn(console, "log").mockImplementation(() => {});
    resetImportState();
    vi.restoreAllMocks();
  });

  it.each([ImportStatus.ProcessingModels, ImportStatus.ProcessingThumbnails])(
    "shows 0 progress for status %s before the model total arrives",
    (status) => {
      Object.assign(importState, { status, model_count: 0 });

      render(ImportProgressIndicator);

      expect(progressValue()).toBe("0");
    },
  );

  it.each([
    [ImportStatus.ProcessingModels, "imported_models_count"],
    [ImportStatus.ProcessingThumbnails, "finished_thumbnails_count"],
  ] as const)(
    "shows the rounded share of models done for status %s",
    (status, doneField) => {
      Object.assign(importState, { status, model_count: 3, [doneField]: 1 });

      render(ImportProgressIndicator);

      expect(progressValue()).toBe("33");
    },
  );
});
