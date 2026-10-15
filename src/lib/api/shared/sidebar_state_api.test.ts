/**
 * Characterization tests for the sidebar state APIs and how
 * updateSidebarState (sidebar_data.svelte.ts) consumes them, pinned before the
 * sidebar-state API simplification. DefaultSidebarStateApi resolves its APIs
 * from the container on every call and maps eight parallel results onto
 * SidebarState; distinct values per source catch a swapped index.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { configuration } from "#lib/configuration.svelte";
import { sidebarState, updateSidebarState } from "#lib/sidebar_data.svelte";
import { getContainer, resetContainer } from "../dependency_injection";
import { IGroupApi } from "./group_api";
import { ILabelApi, type Label } from "./label_api";
import { IModelApi, type ModelFlags } from "./model_api";
import { IResourceApi } from "./resource_api";
import { IShareApi } from "./share_api";
import {
  DefaultSidebarStateApi,
  defaultSidebarState,
  EmptySidebarStateApi,
  ISidebarStateApi,
  type SidebarState,
} from "./sidebar_state_api";
import { ISlicerApi, type SlicerEntry } from "./slicer_api";

const labels = [{ meta: { id: 1 } }] as Label[];
const slicers = [{ slicer: "PrusaSlicer", installed: true }] as SlicerEntry[];

function modelCountFor(flags: ModelFlags | null): number {
  if (flags === null) {
    return 11;
  }

  if (flags.printed && !flags.favorite) {
    return 22;
  }

  if (!flags.printed && flags.favorite) {
    return 33;
  }

  return -1;
}

function registerApis({ slicer = true, share = true } = {}) {
  const apis = {
    model: { getModelCount: vi.fn(async (flags) => modelCountFor(flags)) },
    group: { getGroupCount: vi.fn().mockResolvedValue(44) },
    resource: { getResources: vi.fn().mockResolvedValue([{}, {}, {}]) },
    label: { getLabels: vi.fn().mockResolvedValue(labels) },
    slicer: { availableSlicers: vi.fn().mockResolvedValue(slicers) },
    share: { getShares: vi.fn().mockResolvedValue([{}, {}, {}, {}, {}]) },
  };
  const container = getContainer();
  container.addSingleton(IModelApi, apis.model);
  container.addSingleton(IGroupApi, apis.group);
  container.addSingleton(IResourceApi, apis.resource);
  container.addSingleton(ILabelApi, apis.label);
  if (slicer) {
    container.addSingleton(ISlicerApi, apis.slicer);
  }
  if (share) {
    container.addSingleton(IShareApi, apis.share);
  }

  return apis;
}

const fullState: SidebarState = {
  modelCount: 11,
  printHistoryCount: 22,
  favoriteCount: 33,
  groupCount: 44,
  projectCount: 3,
  labels,
  availableSlicers: slicers,
  shareCount: 5,
};

afterEach(() => {
  resetContainer();
  configuration.show_ungrouped_models_in_groups = true;
  Object.assign(sidebarState, defaultSidebarState());
});

describe("DefaultSidebarStateApi", () => {
  it("maps every source onto the sidebar state", async () => {
    const apis = registerApis();

    const state = await new DefaultSidebarStateApi().getSidebarState();

    expect(state).toStrictEqual(fullState);
    expect(apis.model.getModelCount.mock.calls).toEqual([
      [null],
      [{ printed: true, favorite: false }],
      [{ printed: false, favorite: true }],
    ]);
    expect(apis.group.getGroupCount).toHaveBeenCalledWith(true);
    expect(apis.label.getLabels).toHaveBeenCalledWith(true);
  });

  it("passes show_ungrouped_models_in_groups through to the group count", async () => {
    const apis = registerApis();
    configuration.show_ungrouped_models_in_groups = false;

    await new DefaultSidebarStateApi().getSidebarState();

    expect(apis.group.getGroupCount).toHaveBeenCalledWith(false);
  });

  it("skips the share api when constructed with includeShares false", async () => {
    const apis = registerApis();

    const state = await new DefaultSidebarStateApi(false).getSidebarState();

    expect(state).toStrictEqual({ ...fullState, shareCount: 0 });
    expect(apis.share.getShares).not.toHaveBeenCalled();
  });

  it("treats missing slicer and share apis as empty", async () => {
    registerApis({ slicer: false, share: false });

    const state = await new DefaultSidebarStateApi().getSidebarState();

    expect(state).toStrictEqual({
      ...fullState,
      availableSlicers: [],
      shareCount: 0,
    });
  });

  it("rejects when a required api is not registered", async () => {
    await expect(
      new DefaultSidebarStateApi().getSidebarState(),
    ).rejects.toThrow("Dependency not found");
  });
});

describe("EmptySidebarStateApi", () => {
  it("returns the default sidebar state without touching the container", async () => {
    await expect(
      new EmptySidebarStateApi().getSidebarState(),
    ).resolves.toStrictEqual(defaultSidebarState());
  });
});

describe("updateSidebarState", () => {
  it("leaves the sidebar state untouched when no sidebar state api is registered", async () => {
    sidebarState.modelCount = 99;

    await updateSidebarState();

    expect(sidebarState.modelCount).toBe(99);
  });

  it("assigns the registered api's state onto the shared sidebar state", async () => {
    registerApis();
    getContainer().addSingleton(ISidebarStateApi, new DefaultSidebarStateApi());

    await updateSidebarState();

    expect({ ...sidebarState }).toStrictEqual(fullState);
  });

  it("resets the shared sidebar state to defaults with the empty api", async () => {
    Object.assign(sidebarState, fullState);
    getContainer().addSingleton(ISidebarStateApi, new EmptySidebarStateApi());

    await updateSidebarState();

    expect({ ...sidebarState }).toStrictEqual(defaultSidebarState());
  });
});
