import type { Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { IShareApi, Share } from "../shared/share_api";
import { parseRawShare, type RawShare } from "../web/share";

export class LimitedWebShareApi implements IShareApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async getShares(): Promise<Share[]> {
    return [];
  }

  async getShare(shareId: string): Promise<Share> {
    const rawShare = await this.requestApi.request<RawShare>(
      `/shares/${shareId}`,
      HttpMethod.GET,
    );
    return parseRawShare(rawShare);
  }

  async getShareLink(_share: Share): Promise<string> {
    throw new Error("Method not implemented.");
  }

  async createShare(_shareName: string): Promise<Share> {
    throw new Error("Method not implemented.");
  }

  async addModelsToShare(_share: Share, _models: Model[]): Promise<void> {}

  async setModelsOnShare(_share: Share, _models: Model[]): Promise<void> {}

  async editShare(_share: Share): Promise<void> {}

  async deleteShare(_share: Share): Promise<void> {}
}
