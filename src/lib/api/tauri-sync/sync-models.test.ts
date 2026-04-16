import { describe, expect, it } from "vitest";
import {
  ImportStatus,
  type ImportState,
} from "../shared/tauri_import_api";
import { localModelIdFromSingleFileImport } from "./sync-models";

function importState(partial: Partial<ImportState>): ImportState {
  return {
    imported_models: [],
    imported_models_count: 0,
    model_count: 0,
    finished_thumbnails_count: 0,
    status: ImportStatus.Finished,
    origin_url: "",
    failure_reason: null,
    recursive: false,
    delete_after_import: false,
    ...partial,
  };
}

describe("localModelIdFromSingleFileImport", () => {
  it("returns null when status is Failure", () => {
    expect(
      localModelIdFromSingleFileImport(
        importState({ status: ImportStatus.Failure }),
      ),
    ).toBeNull();
  });

  it("returns null when imported_models is empty", () => {
    expect(
      localModelIdFromSingleFileImport(importState({ imported_models: [] })),
    ).toBeNull();
  });

  it("returns null when model_ids is empty", () => {
    expect(
      localModelIdFromSingleFileImport(
        importState({
          imported_models: [
            { group_id: null, group_name: null, model_ids: [] },
          ],
        }),
      ),
    ).toBeNull();
  });

  it("returns the first model id when present", () => {
    expect(
      localModelIdFromSingleFileImport(
        importState({
          imported_models: [
            { group_id: null, group_name: null, model_ids: [42, 99] },
          ],
        }),
      ),
    ).toBe(42);
  });
});
