import { describe, expect, it } from "vitest";
import { FileType, normalizeFileTypeFilter } from "./blob_api";

describe("normalizeFileTypeFilter", () => {
  it("treats an empty or complete selection as no filter", () => {
    expect(normalizeFileTypeFilter([])).toBeNull();
    expect(normalizeFileTypeFilter(Object.values(FileType))).toBeNull();
  });

  it("dedupes a partial selection", () => {
    expect(
      normalizeFileTypeFilter([FileType.STL, FileType.STL, FileType.THREEMF]),
    ).toEqual([FileType.STL, FileType.THREEMF]);
  });
});
