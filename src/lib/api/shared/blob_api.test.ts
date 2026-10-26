import { describe, expect, it } from "vitest";
import {
  FileType,
  IMPORT_EXTENSIONS,
  MODEL_EXTENSIONS,
  createBlobInstance,
  fileTypeToPlainFileExtension,
  normalizeFileTypeFilter,
} from "./blob_api";

describe("createBlobInstance", () => {
  it.each(["stp", "stp.zip", "STP", "STP.ZIP"])(
    "normalizes %s to STEP for download and sync filenames",
    (filetype) => {
      const blob = createBlobInstance(
        42,
        "model-content-hash",
        filetype,
        1024,
        "2026-10-24T12:00:00Z",
      );

      expect(blob.filetype).toBe(FileType.STEP);
      expect(fileTypeToPlainFileExtension(blob.filetype)).toBe(".step");
      expect(blob).toMatchObject({
        id: 42,
        sha256: "model-content-hash",
        size: 1024,
      });
    },
  );
});

describe("import extensions", () => {
  it("lists every supported model extension, including both STEP aliases", () => {
    expect(MODEL_EXTENSIONS).toEqual([
      "stl",
      "obj",
      "3mf",
      "step",
      "gcode",
      "stp",
    ]);
  });

  it("offers the supported model extensions and zip archives in import file pickers", () => {
    expect(IMPORT_EXTENSIONS).toEqual([
      "stl",
      "obj",
      "3mf",
      "step",
      "gcode",
      "stp",
      "zip",
    ]);
  });
});

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
