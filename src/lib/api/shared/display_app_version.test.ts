import { describe, expect, it } from "vitest";
import { displayAppVersion } from "./display_app_version";

describe("displayAppVersion", () => {
  it("returns trimmed string when non-empty", () => {
    expect(displayAppVersion("  v1.2.3  ")).toBe("v1.2.3");
  });

  it("returns dev for undefined, null, empty, or whitespace-only", () => {
    expect(displayAppVersion(undefined)).toBe("dev");
    expect(displayAppVersion(null)).toBe("dev");
    expect(displayAppVersion("")).toBe("dev");
    expect(displayAppVersion("   \t  ")).toBe("dev");
  });

  it("returns dev for non-string", () => {
    expect(displayAppVersion(123)).toBe("dev");
  });
});
