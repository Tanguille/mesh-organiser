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
  it("keeps every model format available to scanners and import file pickers", () => {
    for (const fileType of Object.values(FileType)) {
      const extension = fileTypeToPlainFileExtension(fileType).slice(1);
      expect(MODEL_EXTENSIONS).toContain(extension);
      expect(IMPORT_EXTENSIONS).toContain(extension);
    }
  });

  it("accepts both STEP filename aliases", () => {
    for (const extension of ["step", "stp"]) {
      expect(MODEL_EXTENSIONS).toContain(extension);
      expect(IMPORT_EXTENSIONS).toContain(extension);
    }
  });

  it("offers zip only for imports and does not duplicate filter entries", () => {
    expect(MODEL_EXTENSIONS).not.toContain("zip");
    expect(IMPORT_EXTENSIONS).toContain("zip");
    for (const extensions of [MODEL_EXTENSIONS, IMPORT_EXTENSIONS]) {
      expect(new Set(extensions).size).toBe(extensions.length);
    }
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
