import { zipSync, type Zippable } from "fflate";
import { fileTypeToPlainFileExtension, type IBlobApi } from "./blob_api";
import type { Model } from "./model_api";
import { nameCollectionOfModels, triggerBlobDownload } from "$lib/utils";

export const IDownloadApi = Symbol("IDownloadApi");

export interface IDownloadApi {
  downloadModel(model: Model): Promise<void>;
  downloadModelsAsZip(models: Model[]): Promise<void>;
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
