import {
  globalImportSettings,
  importState,
  resetImportState,
} from "$lib/import.svelte";
import {
  ImportStatus,
  type ImportedModelsSet,
  type ImportModelSettings,
  type ImportState,
} from "../shared/tauri_import_api";
import type { IServerRequestApi } from "../shared/server_request_api";
import { updateSidebarState } from "$lib/sidebar_data.svelte";
import { TauriImportApi } from "../tauri/tauri_import";
import { invoke } from "@tauri-apps/api/core";
import type { IGroupApi } from "../shared/group_api";
export interface DirectoryScanModel {
  path: string;
  group_set: number | null;
  group_name: string | null;
  model_ids?: number[];
}

export interface UploadResult {
  import_state: ImportState;
  uploaded_models: DirectoryScanModel[];
}

async function uploadModels(
  paths: string[],
  recursive: boolean,
  sourceUrl: string | null,
  openInSlicer: boolean,
): Promise<UploadResult> {
  return await invoke<UploadResult>("upload_models_to_remote_server", {
    paths: paths,
    recursive: recursive,
    sourceUrl: sourceUrl,
    openInSlicer: openInSlicer,
  });
}

export class TauriWebImportApi extends TauriImportApi {
  protected requestApi: IServerRequestApi;
  protected groupApi: IGroupApi;

  constructor(requestApi: IServerRequestApi, groupApi: IGroupApi) {
    super();
    this.requestApi = requestApi;
    this.groupApi = groupApi;
  }

  public async startImportProcess(
    paths: string[],
    settings: ImportModelSettings,
  ): Promise<ImportState> {
    const recursive = settings.recursive ?? globalImportSettings.recursive;
    const directOpenInSlicer = settings.direct_open_in_slicer ?? false;
    const sourceUrl = settings.source_url;
    const importStateClone = { ...importState };

    resetImportState();
    const models = await uploadModels(
      paths,
      recursive,
      sourceUrl ?? null,
      directOpenInSlicer,
    );
    const noGroup = [];
    const groupMap: Record<number, DirectoryScanModel[]> = {};

    const importedModelsSet: ImportedModelsSet[] = [];

    for (const m of models.uploaded_models) {
      if (m.group_set === null || m.group_name === null) {
        noGroup.push(m);
      } else {
        if (!groupMap[m.group_set]) {
          groupMap[m.group_set] = [];
        }

        groupMap[m.group_set].push(m);
      }
    }

    if (noGroup.length > 0) {
      importedModelsSet.push({
        group_id: null,
        group_name: null,
        model_ids: noGroup.flatMap((g) => g.model_ids ?? []),
      });
    }

    for (const groupId in groupMap) {
      const groups = groupMap[groupId];
      const groupMeta = groups[0];
      const modelIds = groups.flatMap((g) => g.model_ids ?? []);

      const newGroup = await this.groupApi.addGroup(groupMeta.group_name!);
      await this.groupApi.addModelsToGroup(
        newGroup,
        modelIds.map((id) => ({ id })),
      );
      importedModelsSet.push({
        group_id: newGroup.id,
        group_name: newGroup.name,
        model_ids: modelIds,
      });
    }

    importStateClone.imported_models = importedModelsSet;
    importState.imported_models = importedModelsSet;
    await updateSidebarState();
    importState.status = ImportStatus.Finished;
    importStateClone.status = ImportStatus.Finished;
    return importStateClone;
  }
}
