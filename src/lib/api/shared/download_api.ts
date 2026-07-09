import { zipSync, type Zippable } from "fflate";
import { fileTypeToPlainFileExtension, type IBlobApi } from "./blob_api";
import type { Model } from "./model_api";
import {
  countWriter,
  nameCollectionOfModels,
  triggerBlobDownload,
} from "$lib/utils";
import { toast } from "svelte-sonner";

export const IDownloadApi = Symbol("IDownloadApi");

export interface IDownloadApi {
  downloadModel(model: Model): Promise<void>;
  downloadModelsAsZip(models: Model[]): Promise<void>;
}

export async function downloadModels(
  models: Model[],
  downloadApi: IDownloadApi | null,
): Promise<void> {
  if (!downloadApi) {
    return;
  }

  let promise;

  if (models.length <= 0) {
    return;
  } else if (models.length === 1) {
    promise = downloadApi.downloadModel(models[0]);
  } else {
    promise = downloadApi.downloadModelsAsZip(models);
  }

  toast.promise(promise, {
    loading: `Downloading ${countWriter("model", models)}...`,
    success: (_) => {
      return `Downloaded ${countWriter("model", models)}`;
    },
  });

  await promise;
}

export class DefaultDownloadApi implements IDownloadApi {
  blobApi: IBlobApi;

  constructor(blobApi: IBlobApi) {
    this.blobApi = blobApi;
  }

  async downloadModel(model: Model): Promise<void> {
    const data = await this.blobApi.getBlobBytes(model.blob);

    triggerBlobDownload(
      data as BlobPart,
      "application/octet-stream",
      model.name + fileTypeToPlainFileExtension(model.blob.filetype),
    );
  }

  makeStringSafeFilename(name: string): string {
    return name.replace(/[\\/:*?"<>|]/g, "_");
  }

  async downloadModelsAsZip(models: Model[]): Promise<void> {
    const promises = models.map((m) => this.blobApi.getBlobBytes(m.blob));

    const allData = await Promise.all(promises);

    const files: Zippable = {};

    for (let i = 0; i < models.length; i++) {
      const model = models[i];
      const data = allData[i];

      files[
        this.makeStringSafeFilename(model.name) +
          fileTypeToPlainFileExtension(model.blob.filetype)
      ] = data;
    }

    const zipped = zipSync(files);

    triggerBlobDownload(
      zipped as BlobPart,
      "application/zip",
      this.makeStringSafeFilename(nameCollectionOfModels(models)) + ".zip",
    );
  }
}
