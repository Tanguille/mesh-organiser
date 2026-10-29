/**
 * Pins isCommandError, the guard shared by every place that formats a failed
 * Tauri command: only the serialized ServiceError shape may pass. Also pins
 * toReadableSize's unit scaling, which must stop at its largest unit (TB).
 */

import { describe, expect, it } from "vitest";
import { isCommandError, toReadableSize } from "./utils";

describe("isCommandError", () => {
  it("accepts the serialized ServiceError shape, extra fields included", () => {
    expect(
      isCommandError({
        error_type: "InternalError",
        error_message: "Failed to find model to restore",
        error_inner_message: "",
      }),
    ).toBe(true);
  });

  it.each([
    ["null", null],
    ["undefined", undefined],
    ["a string", "Failed"],
    ["an Error", new Error("Failed")],
    ["a missing inner message", { error_message: "Failed" }],
    ["a missing message", { error_inner_message: "" }],
    [
      "a non-string message",
      { error_message: 1, error_inner_message: "detail" },
    ],
    [
      "a non-string inner message",
      { error_message: "Failed", error_inner_message: null },
    ],
  ])("rejects %s", (_description, reason) => {
    expect(isCommandError(reason)).toBe(false);
  });
});

describe("toReadableSize", () => {
  it.each([
    [0, "0.00 B"],
    [1023, "1023.00 B"],
    [1536, "1.50 KB"],
    [1024 ** 4, "1.00 TB"],
    // Past the largest unit the value keeps counting in TB instead of naming
    // a unit that doesn't exist.
    [1024 ** 5, "1024.00 TB"],
  ])("formats %d bytes as %s", (size, expected) => {
    expect(toReadableSize(size)).toBe(expected);
  });
});
