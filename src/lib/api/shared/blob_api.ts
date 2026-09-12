export enum FileType {
  STL = "stl.zip",
  OBJ = "obj.zip",
  THREEMF = "3mf",
  STEP = "step.zip",
  GCODE = "gcode.zip",
}

/**
 * Normalizes a file-type selection for backend filtering.
 *
 * Returns `null` when no types or every type is selected. Otherwise, returns
 * the unique selected types in first-seen order.
 */
export function normalizeFileTypeFilter(
  fileTypes: FileType[],
): FileType[] | null {
  const unique = [...new Set(fileTypes)];
  return unique.length === 0 || unique.length === Object.values(FileType).length
    ? null
    : unique;
}

export interface Blob {
  id: number;
  sha256: string;
  filetype: FileType;
  size: number;
  added: Date;
}

export function fileTypeToPlainFileExtension(fileType: FileType): string {
  switch (fileType) {
    case FileType.STL:
      return ".stl";
    case FileType.OBJ:
      return ".obj";
    case FileType.THREEMF:
      return ".3mf";
    case FileType.STEP:
      return ".step";
    case FileType.GCODE:
      return ".gcode";
  }
}

function plainFileExtensionToFileType(extension: string): FileType {
  switch (extension.toLowerCase()) {
    case "stl":
      return FileType.STL;
    case "obj":
      return FileType.OBJ;
    case "step":
      return FileType.STEP;
    case "gcode":
      return FileType.GCODE;
    default:
      return extension as FileType;
  }
}

export function createBlobInstance(
  id: number,
  sha256: string,
  filetype: string,
  size: number,
  added: string,
): Blob {
  return {
    id,
    sha256,
    filetype: plainFileExtensionToFileType(filetype),
    size,
    added: new Date(added),
  };
}

// Builds the blob thumbnail URL shared by the web and web-share blob APIs.
export function blobThumbnailUrl(origin: string, sha256: string): string {
  return origin + "/api/v1/blobs/" + sha256 + "/thumb";
}

export const IBlobApi = Symbol("IBlobApi");

export interface IBlobApi {
  getBlobBytes(blob: Blob): Promise<Uint8Array>;
  getBlobThumbnailUrl(blob: Blob): Promise<string>;
  // TODO: Move this to model at some point as it also includes data from the model serverside
  getBlobDownloadUrl(blob: Blob): Promise<string>;
  getBlobsDownloadUrl(blobs: Blob[]): Promise<string>;
}
