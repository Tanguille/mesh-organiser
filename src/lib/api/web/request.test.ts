/**
 * Pins how ServerRequestApi turns a failed response into an Error: a command
 * error body keeps its fields so isCommandError serves both platforms, other
 * JSON is stringified, and a body without JSON falls back to the status.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isCommandError } from "#lib/utils";
import { HttpMethod } from "../shared/server_request_api";
import { ServerRequestApi } from "./request";

function failingApi(body: string | null) {
  const fetchImpl = vi.fn(
    async () =>
      new Response(body, { status: 500, statusText: "Internal Server Error" }),
  );

  return new ServerRequestApi("https://mesh.example", fetchImpl);
}

async function requestError(body: string | null): Promise<unknown> {
  return failingApi(body)
    .request("/models/restore", HttpMethod.POST, { ids: [1] })
    .catch((error: unknown) => error);
}

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

describe("ServerRequestApi failed responses", () => {
  beforeEach(() => {
    vi.spyOn(console, "log").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("a command error body becomes an Error that isCommandError accepts", async () => {
    const body = {
      error_message: "Internal error",
      error_inner_message: "Failed to find model to restore",
      error_type: "InternalError",
    };

    const error = await requestError(JSON.stringify(body));

    expect(error).toBeInstanceOf(Error);
    expect((error as Error).message).toBe(
      "Internal error: Failed to find model to restore",
    );
    expect(isCommandError(error)).toBe(true);
    expect(error).toMatchObject(body);
  });

  it.each([
    ["an object without both messages", { error_message: "Internal error" }],
    ["an array", ["first", "second"]],
  ])(
    "any other JSON, such as %s, becomes an Error of its JSON",
    async (_name, body) => {
      const error = await requestError(JSON.stringify(body));

      expect(error).toBeInstanceOf(Error);
      expect((error as Error).message).toBe(JSON.stringify(body));
      expect(isCommandError(error)).toBe(false);
    },
  );

  it.each([
    ["a text", "Service Unavailable"],
    ["an empty", null],
  ])(
    "%s body becomes an Error naming the endpoint and status",
    async (_name, body) => {
      const error = await requestError(body);

      expect(error).toBeInstanceOf(Error);
      expect((error as Error).message).toBe(
        "Request to /models/restore failed with status 500: Internal Server Error",
      );
      expect(isCommandError(error)).toBe(false);
    },
  );
});
