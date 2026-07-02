import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";
import type { Configuration } from "./api/shared/settings_api";
import type { Model } from "./api/shared/model_api";
import { FileType } from "./api/shared/blob_api";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function debounce<T extends unknown[]>(
  callback: (...args: T) => void,
  timeMs: number,
): (...args: T) => void {
  let timeoutId: number | undefined = undefined;

  return (...args: T) => {
    if (timeoutId !== undefined) window.clearTimeout(timeoutId);

    timeoutId = window.setTimeout(() => {
      callback(...args);
    }, timeMs);
  };
}

export function toReadableSize(size: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];

  let unitIndex = 0;
  while (size >= 1024 && unitIndex < units.length) {
    size /= 1024;
    unitIndex++;
  }

  return `${size.toFixed(2)} ${units[unitIndex]}`;
}

export function countWriter(type: string, groups: unknown[]): string {
  return `${groups.length} ${type}${groups.length === 1 ? "" : "s"}`;
}

export function loadModelAutomatically(
  configuration: Configuration,
  model: Model,
): boolean {
  const modelSizeInMb = model.blob.size / 1024 / 1024;

  let maxSize = 0;

  switch (model.blob.filetype) {
    case FileType.STL:
      maxSize = configuration.max_size_model_stl_preview;
      break;
    case FileType.OBJ:
      maxSize = configuration.max_size_model_obj_preview;
      break;
    case FileType.THREEMF:
      maxSize = configuration.max_size_model_3mf_preview;
      break;
  }

  return modelSizeInMb <= maxSize;
}

export function isModelPreviewable(model: Model): boolean {
  return (
    model.blob.filetype === FileType.STL ||
    model.blob.filetype === FileType.OBJ ||
    model.blob.filetype === FileType.THREEMF
  );
}

export function isModelSlicable(model: Model): boolean {
  return (
    model.blob.filetype === FileType.STL ||
    model.blob.filetype === FileType.OBJ ||
    model.blob.filetype === FileType.THREEMF ||
    model.blob.filetype === FileType.STEP
  );
}

export function fileTypeToDisplayName(fileType: FileType): string {
  switch (fileType) {
    case FileType.STL:
      return "STL";
    case FileType.OBJ:
      return "OBJ";
    case FileType.THREEMF:
      return "3MF";
    case FileType.STEP:
      return "STEP";
    case FileType.GCODE:
      return "GCODE";
    default:
      return "Unknown";
  }
}

export function fileTypeToColor(fileType: FileType): string {
  switch (fileType) {
    case FileType.STL:
      return "text-black bg-blue-400 hover:bg-blue-500";
    case FileType.THREEMF:
      return "text-black bg-emerald-500 hover:bg-emerald-600";
    case FileType.OBJ:
      return "text-black bg-purple-400 hover:bg-purple-500";
    case FileType.GCODE:
      return "text-black bg-orange-400 hover:bg-orange-500";
    default:
      return "text-black bg-gray-300 hover:bg-gray-400";
  }
}

export function nameCollectionOfModels(models: Model[]): string {
  const set = new Set<number>(models.map((x) => x.group?.id ?? -1));
  if (set.size === 1 && !set.has(-1)) {
    return models[0].group!.name;
  }

  let str = models
    .slice(0, 5)
    .map((x) => x.name)
    .join("+");

  if (models.length > 5) {
    str += `+${models.length - 5} more...`;
  }

  return str;
}

export function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// Dedupe by id, preserving first occurrence. O(n) via a Set instead of the
// O(n^2) `.filter((v, i, a) => a.findIndex(...) === i)` idiom it replaces.
export function uniqueById<T extends { id: number }>(items: T[]): T[] {
  const seen = new Set<number>();
  return items.filter((item) =>
    seen.has(item.id) ? false : (seen.add(item.id), true),
  );
}

// Pick the model with the largest blob as the single representative image
// for a group/collection.
export function representativeModel(models: Model[]): Model {
  // Single-pass max; this runs per group thumbnail in scrolling grid views,
  // so avoid the copy + O(n log n) sort. Like the sort it replaces, an empty
  // list yields undefined.
  let best = models[0];
  for (const model of models) {
    if (model.blob.size > best.blob.size) {
      best = model;
    }
  }
  return best;
}

// Runs `fn` over `items` with at most `limit` tasks in flight, starting tasks
// lazily as slots free up and rejecting on the first failure without starting
// more. Shared by the web file importer and the sync steps.
export async function runWithLimit<T>(
  items: T[],
  fn: (item: T) => Promise<void>,
  limit: number = 4,
): Promise<void> {
  let index = 0;
  let active = 0;
  let failed = false;

  return new Promise((resolve, reject) => {
    const launchNext = () => {
      while (active < limit) {
        if (failed) {
          return;
        }

        if (index >= items.length) {
          if (active === 0) resolve();
          break;
        }

        // A synchronous throw from fn must reject the run, not escape a
        // .finally() callback as an unhandled rejection that hangs the await.
        let task: Promise<void>;
        try {
          task = fn(items[index++]);
        } catch (err) {
          failed = true;
          reject(err);
          return;
        }

        active++;

        task
          .catch((err) => {
            failed = true;
            reject(err);
          })
          .finally(() => {
            active--;
            launchNext();
          });
      }
    };

    launchNext();
  });
}

// Trigger a browser download/navigation via a transient anchor element.
// Setting `filename` adds a `download` attribute; omit it for deep-link hrefs.
export function triggerDownload(url: string, filename?: string): void {
  const link = document.createElement("a");
  link.href = url;
  if (filename !== undefined) {
    link.download = filename;
  }
  link.click();
  link.remove();
}

// Wrap blob data in an object URL and trigger a download with the given filename.
export function triggerBlobDownload(
  data: BlobPart,
  mime: string,
  filename: string,
): void {
  triggerDownload(
    URL.createObjectURL(new Blob([data], { type: mime })),
    filename,
  );
}

export function dateToString(date: Date): string {
  const isoString = date.toISOString();
  if (isoString.includes(".")) {
    return isoString.split(".")[0] + "Z";
  }

  return isoString;
}

export function timeSinceDate(date: Date): string {
  const seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (days > 0) {
    return `${days} day${days === 1 ? "" : "s"} ago`;
  } else if (hours > 0) {
    return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  } else if (minutes > 0) {
    return `${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  } else {
    return `${seconds} second${seconds === 1 ? "" : "s"} ago`;
  }
}

export function handleGridItemKeyDown<T>(
  item: T,
  event: KeyboardEvent,
  onClick: (item: T, event: MouseEvent | KeyboardEvent) => void | Promise<void>,
  useSyntheticMouseEvent: boolean = false,
) {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    if (useSyntheticMouseEvent) {
      // Create a synthetic click event for grid items that expect mouse events
      const syntheticEvent = new MouseEvent("click", {
        ctrlKey: event.ctrlKey || event.metaKey,
        shiftKey: event.shiftKey,
        bubbles: true,
      });
      onClick(item, syntheticEvent);
    } else {
      // Pass the keyboard event directly
      onClick(item, event);
    }
  }
}
