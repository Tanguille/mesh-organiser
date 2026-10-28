/**
 * Characterization tests for the read-only resource/label API stubs used by demo
 * mode, and resource/label/model-trash API stubs used by public share pages.
 * Demo mutators throw a "Demo mode: ..." error; web-share mutators are silent no-ops
 * except the creators, which throw "Method not implemented.". Reads return
 * empty lists.
 */

import { describe, expect, it, vi } from "vitest";
import { DemoLabelApi } from "../demo/label";
import { DemoResourceApi } from "../demo/resource";
import { WebShareLabelApi } from "../web_share/label";
import { WebShareModelApi } from "../web_share/model";
import { WebShareResourceApi } from "../web_share/resource";
import type { LabelMeta } from "./label_api";
import type { Model } from "./model_api";
import type { ResourceMeta } from "./resource_api";
import type { IServerRequestApi } from "./server_request_api";
import type { Share } from "./share_api";

const resource: ResourceMeta = {
  id: 1,
  name: "project",
  flags: { completed: false },
  created: new Date(0),
  uniqueGlobalId: "resource-gid",
  lastModified: new Date(0),
};

const label: LabelMeta = {
  id: 1,
  name: "label",
  color: "#ff0000",
  lastModified: new Date(0),
  uniqueGlobalId: "label-gid",
};

const model = { id: 1 } as Model;

describe("DemoResourceApi", () => {
  const api = new DemoResourceApi();

  it("reads return empty lists", async () => {
    await expect(api.getResources()).resolves.toEqual([]);
    await expect(api.getGroupsForResource(resource)).resolves.toEqual([]);
  });

  it.each([
    [
      "addResource",
      () => api.addResource("name"),
      "Demo mode: Cannot add resources",
    ],
    [
      "editResource",
      () => api.editResource(resource),
      "Demo mode: Cannot edit resources",
    ],
    [
      "deleteResource",
      () => api.deleteResource(resource),
      "Demo mode: Cannot delete resources",
    ],
    [
      "setResourceOnGroup",
      () => api.setResourceOnGroup(resource, 1),
      "Demo mode: Cannot set resources on groups",
    ],
  ])("%s throws %j", async (_name, call, message) => {
    await expect(call()).rejects.toThrow(message);
  });
});

describe("WebShareResourceApi", () => {
  const api = new WebShareResourceApi();

  it("reads return empty lists", async () => {
    await expect(api.getResources()).resolves.toEqual([]);
    await expect(api.getGroupsForResource(resource)).resolves.toEqual([]);
  });

  it("addResource throws 'Method not implemented.'", async () => {
    await expect(api.addResource("name")).rejects.toThrow(
      "Method not implemented.",
    );
  });

  it("editResource, deleteResource and setResourceOnGroup are silent no-ops", async () => {
    await expect(api.editResource(resource)).resolves.toBeUndefined();
    await expect(api.deleteResource(resource)).resolves.toBeUndefined();
    await expect(api.setResourceOnGroup(resource, 1)).resolves.toBeUndefined();
    await expect(api.setResourceOnGroup(null, 1)).resolves.toBeUndefined();
  });
});

describe("WebShareLabelApi", () => {
  const api = new WebShareLabelApi();

  it("reads return empty lists", async () => {
    await expect(api.getLabels(true)).resolves.toEqual([]);
    await expect(api.getLabels(false)).resolves.toEqual([]);
    await expect(api.getKeywordsForLabel(label)).resolves.toEqual([]);
  });

  it("addLabel throws 'Method not implemented.'", async () => {
    await expect(api.addLabel("name", "#ffffff")).rejects.toThrow(
      "Method not implemented.",
    );
  });

  it("every other mutator is a silent no-op", async () => {
    await expect(api.editLabel(label)).resolves.toBeUndefined();
    await expect(api.deleteLabel(label)).resolves.toBeUndefined();
    await expect(api.setLabelsOnModel([label], model)).resolves.toBeUndefined();
    await expect(api.addLabelToModels(label, [model])).resolves.toBeUndefined();
    await expect(
      api.removeLabelFromModels(label, [model]),
    ).resolves.toBeUndefined();
    await expect(api.setKeywordsOnLabel(label, ["k"])).resolves.toBeUndefined();
    await expect(
      api.setChildrenOnLabel(label, [label]),
    ).resolves.toBeUndefined();
  });
});

describe("DemoLabelApi stubs", () => {
  const api = new DemoLabelApi();

  it("getKeywordsForLabel returns an empty list", async () => {
    await expect(api.getKeywordsForLabel(label)).resolves.toEqual([]);
  });

  it("setKeywordsOnLabel and setChildrenOnLabel throw demo-mode errors", async () => {
    await expect(api.setKeywordsOnLabel(label, ["k"])).rejects.toThrow(
      "Demo mode: Cannot modify label keywords",
    );
    await expect(api.setChildrenOnLabel(label, [label])).rejects.toThrow(
      "Demo mode: Cannot modify label hierarchy",
    );
  });
});

describe("WebShareModelApi trash stubs", () => {
  const request = vi.fn();
  const share: Share = {
    id: "share-1",
    createdAt: new Date(0),
    shareName: "share",
    userName: "alice",
    modelIds: [1],
  };
  const api = new WebShareModelApi(
    { request } as unknown as IServerRequestApi,
    share,
  );

  it("getTrashedModels returns an empty list and restoreModels is a silent no-op, without a request", async () => {
    await expect(api.getTrashedModels()).resolves.toEqual([]);
    await expect(api.restoreModels([model])).resolves.toBeUndefined();
    expect(request).not.toHaveBeenCalled();
  });
});
