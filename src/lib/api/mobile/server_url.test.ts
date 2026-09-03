import { describe, expect, it } from "vitest";
import { normalizeServerBaseUrl } from "./server_url";

describe("normalizeServerBaseUrl", () => {
  it("strips trailing slash for https://nas.local:3000/", () => {
    expect(normalizeServerBaseUrl("https://nas.local:3000/")).toBe(
      "https://nas.local:3000",
    );
  });

  it("rejects ftp://x", () => {
    expect(() => normalizeServerBaseUrl("ftp://x")).toThrow(
      "URL must start with http:// or https://",
    );
  });

  it("trims surrounding whitespace and trailing slashes", () => {
    expect(normalizeServerBaseUrl("  https://example.com/  ")).toBe(
      "https://example.com",
    );
  });
});
