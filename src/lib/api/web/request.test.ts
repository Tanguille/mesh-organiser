import { describe, it, expect, vi } from "vitest";
import { HttpMethod } from "../shared/server_request_api";
import { ServerRequestApi } from "./request";

describe("ServerRequestApi", () => {
  it("passes credentials include to fetch for JSON requests", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ ok: true }), { status: 200 }),
    );
    const api = new ServerRequestApi("https://example.com", fetchImpl, "include");
    await api.request("/test", HttpMethod.GET);

    expect(fetchImpl).toHaveBeenCalledWith(
      "https://example.com/api/v1/test",
      expect.objectContaining({ credentials: "include" }),
    );
  });

  it("defaults credentials to same-origin", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
    );
    const api = new ServerRequestApi("https://example.com", fetchImpl);
    await api.request("/x", HttpMethod.GET);

    expect(fetchImpl.mock.calls[0]?.[1]).toMatchObject({
      credentials: "same-origin",
    });
  });

  it("uses configured credentials for requestBinary and sendBinary", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response(new ArrayBuffer(0), { status: 200 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({}), { status: 200 }),
      );

    const api = new ServerRequestApi("https://example.com", fetchImpl, "include");
    await api.requestBinary("/bin", HttpMethod.GET);
    await api.sendBinary("/up", HttpMethod.POST, new File([], "f.stl"));

    expect(fetchImpl.mock.calls[0]?.[1]).toMatchObject({ credentials: "include" });
    expect(fetchImpl.mock.calls[1]?.[1]).toMatchObject({ credentials: "include" });
  });

  it("rejects on HTTP 500 with JSON body", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ code: "ERR" }), {
        status: 500,
        statusText: "Internal Server Error",
      }),
    );
    const api = new ServerRequestApi("https://example.com", fetchImpl);

    await expect(api.request("/x", HttpMethod.GET)).rejects.toThrow();
  });

  it("propagates network failure from fetch", async () => {
    const fetchImpl = vi
      .fn()
      .mockRejectedValue(new TypeError("Failed to fetch"));
    const api = new ServerRequestApi("https://example.com", fetchImpl);

    await expect(api.request("/x", HttpMethod.GET)).rejects.toThrow("Failed to fetch");
  });
});
