import type { Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { IShareApi, Share } from "../shared/share_api";

export interface RawShare {
  id: string;
  share_name: string;
  created_at: string;
  user_name: string;
  model_ids: number[];
}

export function parseRawShare(raw: RawShare): Share {
  return {
    id: raw.id,
    createdAt: new Date(raw.created_at),
    shareName: raw.share_name,
    userName: raw.user_name,
    modelIds: raw.model_ids,
  };
}

export class WebShareApi implements IShareApi {
  private requestApi: IServerRequestApi;

  constructor(requestApi: IServerRequestApi) {
    this.requestApi = requestApi;
  }

  async getShares(): Promise<Share[]> {
    const shares = await this.requestApi.request<RawShare[]>(
      `/shares`,
      HttpMethod.GET,
    );
    return shares.map((rawShare) => parseRawShare(rawShare));
  }

  async getShare(shareId: string): Promise<Share> {
    // Lazy: scans the full share list instead of requesting GET /shares/{id}.
    const shares = await this.getShares();

    return (
      shares.find((share) => share.id === shareId) ??
      Promise.reject("Share not found")
    );
  }

  async createShare(shareName: string): Promise<Share> {
    const data = {
      share_name: shareName,
    };

    const rawShare = await this.requestApi.request<RawShare>(
      `/shares`,
      HttpMethod.POST,
      data,
    );
    return parseRawShare(rawShare);
  }

  async setModelsOnShare(share: Share, models: Model[]): Promise<void> {
    await this.requestApi.request<void>(
      `/shares/${share.id}/models`,
      HttpMethod.PUT,
      { model_ids: models.map((model) => model.id) },
    );
  }

  async editShare(share: Share): Promise<void> {
    const data = {
      share_name: share.shareName,
    };

    await this.requestApi.request<void>(
      `/shares/${share.id}`,
      HttpMethod.PUT,
      data,
    );
  }

  async deleteShare(share: Share): Promise<void> {
    await this.requestApi.request<void>(
      `/shares/${share.id}`,
      HttpMethod.DELETE,
    );
  }

  async getShareLink(share: Share): Promise<string> {
    return `${this.requestApi.baseUrl}/share/${share.id}`;
  }
}
