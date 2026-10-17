/**
 * Characterization tests for TauriImportApi's event wiring. Locks the exact set
 * of Tauri events listened to, each handler's effect on the shared importState,
 * and the argument objects remote imports (deep link, built-in browser
 * download, Downloads folder watch) pass to startImportProcess and the
 * add_model command.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WatchEvent } from "@tauri-apps/plugin-fs";
import { BaseDirectory } from "@tauri-apps/api/path";
import { accountLinkData } from "#lib/account_link_data.svelte";
import { configuration } from "#lib/configuration.svelte";
import {
  defaultImportState,
  globalImportSettings,
  importState,
  resetImportState,
} from "#lib/import.svelte";
import { configurationDefault } from "../shared/settings_api";
import { ImportStatus, type ImportState } from "../shared/tauri_import_api";
import { TauriImportApi } from "./tauri_import";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  invoke: vi.fn(),
  watch: vi.fn(),
  goto: vi.fn(),
  toastPromise: vi.fn(),
  unminimize: vi.fn(),
  setFocus: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    unminimize: mocks.unminimize,
    setFocus: mocks.setFocus,
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/plugin-fs")>()),
  watch: mocks.watch,
}));
vi.mock("svelte-sonner", () => ({ toast: { promise: mocks.toastPromise } }));
vi.mock("$app/navigation", () => ({ goto: mocks.goto }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({
  page: { url: new URL("http://localhost/") },
}));

type Handler = (event: { payload: unknown }) => Promise<void> | void;

const handlers = new Map<string, Handler>();
const unlisteners = new Map<string, ReturnType<typeof vi.fn>>();
const watchUnlisten = vi.fn();
let watchCallback: ((event: WatchEvent) => Promise<void>) | undefined;

const finishedImport: ImportState = {
  ...defaultImportState(),
  status: ImportStatus.Finished,
};

beforeEach(() => {
  vi.spyOn(console, "log").mockImplementation(() => {});
  mocks.listen.mockImplementation(async (name: string, handler: Handler) => {
    const unlisten = vi.fn();
    handlers.set(name, handler);
    unlisteners.set(name, unlisten);

    return unlisten;
  });
  mocks.watch.mockImplementation(
    async (_path: string, callback: (event: WatchEvent) => Promise<void>) => {
      watchCallback = callback;

      return watchUnlisten;
    },
  );
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "download_file") {
      return {
        path: "/downloads/benchy.stl",
        source_uri: "https://cdn/benchy",
      };
    }

    return finishedImport;
  });
  mocks.toastPromise.mockImplementation((promise: Promise<unknown>) => promise);
});

afterEach(() => {
  vi.clearAllMocks();
  vi.restoreAllMocks();
  handlers.clear();
  unlisteners.clear();
  watchCallback = undefined;
  Object.assign(configuration, configurationDefault());
  resetImportState();
  Object.assign(accountLinkData, {
    showLinkUi: false,
    baseUrl: "",
    userName: "",
    linkToken: "",
  });
});

async function initApi(): Promise<TauriImportApi> {
  const api = new TauriImportApi();
  await api.initImportListeners();

  return api;
}

function emit(name: string, payload: unknown) {
  const handler = handlers.get(name);
  if (!handler) {
    throw new Error(`No listener registered for ${name}`);
  }

  return handler({ payload });
}

function addModelCalls() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "add_model");
}

const listenedEvents = [
  "import-status",
  "import-model-count",
  "import-thumbnail-count",
  "import-model-group",
  "import-failure",
  "import-model-total",
  "import-all-data",
  "deep-link",
  "download-started",
  "download-finished",
  "tauri://drag-drop",
  "account-link",
];

describe("initImportListeners registration", () => {
  it("listens to exactly the import, download, drag-drop and account-link events", async () => {
    await initApi();

    expect(mocks.listen.mock.calls.map(([name]) => name)).toEqual(
      listenedEvents,
    );
    expect(mocks.watch).not.toHaveBeenCalled();
  });

  it("watches the Downloads folder only when watch_downloads_folder is enabled", async () => {
    configuration.watch_downloads_folder = true;

    await initApi();

    expect(mocks.watch.mock.calls).toEqual([
      ["", expect.any(Function), { baseDir: BaseDirectory.Download }],
    ]);
  });

  it("unlistens every previous listener, including the folder watch, when re-initialised", async () => {
    configuration.watch_downloads_folder = true;
    const api = await initApi();
    const firstUnlisteners = [...unlisteners.values()];

    await api.initImportListeners();

    for (const unlisten of firstUnlisteners) {
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
    expect(watchUnlisten).toHaveBeenCalledTimes(1);
    expect(mocks.listen).toHaveBeenCalledTimes(listenedEvents.length * 2);
  });

  it("resets the import state and copies the default import toggles from the configuration", async () => {
    importState.status = ImportStatus.Failure;
    configuration.default_enabled_delete_after_import = true;
    configuration.default_enabled_recursive_import = false;
    configuration.default_enabled_import_as_path = true;

    await initApi();

    expect({ ...importState }).toStrictEqual(defaultImportState());
    expect({ ...globalImportSettings }).toStrictEqual({
      delete_after_import: true,
      recursive: false,
      import_as_path: true,
    });
  });
});

describe("import progress handlers", () => {
  it.each([
    ["import-status", ImportStatus.ProcessingThumbnails, "status"],
    ["import-model-count", 4, "imported_models_count"],
    ["import-thumbnail-count", 3, "finished_thumbnails_count"],
    ["import-model-group", "Benchy group", "current_importing_group"],
    ["import-failure", "disk full", "failure_reason"],
    ["import-model-total", 9, "model_count"],
  ])(
    "%s writes its payload to importState.%s only",
    async (name, payload, field) => {
      await initApi();

      await emit(name, payload);

      expect({ ...importState }).toStrictEqual({
        ...defaultImportState(),
        [field]: payload,
      });
      expect(mocks.goto).not.toHaveBeenCalled();
    },
  );

  it("import-all-data copies every result field, clears the current group and opens the import page", async () => {
    await initApi();
    await emit("import-model-group", "in progress group");
    const payload: ImportState = {
      imported_models: [
        { group_id: 5, group_name: "Benchy", model_ids: [1, 2] },
      ],
      imported_models_count: 2,
      finished_thumbnails_count: 1,
      model_count: 3,
      status: ImportStatus.Finished,
      origin_url: "https://printables/model/1",
      failure_reason: "partial",
      recursive: true,
      delete_after_import: true,
      current_importing_group: "ignored",
    };

    await emit("import-all-data", payload);

    expect({ ...importState }).toStrictEqual({
      ...payload,
      current_importing_group: undefined,
    });
    expect(mocks.goto.mock.calls).toEqual([["/import"]]);
  });
});

describe("remote imports", () => {
  it("deep-link focuses the window, downloads the file and imports it with the link's source url", async () => {
    configuration.open_slicer_on_remote_model_import = true;
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("deep-link", {
      download_url: "https://host/files/benchy.stl?token=1",
      source_url: "https://printables/model/1",
    });
    await vi.waitFor(() =>
      expect(importState.status).toBe(ImportStatus.Finished),
    );

    expect(mocks.unminimize).toHaveBeenCalledTimes(1);
    expect(mocks.setFocus).toHaveBeenCalledTimes(1);
    expect(mocks.toastPromise.mock.calls).toEqual([
      [
        expect.any(Promise),
        {
          loading: "Downloading model benchy.stl",
          success: "Downloaded model benchy.stl",
          error: "Failed to download model benchy.stl",
        },
      ],
    ]);
    expect(startImport.mock.calls).toEqual([
      [
        ["/downloads/benchy.stl"],
        {
          source_url: "https://printables/model/1",
          direct_open_in_slicer: true,
          delete_after_import: true,
          recursive: false,
          import_as_path: false,
        },
      ],
    ]);
    expect(mocks.invoke.mock.calls).toEqual([
      [
        "download_file",
        { url: "https://host/files/benchy.stl?token=1", filename: null },
      ],
      [
        "add_model",
        {
          path: "/downloads/benchy.stl",
          recursive: false,
          deleteImported: true,
          originUrl: "https://printables/model/1",
          openInSlicer: true,
          importAsPath: false,
        },
      ],
    ]);
    expect(mocks.goto.mock.calls).toEqual([["/import"]]);
  });

  it("deep-link without a source url falls back to the download's source uri and skips focusing when disabled", async () => {
    configuration.focus_after_link_import = false;
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("deep-link", {
      download_url: "https://host/benchy.stl",
      source_url: null,
    });
    await vi.waitFor(() =>
      expect(importState.status).toBe(ImportStatus.Finished),
    );

    expect(mocks.unminimize).not.toHaveBeenCalled();
    expect(mocks.setFocus).not.toHaveBeenCalled();
    expect(startImport.mock.calls[0][1].source_url).toBe("https://cdn/benchy");
    expect(addModelCalls()[0][1].originUrl).toBe("https://cdn/benchy");
  });

  it("deep-link with no source url anywhere imports with a null origin url", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "download_file"
        ? { path: "/downloads/benchy.stl", source_uri: null }
        : finishedImport,
    );
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("deep-link", {
      download_url: "https://host/benchy.stl",
      source_url: null,
    });
    await vi.waitFor(() =>
      expect(importState.status).toBe(ImportStatus.Finished),
    );

    expect(startImport.mock.calls[0][1].source_url).toBeUndefined();
    expect(addModelCalls()[0][1].originUrl).toBeNull();
  });

  it("built-in browser download resolves the pending toast and imports with the download url as source", async () => {
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("download-started", "https://host/files/cube.3mf?x=1");
    const [[pendingToast, toastMessages]] = mocks.toastPromise.mock.calls;
    await emit("download-finished", {
      path: "/downloads/cube.3mf",
      url: "https://host/files/cube.3mf?x=1",
    });
    await vi.waitFor(() =>
      expect(importState.status).toBe(ImportStatus.Finished),
    );

    await expect(pendingToast).resolves.toBeNull();
    expect(toastMessages).toEqual({
      loading: "Downloading model cube.3mf",
      success: "Downloaded model cube.3mf",
      error: "Failed to download model cube.3mf",
    });
    expect(mocks.unminimize).toHaveBeenCalledTimes(1);
    expect(mocks.setFocus).toHaveBeenCalledTimes(1);
    expect(startImport.mock.calls).toEqual([
      [
        ["/downloads/cube.3mf"],
        {
          source_url: "https://host/files/cube.3mf?x=1",
          direct_open_in_slicer: false,
          delete_after_import: true,
          recursive: false,
          import_as_path: false,
        },
      ],
    ]);
    expect(addModelCalls()).toEqual([
      [
        "add_model",
        {
          path: "/downloads/cube.3mf",
          recursive: false,
          deleteImported: true,
          originUrl: "https://host/files/cube.3mf?x=1",
          openInSlicer: false,
          importAsPath: false,
        },
      ],
    ]);
    expect(mocks.goto.mock.calls).toEqual([["/import"]]);
  });

  it("download-finished without a started download still imports", async () => {
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("download-finished", {
      path: "/downloads/cube.3mf",
      url: "https://host/cube.3mf",
    });
    await vi.waitFor(() =>
      expect(importState.status).toBe(ImportStatus.Finished),
    );

    expect(mocks.toastPromise).not.toHaveBeenCalled();
    expect(startImport).toHaveBeenCalledTimes(1);
  });

  it("Downloads folder watch imports only new model files, without deleting or a source url", async () => {
    configuration.watch_downloads_folder = true;
    configuration.open_slicer_on_remote_model_import = true;
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await watchCallback!({
      type: { create: { kind: "any" } },
      paths: ["/dl/a.STL", "/dl/readme.txt", "/dl/b.3mf", "/dl/c.zip"],
      attrs: {},
    });

    expect(startImport.mock.calls).toEqual([
      [
        ["/dl/a.STL", "/dl/b.3mf"],
        {
          direct_open_in_slicer: true,
          delete_after_import: false,
          recursive: false,
          import_as_path: false,
        },
      ],
    ]);
    expect(addModelCalls().map(([, args]) => args)).toEqual(
      ["/dl/a.STL", "/dl/b.3mf"].map((path) => ({
        path,
        recursive: false,
        deleteImported: false,
        originUrl: null,
        openInSlicer: true,
        importAsPath: false,
      })),
    );
    expect(mocks.toastPromise.mock.calls).toEqual([
      [
        expect.any(Promise),
        {
          loading: "Importing 2 new file(s) from Downloads folder...",
          success: "Imported 2 new file(s) from Downloads folder.",
          error: "Failed to import new file(s) from Downloads folder.",
        },
      ],
    ]);
    expect(mocks.goto.mock.calls).toEqual([["/import"]]);
  });

  it("Downloads folder watch accepts modify-any events for every model extension", async () => {
    configuration.watch_downloads_folder = true;
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");
    const paths = [
      "/dl/a.stl",
      "/dl/b.obj",
      "/dl/c.3mf",
      "/dl/d.gcode",
      "/dl/e.step",
    ];

    await watchCallback!({
      type: { modify: { kind: "any" } },
      paths,
      attrs: {},
    });

    expect(startImport.mock.calls[0][0]).toEqual(paths);
  });

  it.each([
    ["remove events", { remove: { kind: "any" } }, ["/dl/a.stl"]],
    ["the bare 'any' kind", "any", ["/dl/a.stl"]],
    [
      "non-model files",
      { create: { kind: "any" } },
      ["/dl/a.txt", "/dl/b.zip"],
    ],
  ])("Downloads folder watch ignores %s", async (_name, type, paths) => {
    configuration.watch_downloads_folder = true;
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await watchCallback!({ type, paths, attrs: {} } as WatchEvent);

    expect(startImport).not.toHaveBeenCalled();
    expect(mocks.toastPromise).not.toHaveBeenCalled();
    expect(mocks.goto).not.toHaveBeenCalled();
  });
});

describe("drag-drop and account-link handlers", () => {
  it("drag-drop opens the import page and imports the dropped paths with default settings", async () => {
    const api = await initApi();
    const startImport = vi.spyOn(api, "startImportProcess");

    await emit("tauri://drag-drop", { paths: ["/a.stl", "/b.obj"] });

    expect(mocks.goto.mock.calls).toEqual([["/import"]]);
    expect(startImport.mock.calls).toEqual([[["/a.stl", "/b.obj"], {}]]);
  });

  it.each([undefined, {}, { paths: [] }])(
    "drag-drop with payload %j does nothing",
    async (payload) => {
      const api = await initApi();
      const startImport = vi.spyOn(api, "startImportProcess");

      await emit("tauri://drag-drop", payload);

      expect(startImport).not.toHaveBeenCalled();
      expect(mocks.goto).not.toHaveBeenCalled();
    },
  );

  it("account-link focuses the window and shows the link ui with the emitted account", async () => {
    await initApi();

    await emit("account-link", {
      base_url: "https://mesh.example",
      user_name: "alice",
      link_token: "token-1",
    });

    expect(mocks.setFocus).toHaveBeenCalledTimes(1);
    expect({ ...accountLinkData }).toStrictEqual({
      showLinkUi: true,
      baseUrl: "https://mesh.example",
      userName: "alice",
      linkToken: "token-1",
    });
  });
});
