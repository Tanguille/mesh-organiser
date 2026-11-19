import { describe, expect, it } from "vitest";
import { assertMobileReachable, normalizeServerBaseUrl } from "./server_url";

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

describe("assertMobileReachable", () => {
  it.each([
    "https://nas.local:3000",
    "http://127.0.0.1:3000",
    "http://localhost",
  ])("accepts %s", (url) => {
    expect(() => assertMobileReachable(url)).not.toThrow();
  });

  it("rejects plain HTTP to a LAN host", () => {
    expect(() => assertMobileReachable("http://192.168.1.10:3000")).toThrow(
      "HTTPS only",
    );
  });
});
