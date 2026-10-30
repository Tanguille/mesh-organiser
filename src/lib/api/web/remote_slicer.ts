import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { IBlobApi } from "../shared/blob_api";
import {
  DefaultSlicerApi,
  type SliceServerResponse,
} from "../shared/slicer_api";

/**
 * Desktop-style slicer hand-off plus server-side slicing for Tauri mobile remote.
 */
export class MobileRemoteSlicerApi extends DefaultSlicerApi {
  private readonly request: IServerRequestApi;

  constructor(blob: IBlobApi, request: IServerRequestApi) {
    super(blob);
    this.request = request;
  }

  sliceOnServer(modelId: number): Promise<SliceServerResponse> {
    return this.request.request<SliceServerResponse>(
      "/slicer/slice",
      HttpMethod.POST,
      { modelId },
    );
  }
}
