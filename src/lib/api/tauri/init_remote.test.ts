// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";

const hoisted = vi.hoisted(() => ({
  serverUrl: null as string | null,
  initWebApi: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-http", () => ({ fetch: vi.fn() }));

vi.mock("./server_url", () => ({
  getServerUrl: async () => hoisted.serverUrl,
}));

vi.mock("../web/init", () => ({
  initWebApi: (...args: unknown[]) => hoisted.initWebApi(...args),
}));

import { getContainer } from "../dependency_injection";
import { IBlobApi } from "../shared/blob_api";
import { IServerRequestApi } from "../shared/server_request_api";
import { ISlicerApi } from "../shared/slicer_api";
import { panicState } from "#lib/configuration.svelte";
import { initTauriRemoteApis } from "./init_remote";

describe("initTauriRemoteApis", () => {
  beforeEach(() => {
    vi.unstubAllEnvs();
    vi.stubEnv("VITE_MOBILE_SERVER_URL", "");
    hoisted.initWebApi.mockReset();
    panicState.inPanic = false;
    panicState.serverSetup = false;
  });

  it.each([null, "nas.example.com:3000"])(
    "panics with server setup for the unusable URL %s",
    async (url) => {
      hoisted.serverUrl = url;

      await initTauriRemoteApis();

      expect(panicState.inPanic).toBe(true);
      expect(panicState.serverSetup).toBe(true);
      expect(hoisted.initWebApi).not.toHaveBeenCalled();
    },
  );

  it("inits against the normalised URL and registers the server slicer once authenticated", async () => {
    hoisted.serverUrl = " https://nas.example.com:3000/ ";
    hoisted.initWebApi.mockImplementation(async () => {
      getContainer().addSingleton(IBlobApi, {});
      getContainer().addSingleton(IServerRequestApi, {});
    });

    await initTauriRemoteApis();

    expect(panicState.inPanic).toBe(false);
    expect(hoisted.initWebApi.mock.calls[0][0]).toBe(
      "https://nas.example.com:3000",
    );
    expect(
      getContainer().optional<ISlicerApi>(ISlicerApi)?.sliceOnServer,
    ).toBeTypeOf("function");
  });

  it("registers no slicer when unauthenticated", async () => {
    hoisted.serverUrl = "https://nas.example.com";

    await initTauriRemoteApis();

    expect(getContainer().optional<ISlicerApi>(ISlicerApi)).toBeFalsy();
  });
});
