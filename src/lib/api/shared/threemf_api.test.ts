/**
 * Pins the toast extractThreemfModels shows once extraction succeeds: the new
 * group's name in single quotes, with nothing trailing.
 */

import { describe, expect, it, vi } from "vitest";
import { createGroupMetaInstance, type IGroupApi } from "./group_api";
import type { Model } from "./model_api";
import { extractThreemfModels, type IThreemfApi } from "./threemf_api";

const mocks = vi.hoisted(() => ({ toastPromise: vi.fn() }));

vi.mock("svelte-sonner", () => ({ toast: { promise: mocks.toastPromise } }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("#lib/sidebar_data.svelte", () => ({
  updateSidebarState: vi.fn(),
}));

describe("extractThreemfModels", () => {
  it("toasts the imported group's name without a stray quote", async () => {
    const newGroup = createGroupMetaInstance(
      5,
      "Benchy plate",
      "2025-06-01T00:00:00Z",
      "2025-06-01T00:00:00Z",
      "group-gid",
    );
    const threemfApi = {
      extractThreemfModels: vi.fn().mockResolvedValue(newGroup),
    };
    const groupApi = { addModelsToGroup: vi.fn().mockResolvedValue(undefined) };

    await extractThreemfModels(
      { name: "benchy.3mf" } as Model,
      threemfApi as unknown as IThreemfApi,
      groupApi as unknown as IGroupApi,
    );

    const [[, toastMessages]] = mocks.toastPromise.mock.calls;
    expect(toastMessages.success(newGroup)).toBe("Imported 'Benchy plate'");
  });
});
