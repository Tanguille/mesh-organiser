import { afterEach, describe, expect, it, vi } from "vitest";
import type { IGroupApi } from "../shared/group_api";
import { GroupApi } from "./group";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("GroupApi group Trash", () => {
  afterEach(() => invoke.mockReset());

  it("preserves a null deletion identity on ambiguous legacy entries", async () => {
    invoke.mockResolvedValue([
      {
        id: 9,
        name: "Legacy",
        deleted: "2026-10-22T18:45:00Z",
        group_global_id: "group-9",
        deletion_operation_id: null,
      },
    ]);
    expect(await new GroupApi().getTrashedGroups()).toEqual([
      {
        id: 9,
        name: "Legacy",
        deleted: new Date("2026-10-22T18:45:00Z"),
        groupGlobalId: "group-9",
        deletionOperationId: null,
      },
    ]);
  });

  it("loads deletion identities and parses timestamps", async () => {
    invoke.mockResolvedValue([
      {
        id: 7,
        name: "Cabinet",
        deleted: "2026-10-22T18:45:00Z",
        group_global_id: "group-7",
        deletion_operation_id: "deletion-1",
      },
    ]);
    const api: IGroupApi = new GroupApi();
    expect(await api.getTrashedGroups!()).toEqual([
      {
        id: 7,
        name: "Cabinet",
        deleted: new Date("2026-10-22T18:45:00Z"),
        groupGlobalId: "group-7",
        deletionOperationId: "deletion-1",
      },
    ]);
    expect(invoke.mock.calls).toEqual([["get_trashed_groups"]]);
  });

  it("restores only the exact selected deletion identities", async () => {
    invoke.mockResolvedValue(undefined);
    const api: IGroupApi = new GroupApi();
    const selected = [
      { id: 7, groupGlobalId: "group-7", deletionOperationId: "deletion-1" },
      { id: 2, groupGlobalId: "group-2", deletionOperationId: "deletion-2" },
    ];
    await api.restoreGroups!(selected);
    expect(invoke.mock.calls).toEqual([
      [
        "restore_groups",
        {
          groups: [
            {
              id: 7,
              group_global_id: "group-7",
              deletion_operation_id: "deletion-1",
            },
            {
              id: 2,
              group_global_id: "group-2",
              deletion_operation_id: "deletion-2",
            },
          ],
        },
      ],
    ]);
  });
});
