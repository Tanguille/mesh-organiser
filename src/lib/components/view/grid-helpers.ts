import type { IDownloadApi } from "$lib/api/shared/download_api";
import type { Model } from "$lib/api/shared/model_api";
import { countWriter } from "$lib/utils";
import { toast } from "svelte-sonner";

/** Download one model or a zip of many, with the standard grid toast copy. */
export function downloadModelsWithToast(
  downloadApi: IDownloadApi,
  models: Model[],
): Promise<void> {
  if (models.length <= 0) {
    return Promise.resolve();
  }

  const promise =
    models.length === 1
      ? downloadApi.downloadModel(models[0]!)
      : downloadApi.downloadModelsAsZip(models);

  toast.promise(promise, {
    loading: `Downloading ${countWriter("model", models)}...`,
    success: (_: unknown) => {
      return `Downloaded ${countWriter("model", models)}`;
    },
  });

  return promise;
}
