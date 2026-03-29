import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const hoisted = vi.hoisted(() => ({
  mockIsTauri: vi.fn(),
  mockInvoke: vi.fn(),
  mockInitRemote: vi.fn().mockResolvedValue(undefined),
  mockInitLocal: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => hoisted.mockIsTauri(),
  invoke: (cmd: string, ...args: unknown[]) => hoisted.mockInvoke(cmd, ...args),
}));

vi.mock("./tauri/init_remote", () => ({
  initTauriRemoteApis: () => hoisted.mockInitRemote(),
}));

vi.mock("./tauri/init", () => ({
  initTauriLocalApis: () => hoisted.mockInitLocal(),
}));

import { initApi } from "./api";

describe("initApi", () => {
  beforeEach(() => {
    vi.unstubAllEnvs();
    hoisted.mockIsTauri.mockReturnValue(true);
    hoisted.mockInvoke.mockReset();
    hoisted.mockInitRemote.mockClear();
    hoisted.mockInitLocal.mockClear();
  });

  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("calls initTauriRemoteApis when running under Tauri mobile", async () => {
    vi.stubEnv("VITE_API_PLATFORM", "tauri");
    hoisted.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "is_mobile") {
        return Promise.resolve(true);
      }
      return Promise.resolve(undefined);
    });

    await initApi();

    expect(hoisted.mockInitRemote).toHaveBeenCalledTimes(1);
    expect(hoisted.mockInitLocal).not.toHaveBeenCalled();
  });

  it("calls initTauriLocalApis when running under Tauri desktop", async () => {
    vi.stubEnv("VITE_API_PLATFORM", "tauri");
    hoisted.mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "is_mobile") {
        return Promise.resolve(false);
      }
      return Promise.resolve(undefined);
    });

    await initApi();

    expect(hoisted.mockInitLocal).toHaveBeenCalledTimes(1);
    expect(hoisted.mockInitRemote).not.toHaveBeenCalled();
  });
});
