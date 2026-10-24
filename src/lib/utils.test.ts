/**
 * Pins isCommandError, the guard shared by every place that formats a failed
 * Tauri command: only the serialized ServiceError shape may pass.
 */

import { describe, expect, it } from "vitest";
import { isCommandError } from "./utils";

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
