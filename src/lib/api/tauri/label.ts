import { invoke } from "@tauri-apps/api/core";
import {
  type Label,
  type LabelMeta,
  stringColorToNumber,
  type ILabelApi,
} from "../shared/label_api";
import type { Model } from "../shared/model_api";
import {
  parseRawLabel,
  parseRawLabelMeta,
  type RawLabel,
  type RawLabelKeyword,
  type RawLabelMeta,
} from "../shared/raw_model";
import { dateToString } from "$lib/utils";

export {
  parseRawLabel,
  parseRawLabelMeta,
  type RawLabel,
  type RawLabelKeyword,
  type RawLabelMeta,
} from "../shared/raw_model";

export class LabelApi implements ILabelApi {
  async getLabels(includeUngroupedModels: boolean): Promise<Label[]> {
    const labels = await invoke<RawLabel[]>("get_labels", {
      includeUngroupedModels: includeUngroupedModels,
    });
    return labels.map((label) => parseRawLabel(label));
  }

  async addLabel(name: string, color: string): Promise<LabelMeta> {
    const label = await invoke<RawLabelMeta>("add_label", {
      labelName: name,
      labelColor: stringColorToNumber(color),
    });
    return parseRawLabelMeta(label);
  }

  async editLabel(
    label: LabelMeta,
    editTimestamp?: boolean,
    editGlobalId?: boolean,
  ): Promise<void> {
    const data: Record<string, unknown> = {
      labelId: label.id,
      labelName: label.name,
      labelColor: stringColorToNumber(label.color),
    };

    if (editTimestamp) {
      data.labelTimestamp = dateToString(label.lastModified);
    }

    if (editGlobalId) {
      data.labelGlobalId = label.uniqueGlobalId;
    }

    return await invoke("edit_label", data);
  }

  async deleteLabel(label: LabelMeta): Promise<void> {
    return await invoke("delete_label", { labelId: label.id });
  }

  async setLabelsOnModel(labels: LabelMeta[], model: Model): Promise<void> {
    return await invoke("set_labels_on_model", {
      labelIds: labels.map((label) => label.id),
      modelId: model.id,
    });
  }

  async addLabelToModels(label: LabelMeta, models: Model[]): Promise<void> {
    return await invoke("set_label_on_models", {
      labelId: label.id,
      modelIds: models.map((model) => model.id),
    });
  }

  async removeLabelFromModels(
    label: LabelMeta,
    models: Model[],
  ): Promise<void> {
    return await invoke("remove_label_from_models", {
      labelId: label.id,
      modelIds: models.map((model) => model.id),
    });
  }

  async setKeywordsOnLabel(
    label: LabelMeta,
    keywords: string[],
  ): Promise<void> {
    return await invoke("set_keywords_on_label", {
      labelId: label.id,
      keywords: keywords,
    });
  }

  async getKeywordsForLabel(label: LabelMeta): Promise<string[]> {
    return (
      await invoke<RawLabelKeyword[]>("get_keywords_for_label", {
        labelId: label.id,
      })
    ).map((kw) => kw.name);
  }

  async setChildrenOnLabel(
    label: LabelMeta,
    children: LabelMeta[],
  ): Promise<void> {
    return await invoke("set_childs_on_label", {
      parentLabelId: label.id,
      childLabelIds: children.map((child) => child.id),
    });
  }

  async removeChildrenFromLabel(
    label: LabelMeta,
    children: LabelMeta[],
  ): Promise<void> {
    return await invoke("remove_childs_from_label", {
      parentLabelId: label.id,
      childLabelIds: children.map((child) => child.id),
    });
  }
}
