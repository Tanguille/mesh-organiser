import { afterEach, describe, expect, it, vi } from "vitest";
import type { IModelApi, Model } from "../shared/model_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import type { Share } from "../shared/share_api";
import { TauriProxyShareApi } from "./local-proxy-share";

const model = (id: number, uniqueGlobalId: string) =>
  ({ id, uniqueGlobalId }) as Model;

function catalog(models: Model[]) {
  return {
    getModels: vi.fn().mockResolvedValueOnce(models).mockResolvedValue([]),
  } as unknown as IModelApi;
}

afterEach(() => vi.restoreAllMocks());

describe("TauriProxyShareApi", () => {
  it("reads in local order, uses the first remote match and diagnoses missing or repeated members", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const request = vi.fn().mockResolvedValue([
      { id: "shared", model_ids: [1, 2, 1, 99] },
      { id: "empty", model_ids: [] },
    ]);
    const api = new TauriProxyShareApi(
      { request } as unknown as IServerRequestApi,
      catalog([model(1, "a"), model(2, "b"), model(1, "c")]),
      catalog([model(12, "b"), model(13, "c"), model(11, "a")]),
    );

    expect((await api.getShares()).map((s) => s.modelIds)).toEqual([
      [12, 11],
      [],
    ]);
    expect(error.mock.calls).toEqual([
      ["Some models in share shared do not exist on the remote server"],
      ["Some models in share shared do not exist on the local server"],
    ]);
  });

  it("writes members in catalog order and rejects a count mismatch before the request", async () => {
    const request = vi.fn();
    const share = { id: "shared" } as Share;
    const remote = [model(2, "b"), model(1, "a")];
    const api = () =>
      new TauriProxyShareApi(
        { request } as unknown as IServerRequestApi,
        catalog(remote),
        catalog([]),
      );

    await api().setModelsOnShare(share, [model(11, "a"), model(12, "b")]);
    for (const selected of [
      [model(11, "a"), model(11, "a")],
      [model(13, "c")],
    ]) {
      await expect(api().setModelsOnShare(share, selected)).rejects.toThrow(
        "Some models to set on the share do not exist on the remote server",
      );
    }
    expect(request.mock.calls).toEqual([
      ["/shares/shared/models", HttpMethod.PUT, { model_ids: [2, 1] }],
    ]);
  });
});
