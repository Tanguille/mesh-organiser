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
