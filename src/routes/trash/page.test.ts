// @vitest-environment jsdom
/**
 * Behaviour tests for the trash page: it lists IModelApi.getTrashedModels,
 * restores only the selected models, refetches the list after every restore,
 * and shows the retention only when the desktop Behaviour settings are
 * available to an offline user.
 */

import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { createBlobInstance, IBlobApi } from "#lib/api/shared/blob_api";
import {
  createModelInstance,
  IModelApi,
  type Model,
} from "#lib/api/shared/model_api";
import {
  configurationDefault,
  ISettingsApi,
  SettingSection,
} from "#lib/api/shared/settings_api";
import { ISidebarStateApi } from "#lib/api/shared/sidebar_state_api";
import { configuration, currentUser } from "#lib/configuration.svelte";
import Page from "./+page.svelte";

const toastPromise = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());

vi.mock("svelte-sonner", () => ({
  toast: { promise: toastPromise, error: toastError },
}));

const DAY_MS = 24 * 60 * 60 * 1000;

function trashedModel(id: number, name: string, daysAgo: number) {
  return createModelInstance(
    id,
    name,
    createBlobInstance(id, `sha-${id}`, "stl", 1, "2024-01-01T00:00:00Z"),
    null,
    null,
    "2024-01-01T00:00:00Z",
    "2024-01-01T00:00:00Z",
    null,
    [],
    [],
    `gid-${id}`,
    new Date(Date.now() - daysAgo * DAY_MS).toISOString(),
  );
}

function provideApis(
  models: ReturnType<typeof trashedModel>[],
  sections: SettingSection[] = [],
) {
  let trashed = models;
  const modelApi = {
    getTrashedModels: vi.fn(async () => trashed),
    // Restores like the backend, so the page's refetch no longer lists them.
    restoreModels: vi.fn(async (restored: Model[]) => {
      const restoredIds = new Set(restored.map((model) => model.id));
      trashed = trashed.filter((model) => !restoredIds.has(model.id));
    }),
  };
  const loadSidebarState = vi.fn().mockResolvedValue({});
  const container = getContainer();
  container.addSingleton(IModelApi, modelApi);
  container.addSingleton(IBlobApi, {
    getBlobThumbnailUrl: vi.fn().mockResolvedValue(""),
  });
  container.addSingleton(ISettingsApi, { availableSections: () => sections });
  container.addSingleton(ISidebarStateApi, loadSidebarState);

  return { modelApi, loadSidebarState };
}

function restoreButton() {
  return screen.getByRole("button", { name: "Restore" }) as HTMLButtonElement;
}

// The first checkbox is "Select all"; the rest follow the model rows in order.
function rowCheckboxes() {
  return screen.getAllByRole("checkbox").slice(1);
}

describe("trash +page", () => {
  afterEach(() => {
    resetContainer();
    toastPromise.mockReset();
    toastError.mockReset();
    currentUser.permissions.onlineAccount = false;
    configuration.trash_retention_days =
      configurationDefault().trash_retention_days;
    configuration.show_date_on_list_view =
      configurationDefault().show_date_on_list_view;
  });

  it("lists every trashed model with its deletion age, restore disabled until something is selected", async () => {
    provideApis([trashedModel(1, "Benchy", 3), trashedModel(2, "Cube", 1)]);

    render(Page);

    expect(await screen.findByText("Benchy")).toBeTruthy();
    expect(screen.getByText("Cube")).toBeTruthy();
    expect(screen.getByText("Deleted 3 days ago")).toBeTruthy();
    expect(screen.getByText("Deleted 1 day ago")).toBeTruthy();
    expect(screen.getByText("2 models")).toBeTruthy();
    expect(restoreButton().disabled).toBe(true);
  });

  it("shows the deletion age in place of the added date even when list dates are on", async () => {
    configuration.show_date_on_list_view = true;
    provideApis([trashedModel(1, "Benchy", 3)]);

    render(Page);

    expect(await screen.findByText("Deleted 3 days ago")).toBeTruthy();
    expect(screen.queryByText(/^Added /)).toBeNull();
  });

  it("shows the empty state once an empty trash has loaded", async () => {
    provideApis([]);

    render(Page);

    expect(await screen.findByText(/The trash is empty\./)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Restore" })).toBeNull();
  });

  it("shows a load error instead of an empty or stuck-loading trash when the first fetch fails", async () => {
    const { modelApi } = provideApis([]);
    modelApi.getTrashedModels.mockRejectedValueOnce(new Error("offline"));

    render(Page);

    expect(await screen.findByText("Could not load the trash.")).toBeTruthy();
    expect(screen.queryByText(/The trash is empty\./)).toBeNull();
    expect(screen.queryByRole("button", { name: "Restore" })).toBeNull();
  });

  it("restores only the selected model, drops it from the list and refreshes the sidebar", async () => {
    const benchy = trashedModel(1, "Benchy", 3);
    const cube = trashedModel(2, "Cube", 1);
    const { modelApi, loadSidebarState } = provideApis([benchy, cube]);
    render(Page);
    await screen.findByText("Benchy");

    await fireEvent.click(rowCheckboxes()[1]);
    expect(screen.getByText("1 selected")).toBeTruthy();
    await fireEvent.click(restoreButton());

    await vi.waitFor(() => expect(screen.queryByText("Cube")).toBeNull());
    expect(modelApi.restoreModels).toHaveBeenCalledExactlyOnceWith([
      expect.objectContaining({ id: 2 }),
    ]);
    expect(modelApi.getTrashedModels).toHaveBeenCalledTimes(2);
    expect(screen.getByText("Benchy")).toBeTruthy();
    expect(loadSidebarState).toHaveBeenCalledTimes(1);
    expect(toastPromise).toHaveBeenCalledExactlyOnceWith(
      expect.any(Promise),
      expect.objectContaining({ loading: "Restoring 1 model..." }),
    );
  });

  it("removes successfully restored rows and refreshes the sidebar even when reloading the trash fails", async () => {
    const { modelApi, loadSidebarState } = provideApis([
      trashedModel(1, "Benchy", 3),
      trashedModel(2, "Cube", 1),
    ]);
    render(Page);
    await screen.findByText("Benchy");
    modelApi.getTrashedModels.mockRejectedValueOnce(new Error("offline"));

    await fireEvent.click(rowCheckboxes()[1]);
    await fireEvent.click(restoreButton());

    await vi.waitFor(() => expect(screen.queryByText("Cube")).toBeNull());
    expect(screen.getByText("Benchy")).toBeTruthy();
    expect(modelApi.restoreModels).toHaveBeenCalledExactlyOnceWith([
      expect.objectContaining({ id: 2 }),
    ]);
    expect(modelApi.getTrashedModels).toHaveBeenCalledTimes(2);
    await vi.waitFor(() => expect(loadSidebarState).toHaveBeenCalledTimes(1));
  });

  it("select all restores every model and leaves the empty state", async () => {
    const { modelApi } = provideApis([
      trashedModel(1, "Benchy", 3),
      trashedModel(2, "Cube", 1),
    ]);
    render(Page);
    await screen.findByText("Benchy");

    await fireEvent.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(screen.getByText("2 selected")).toBeTruthy();
    await fireEvent.click(restoreButton());

    expect(await screen.findByText(/The trash is empty\./)).toBeTruthy();
    expect(
      modelApi.restoreModels.mock.calls[0][0].map(
        (model: { id: number }) => model.id,
      ),
    ).toEqual([1, 2]);
  });

  it("a failed restore reports retry guidance, refetches and stays usable", async () => {
    const { modelApi, loadSidebarState } = provideApis([
      trashedModel(1, "Benchy", 3),
      trashedModel(2, "Cube", 1),
    ]);
    const failure = {
      error_message: "Internal error",
      error_inner_message: "Failed to find model to restore",
      error_type: "InternalError",
    };
    modelApi.restoreModels.mockRejectedValueOnce(failure);
    render(Page);
    await screen.findByText("Benchy");

    await fireEvent.click(rowCheckboxes()[1]);
    await fireEvent.click(restoreButton());

    await vi.waitFor(() =>
      expect(modelApi.getTrashedModels).toHaveBeenCalledTimes(2),
    );
    await vi.waitFor(() => expect(loadSidebarState).toHaveBeenCalledTimes(1));
    const [[, toastMessages]] = toastPromise.mock.calls;
    expect(toastMessages.error(failure)).toBe(
      "Failed to restore models. Refresh the trash and try again.",
    );
    expect(toastMessages.error(new Error("offline"))).toBe(
      "Failed to restore models. Refresh the trash and try again.",
    );
    expect(screen.getByText("Benchy")).toBeTruthy();
    expect(screen.getByText("Cube")).toBeTruthy();
    expect(screen.getByText("2 models")).toBeTruthy();
    expect(restoreButton().disabled).toBe(true);
    await fireEvent.click(rowCheckboxes()[1]);
    await fireEvent.click(restoreButton());

    await vi.waitFor(() => expect(screen.queryByText("Cube")).toBeNull());
    expect(modelApi.restoreModels).toHaveBeenCalledTimes(2);
  });

  it("keeps unrestored rows and refreshes the sidebar when both restore and reload fail", async () => {
    const { modelApi, loadSidebarState } = provideApis([
      trashedModel(1, "Benchy", 3),
      trashedModel(2, "Cube", 1),
    ]);
    render(Page);
    await screen.findByText("Benchy");
    modelApi.restoreModels.mockRejectedValueOnce(new Error("restore refused"));
    modelApi.getTrashedModels.mockRejectedValueOnce(new Error("offline"));

    await fireEvent.click(rowCheckboxes()[1]);
    await fireEvent.click(restoreButton());

    await vi.waitFor(() => expect(loadSidebarState).toHaveBeenCalledTimes(1));
    expect(modelApi.getTrashedModels).toHaveBeenCalledTimes(2);
    expect(screen.getByText("Benchy")).toBeTruthy();
    expect(screen.getByText("Cube")).toBeTruthy();
    expect(screen.getByText("2 models")).toBeTruthy();
    expect(restoreButton().disabled).toBe(true);
  });

  it("prunes a row selected during the restore once the refetch no longer lists it", async () => {
    const benchy = trashedModel(1, "Benchy", 3);
    const cube = trashedModel(2, "Cube", 1);
    const { modelApi } = provideApis([benchy, cube]);
    let finishRestore!: () => void;
    modelApi.restoreModels.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishRestore = resolve;
        }),
    );
    render(Page);
    await screen.findByText("Benchy");
    // Cube is restored elsewhere meanwhile, so only Benchy comes back.
    modelApi.getTrashedModels.mockResolvedValue([benchy]);

    await fireEvent.click(rowCheckboxes()[1]);
    await fireEvent.click(restoreButton());
    await fireEvent.click(rowCheckboxes()[0]);
    await fireEvent.click(rowCheckboxes()[1]);
    expect(screen.getByText("2 selected")).toBeTruthy();
    finishRestore();

    await vi.waitFor(() => expect(screen.queryByText("Cube")).toBeNull());
    expect(screen.getByText("1 selected")).toBeTruthy();
    expect(rowCheckboxes()[0].getAttribute("aria-checked")).toBe("true");
    expect(restoreButton().disabled).toBe(false);
  });

  it("shows the retention for an offline user when the Behaviour settings exist", async () => {
    configuration.trash_retention_days = 14;
    provideApis([], [SettingSection.Behaviour]);

    render(Page);

    expect(
      await screen.findByText(/permanently deleted 14\s+days after/),
    ).toBeTruthy();
  });

  it.each([
    ["an online account", [SettingSection.Behaviour], true],
    ["no Behaviour settings section", [], false],
  ])(
    "hides the desktop retention but explains permanent purging for %s",
    async (_name, sections, onlineAccount) => {
      currentUser.permissions.onlineAccount = onlineAccount;
      provideApis([], sections);

      render(Page);

      await screen.findByText(/The trash is empty\./);
      expect(
        screen.queryByText(/permanently deleted \d+\s+days? after/),
      ).toBeNull();
      expect(screen.getByText(/permanently (?:deleted|purged)/)).toBeTruthy();
    },
  );
});
