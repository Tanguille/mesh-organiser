import type { ILabelApi, Label, LabelMeta } from "../shared/label_api";
import type { Model } from "../shared/model_api";

export class WebShareLabelApi implements ILabelApi {
  async getLabels(_includeUngroupedModels: boolean): Promise<Label[]> {
    return [];
  }

  async addLabel(_name: string, _color: string): Promise<LabelMeta> {
    throw new Error("Method not implemented.");
  }

  async editLabel(_label: LabelMeta): Promise<void> {}

  async deleteLabel(_label: LabelMeta): Promise<void> {}

  async setLabelsOnModel(_labels: LabelMeta[], _model: Model): Promise<void> {}

  async addLabelToModels(_label: LabelMeta, _models: Model[]): Promise<void> {}

  async removeLabelFromModels(
    _label: LabelMeta,
    _models: Model[],
  ): Promise<void> {}

  async setKeywordsOnLabel(
    _label: LabelMeta,
    _keywords: string[],
  ): Promise<void> {}

  async getKeywordsForLabel(_label: LabelMeta): Promise<string[]> {
    return [];
  }

  async setChildrenOnLabel(
    _label: LabelMeta,
    _children: LabelMeta[],
  ): Promise<void> {}

  async removeChildrenFromLabel(
    _label: LabelMeta,
    _children: LabelMeta[],
  ): Promise<void> {}
}
