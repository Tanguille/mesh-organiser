/**
 * Characterization tests for the host API implementations (demo, web, tauri)
 * and isCurrentPlatformDesktop. Demo and web share WebHostApi and differ only
 * in the platform they report. Vitest exposes a mutable import.meta.env for
 * the version cases below; production Vite builds inline the configured value.
 * Tauri asks the app.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { HostApi } from "../tauri/host";
import { WebHostApi } from "../web/host";
import { isCurrentPlatformDesktop, Platform, type IHostApi } from "./host_api";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("1.2.3-tauri"),
}));

afterEach(() => {
  vi.unstubAllEnvs();
});

describe.each([
  {
    name: "WebHostApi (demo)",
    host: new WebHostApi(Platform.DemoWebApp),
    platform: Platform.DemoWebApp,
  },
  { name: "WebHostApi", host: new WebHostApi(), platform: Platform.WebApp },
])("$name", ({ host, platform }) => {
  it("reports its platform and is not the desktop platform", async () => {
    await expect(host.getPlatform()).resolves.toBe(platform);
    await expect(isCurrentPlatformDesktop(host)).resolves.toBe(false);
  });

  it("returns the trimmed VITE_APP_VERSION", async () => {
    vi.stubEnv("VITE_APP_VERSION", "  v9.8.7 ");
    expect(import.meta.env.VITE_APP_VERSION).toBe("  v9.8.7 ");

    await expect(host.getVersion()).resolves.toBe("v9.8.7");
  });

  it("falls back to 'dev' when VITE_APP_VERSION is blank", async () => {
    vi.stubEnv("VITE_APP_VERSION", "   ");
    expect(import.meta.env.VITE_APP_VERSION).toBe("   ");

    await expect(host.getVersion()).resolves.toBe("dev");
  });
});

describe("HostApi (tauri)", () => {
  it("reports the desktop platform and the version from the tauri app", async () => {
    const host = new HostApi();

    await expect(host.getPlatform()).resolves.toBe(
      Platform.TauriOfflineDesktop,
    );
    await expect(host.getVersion()).resolves.toBe("1.2.3-tauri");
    await expect(isCurrentPlatformDesktop(host)).resolves.toBe(true);
  });
});

describe("isCurrentPlatformDesktop", () => {
  it.each([
    [Platform.TauriOfflineDesktop, true],
    [Platform.DemoWebApp, false],
    [Platform.WebApp, false],
  ])("platform %s -> %s", async (platform, expected) => {
    const host: IHostApi = {
      getPlatform: async () => platform,
      getVersion: async () => "",
    };

    await expect(isCurrentPlatformDesktop(host)).resolves.toBe(expected);
  });
});
