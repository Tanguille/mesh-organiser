import { describe, expect, it, vi } from "vitest";
import type { IGroupApi } from "../shared/group_api";
import {
  HttpMethod,
  type IServerRequestApi,
} from "../shared/server_request_api";
import { WebGroupApi } from "./group";

describe("WebGroupApi group Trash", () => {
  it("preserves a null deletion identity on ambiguous legacy entries", async () => {
    const request = vi.fn().mockResolvedValue([
      {
        id: 9,
        name: "Legacy",
        deleted: "2026-10-22T18:45:00Z",
        group_global_id: "group-9",
        deletion_operation_id: null,
      },
    ]);
    const api = new WebGroupApi({ request } as unknown as IServerRequestApi);
    expect(await api.getTrashedGroups()).toEqual([
      {
        id: 9,
        name: "Legacy",
        deleted: new Date("2026-10-22T18:45:00Z"),
        groupGlobalId: "group-9",
        deletionOperationId: null,
      },
    ]);
  });
  it("loads deleted groups with their deletion identity and parses timestamps", async () => {
    const request = vi.fn().mockResolvedValue([
      {
        id: 7,
        name: "Cabinet",
        deleted: "2026-10-22T18:45:00Z",
        group_global_id: "group-7",
        deletion_operation_id: "deletion-1",
      },
    ]);
    const api: IGroupApi = new WebGroupApi({
      request,
    } as unknown as IServerRequestApi);
    expect(await api.getTrashedGroups!()).toEqual([
      {
        id: 7,
        name: "Cabinet",
        deleted: new Date("2026-10-22T18:45:00Z"),
        groupGlobalId: "group-7",
        deletionOperationId: "deletion-1",
      },
    ]);
    expect(request.mock.calls).toEqual([["/groups/trash", HttpMethod.GET]]);
  });

  it("restores the exact selected deletion identities", async () => {
    const request = vi.fn().mockResolvedValue(undefined);
    const api: IGroupApi = new WebGroupApi({
      request,
    } as unknown as IServerRequestApi);
    const selected = [
      { id: 7, groupGlobalId: "group-7", deletionOperationId: "deletion-1" },
      { id: 2, groupGlobalId: "group-2", deletionOperationId: "deletion-2" },
    ];
    await api.restoreGroups!(selected);
    expect(request.mock.calls).toEqual([
      [
        "/groups/restore",
        HttpMethod.POST,
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
