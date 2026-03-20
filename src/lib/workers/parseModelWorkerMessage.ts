import { FileType } from "$lib/api/shared/blob_api";

const FILE_TYPE_VALUES = new Set<string>(Object.values(FileType));

export function isValidWorkerMessage(
  data: unknown,
): data is { buffer: Uint8Array; fileType: FileType } {
  if (data === null || typeof data !== "object") {
    return false;
  }
  const { buffer, fileType } = data as {
    buffer?: unknown;
    fileType?: unknown;
  };
  if (!(buffer instanceof Uint8Array)) {
    return false;
  }
  return typeof fileType === "string" && FILE_TYPE_VALUES.has(fileType);
}
