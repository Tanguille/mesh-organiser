/**
 * Characterization tests for the web share APIs (WebShareApi and the
 * read-only LimitedWebShareApi used on public share pages) and parseRawShare.
 * Request method, url and body are the contract with the web server.
 */

import { describe, expect, it, vi } from "vitest";
import type { Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { LimitedWebShareApi } from "../web_share/share";
import { parseRawShare, WebShareApi, type RawShare } from "./share";

const rawShare: RawShare = {
  id: "share-1",
  share_name: "Benchy collection",
  created_at: "2024-05-06T07:08:09Z",
  user_name: "alice",
  model_ids: [3, 1, 2],
};

const share: Share = {
  id: "share-1",
  createdAt: new Date("2024-05-06T07:08:09Z"),
  shareName: "Benchy collection",
  userName: "alice",
  modelIds: [3, 1, 2],
};

function mockRequestApi(response?: unknown) {
  const request = vi.fn().mockResolvedValue(response);
  const requestApi = {
    baseUrl: "https://mesh.example",
    request,
  } as unknown as IServerRequestApi;

  return { request, requestApi };
}

function models(...ids: number[]): Model[] {
  return ids.map((id) => ({ id }) as Model);
}

describe("parseRawShare", () => {
  it("maps every raw field onto the share shape", () => {
    expect(parseRawShare(rawShare)).toStrictEqual(share);
  });
});

describe("WebShareApi", () => {
  it("getShares requests GET /shares and parses every share", async () => {
    const { request, requestApi } = mockRequestApi([
      rawShare,
      { ...rawShare, id: "share-2", model_ids: [] },
    ]);

    const shares = await new WebShareApi(requestApi).getShares();

    expect(request.mock.calls).toEqual([["/shares", HttpMethod.GET]]);
    expect(shares).toStrictEqual([
      share,
      { ...share, id: "share-2", modelIds: [] },
    ]);
  });

  it("getShare resolves the matching share from the share list", async () => {
    const { request, requestApi } = mockRequestApi([
      { ...rawShare, id: "other" },
      rawShare,
    ]);

    const found = await new WebShareApi(requestApi).getShare("share-1");

    expect(found).toStrictEqual(share);
    expect(request.mock.calls).toEqual([["/shares", HttpMethod.GET]]);
  });

  it("getShare rejects with the plain string 'Share not found' when no id matches", async () => {
    const { requestApi } = mockRequestApi([rawShare]);

    await expect(new WebShareApi(requestApi).getShare("missing")).rejects.toBe(
      "Share not found",
    );
  });

  it("setModelsOnShare PUTs the model ids in the given order", async () => {
    const { request, requestApi } = mockRequestApi();

    await new WebShareApi(requestApi).setModelsOnShare(share, models(9, 4, 7));

    expect(request.mock.calls).toEqual([
      ["/shares/share-1/models", HttpMethod.PUT, { model_ids: [9, 4, 7] }],
    ]);
  });

  it("createShare, editShare and deleteShare send the expected requests", async () => {
    const { request, requestApi } = mockRequestApi(rawShare);
    const api = new WebShareApi(requestApi);

    const created = await api.createShare("Benchy collection");
    await api.editShare({ ...share, shareName: "Renamed" });
    await api.deleteShare(share);

    expect(created).toStrictEqual(share);
    expect(request.mock.calls).toEqual([
      ["/shares", HttpMethod.POST, { share_name: "Benchy collection" }],
      ["/shares/share-1", HttpMethod.PUT, { share_name: "Renamed" }],
      ["/shares/share-1", HttpMethod.DELETE],
    ]);
  });

  it("getShareLink builds the public share url from the request base url", async () => {
    const { request, requestApi } = mockRequestApi();

    await expect(new WebShareApi(requestApi).getShareLink(share)).resolves.toBe(
      "https://mesh.example/share/share-1",
    );
    expect(request).not.toHaveBeenCalled();
  });
});

describe("LimitedWebShareApi", () => {
  it("getShares returns an empty list without a request", async () => {
    const { request, requestApi } = mockRequestApi();

    await expect(
      new LimitedWebShareApi(requestApi).getShares(),
    ).resolves.toEqual([]);
    expect(request).not.toHaveBeenCalled();
  });

  it("getShare requests GET /shares/{id} directly and parses the share", async () => {
    const { request, requestApi } = mockRequestApi(rawShare);

    const found = await new LimitedWebShareApi(requestApi).getShare("share-1");

    expect(found).toStrictEqual(share);
    expect(request.mock.calls).toEqual([["/shares/share-1", HttpMethod.GET]]);
  });

  it("getShareLink and createShare throw 'Method not implemented.'", async () => {
    const { requestApi } = mockRequestApi();
    const api = new LimitedWebShareApi(requestApi);

    await expect(api.getShareLink(share)).rejects.toThrow(
      "Method not implemented.",
    );
    await expect(api.createShare("name")).rejects.toThrow(
      "Method not implemented.",
    );
  });

  it("setModelsOnShare, editShare and deleteShare are silent no-ops", async () => {
    const { request, requestApi } = mockRequestApi();
    const api = new LimitedWebShareApi(requestApi);

    await expect(
      api.setModelsOnShare(share, models(1)),
    ).resolves.toBeUndefined();
    await expect(api.editShare(share)).resolves.toBeUndefined();
    await expect(api.deleteShare(share)).resolves.toBeUndefined();
    expect(request).not.toHaveBeenCalled();
  });
});
