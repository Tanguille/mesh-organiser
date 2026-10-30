import { afterEach, describe, expect, it, vi } from "vitest";
import type { Blob } from "../shared/blob_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { WebShareBlobApi } from "./blob";

describe("WebShareBlobApi", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("getBlobsDownloadUrl zips the blobs through the share and links the zip for the slicer", async () => {
    vi.stubGlobal("document", { location: { origin: "https://mesh.example" } });
    const request = vi
      .fn()
      .mockResolvedValue("meshorganiser_export_zip_action_abc");
    const requestApi = { request } as unknown as IServerRequestApi;
    const api = new WebShareBlobApi(requestApi, { id: "share-1" } as Share);

    const url = await api.getBlobsDownloadUrl([
      { sha256: "sha-a" },
      { sha256: "sha-b" },
    ] as Blob[]);

    expect(request.mock.calls).toEqual([
      ["/shares/share-1/blobs/download", HttpMethod.POST, ["sha-a", "sha-b"]],
    ]);
    expect(url).toBe(
      "https://mesh.example/api/v1/blobs/download/meshorganiser_export_zip_action_abc",
    );
  });
});
