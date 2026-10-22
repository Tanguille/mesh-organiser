import { describe, it, expect, vi } from "vitest";
import { HttpMethod } from "../shared/server_request_api";
import { ServerRequestApi } from "./request";

describe("ServerRequestApi", () => {
  it("sends same-origin credentials for JSON requests", async () => {
    const fetchImpl = vi.fn().mockResolvedValue(
      new Response("{}", {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
    const api = new ServerRequestApi("https://example.com", fetchImpl);
    await api.request("/x", HttpMethod.GET);

    expect(fetchImpl.mock.calls[0]?.[1]).toMatchObject({
      credentials: "same-origin",
    });
  });

  it("sends same-origin credentials for requestBinary and sendBinary", async () => {
    const fetchImpl = vi
      .fn()
      .mockResolvedValueOnce(new Response(new ArrayBuffer(0), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({}), { status: 200 }));

    const api = new ServerRequestApi("https://example.com", fetchImpl);
    await api.requestBinary("/bin", HttpMethod.GET);
    await api.sendBinary("/up", HttpMethod.POST, new File([], "f.stl"));

    expect(fetchImpl.mock.calls[0]?.[1]).toMatchObject({
      credentials: "same-origin",
    });
    expect(fetchImpl.mock.calls[1]?.[1]).toMatchObject({
      credentials: "same-origin",
    });
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

    await expect(api.request("/x", HttpMethod.GET)).rejects.toThrow(
      "Failed to fetch",
    );
  });
});
