import { describe, it, expect, vi } from "vitest";
import type { IBlobApi } from "../shared/blob_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { MobileRemoteSlicerApi } from "./remote_slicer";

describe("MobileRemoteSlicerApi", () => {
  it("POSTs modelId and settings to /slicer/slice", async () => {
    const request = {
      request: vi.fn().mockResolvedValue({
        success: true,
        outputModelId: 42,
      }),
    };
    const api = new MobileRemoteSlicerApi(
      {} as IBlobApi,
      request as unknown as IServerRequestApi,
    );
    const settings = {
      layerHeight: 0.2,
      infill: 20,
      supports: "none" as const,
      material: "PLA",
    };

    await api.sliceOnServer(7, settings);

    expect(request.request).toHaveBeenCalledWith(
      "/slicer/slice",
      HttpMethod.POST,
      { modelId: 7, settings },
    );
  });
});
