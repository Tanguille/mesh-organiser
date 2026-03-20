import { describe, it, expect } from "vitest";
import { FileType } from "$lib/api/shared/blob_api";
import { isValidWorkerMessage } from "./parseModelWorkerMessage";

describe("isValidWorkerMessage", () => {
  it("returns false for null", () => {
    expect(isValidWorkerMessage(null)).toBe(false);
  });

  it("returns false for undefined", () => {
    expect(isValidWorkerMessage(undefined)).toBe(false);
  });

  it("returns false for empty object", () => {
    expect(isValidWorkerMessage({})).toBe(false);
  });

  it("returns true for valid STL message", () => {
    expect(
      isValidWorkerMessage({
        buffer: new Uint8Array(0),
        fileType: FileType.STL,
      }),
    ).toBe(true);
  });

  it("returns false when buffer is ArrayBuffer instead of Uint8Array", () => {
    expect(
      isValidWorkerMessage({
        buffer: new ArrayBuffer(8),
        fileType: FileType.STL,
      }),
    ).toBe(false);
  });

  it("returns false for invalid fileType string", () => {
    expect(
      isValidWorkerMessage({
        buffer: new Uint8Array(0),
        fileType: "not-a-real-type",
      }),
    ).toBe(false);
  });

  it("returns false for HMR-style noise object", () => {
    expect(isValidWorkerMessage({ type: "hot" })).toBe(false);
  });
});
