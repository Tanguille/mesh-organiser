import { afterEach, describe, expect, it, vi } from "vitest";
import type { Blob } from "../shared/blob_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { WebShareBlobApi } from "./blob";

const share: Share = {
  id: "share-1",
  createdAt: new Date("2024-05-06T07:08:09Z"),
  shareName: "Benchy collection",
  userName: "alice",
  modelIds: [1, 2],
};

function blobs(...sha256s: string[]): Blob[] {
  return sha256s.map((sha256) => ({ sha256 }) as Blob);
}

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

    const url = await new WebShareBlobApi(
      requestApi,
      share,
    ).getBlobsDownloadUrl(blobs("sha-a", "sha-b"));

    expect(request.mock.calls).toEqual([
      ["/shares/share-1/blobs/download", HttpMethod.POST, ["sha-a", "sha-b"]],
    ]);
    expect(url).toBe(
      "https://mesh.example/api/v1/blobs/download/meshorganiser_export_zip_action_abc",
    );
  });
});
