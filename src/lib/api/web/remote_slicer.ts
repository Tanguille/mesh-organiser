import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { IBlobApi } from "../shared/blob_api";
import type { Model } from "../shared/model_api";
import {
  DefaultSlicerApi,
  type ISlicerApi,
  type SliceServerResponse,
  type SlicingSettings,
} from "../shared/slicer_api";

export class RemoteHttpSlicerApi {
  private readonly request: IServerRequestApi;

  constructor(request: IServerRequestApi) {
    this.request = request;
  }

  sliceOnServer(
    modelId: number,
    settings: SlicingSettings,
  ): Promise<SliceServerResponse> {
    return this.request.request<SliceServerResponse>(
      "/slicer/slice",
      HttpMethod.POST,
      { modelId, settings },
    );
  }
}

/**
 * Desktop-style slicer hand-off plus optional server-side slice for Tauri mobile remote.
 */
export class MobileRemoteSlicerApi implements ISlicerApi {
  private readonly local: DefaultSlicerApi;
  private readonly remote: RemoteHttpSlicerApi;

  constructor(blob: IBlobApi, request: IServerRequestApi) {
    this.local = new DefaultSlicerApi(blob);
    this.remote = new RemoteHttpSlicerApi(request);
  }

  async openInSlicer(models: Model[]): Promise<void> {
    return this.local.openInSlicer(models);
  }

  async availableSlicers() {
    return this.local.availableSlicers();
  }

  sliceOnServer(
    modelId: number,
    settings: SlicingSettings,
  ): Promise<SliceServerResponse> {
    return this.remote.sliceOnServer(modelId, settings);
  }
}
