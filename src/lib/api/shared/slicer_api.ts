import { configuration } from "$lib/configuration.svelte";
import type { IBlobApi } from "./blob_api";
import type { Model } from "./model_api";

export interface SlicerEntry {
  slicer: string;
  installed: boolean;
}

export interface SlicingSettings {
  layerHeight: number; // 0.1, 0.2, 0.3
  infill: number; // 0-100
  supports: "none" | "everywhere" | "touching buildplate";
  material: string; // PLA, PETG, ABS, etc.
}

export interface SliceResult {
  success: boolean;
  slicedFileUrl: string;
  printTimeEstimate: number; // in minutes
  filamentUsed: number; // in grams
}

/** Response from `POST /api/v1/slicer/slice` (remote / web server). */
export interface SliceServerResponse {
  success: boolean;
  /** Model id of the registered slice output (not a separate blob-only id). */
  outputBlobId: number;
  outputBlobSha256: string;
  message?: string | null;
}

export const ISlicerApi = Symbol("ISlicerApi");

export interface ISlicerApi {
  openInSlicer(models: Model[]): Promise<void>;
  availableSlicers(): Promise<SlicerEntry[]>;
  sliceOnServer?(
    modelId: number,
    settings: SlicingSettings,
  ): Promise<SliceServerResponse>;
}

function slicerNameToDeepLink(slicerName: string): string | null {
  switch (slicerName) {
    case "PrusaSlicer":
      return "prusaslicer://open?file=";
    case "Cura":
      return "cura://open?file=";
    case "Bambu Studio":
      return "bambustudio://open?file=";
    case "OrcaSlicer":
      return "orcaslicer://open?file=";
    case "Mesh Organiser":
      return "meshorganiser://open?file=";
    default:
      return null;
  }
}

export class DefaultSlicerApi implements ISlicerApi {
  private blobApi: IBlobApi;

  constructor(blobApi: IBlobApi) {
    this.blobApi = blobApi;
  }

  async openInSlicer(models: Model[]): Promise<void> {
    let modelUrl;

    console.log(models);

    if (models.length === 0) {
      return;
    } else if (models.length === 1) {
      modelUrl = await this.blobApi.getBlobDownloadUrl(models[0].blob);
    } else if (models.length > 1) {
      modelUrl = await this.blobApi.getBlobsDownloadUrl(
        models.map((m) => m.blob),
      );
    }

    let deepLink = slicerNameToDeepLink(configuration.slicer ?? "OrcaSlicer");

    if (deepLink === null) {
      return;
    }

    deepLink += encodeURIComponent(modelUrl!);

    const link = document.createElement("a");
    link.href = deepLink;
    link.click();
    link.remove();
  }

  async availableSlicers(): Promise<SlicerEntry[]> {
    return [
      { slicer: "PrusaSlicer", installed: true },
      { slicer: "Cura", installed: true },
      { slicer: "OrcaSlicer", installed: true },
      { slicer: "Mesh Organiser", installed: true },
    ];
  }
}
