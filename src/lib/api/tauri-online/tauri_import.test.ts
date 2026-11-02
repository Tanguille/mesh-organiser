/**
 * Pins that a failed upload in desktop online-account mode records the Failure
 * status and reason like the local import does, instead of rejecting and
 * leaving the import page on its progress spinner. The reason's formats are
 * tested with the local import.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { importState, resetImportState } from "#lib/import.svelte";
import type { IGroupApi } from "../shared/group_api";
import type { IServerRequestApi } from "../shared/server_request_api";
import { ImportStatus } from "../shared/tauri_import_api";
import { TauriWebImportApi } from "./tauri_import";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({
  page: { url: new URL("http://localhost/") },
}));

describe("TauriWebImportApi.startImportProcess", () => {
  beforeEach(() => {
    vi.spyOn(console, "log").mockImplementation(() => {});
  });

  afterEach(() => {
    resetImportState();
    vi.restoreAllMocks();
    invoke.mockReset();
  });

  it("records a failed upload's reason as an import failure", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    invoke.mockRejectedValueOnce(new Error("server unreachable"));
    const api = new TauriWebImportApi({} as IServerRequestApi, {} as IGroupApi);

    const result = await api.startImportProcess(["/a.stl"], {});

    expect(invoke).toHaveBeenCalledWith(
      "upload_models_to_remote_server",
      expect.anything(),
    );
    expect(result.status).toBe(ImportStatus.Failure);
    expect(result.failure_reason).toBe("server unreachable");
    expect(importState.status).toBe(ImportStatus.Failure);
    expect(importState.failure_reason).toBe("server unreachable");
  });
});
