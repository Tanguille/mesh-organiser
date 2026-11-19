// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { ISyncApi } from "#lib/api/shared/sync_api";
import { resetSyncState } from "#lib/sync.svelte";
import SyncProgressIndicator from "./sync-progress-indicator.svelte";

const toastError = vi.hoisted(() => vi.fn());
vi.mock("svelte-sonner", () => ({ toast: { error: toastError } }));

afterEach(() => {
  cleanup();
  resetContainer();
  resetSyncState();
  vi.clearAllMocks();
});

it("shows upgrade guidance when sync is unavailable", async () => {
  const message = "Sync is unavailable. Upgrade the server and retry.";
  const syncData = vi.fn().mockRejectedValue(new Error(message));
  getContainer().addSingleton(ISyncApi, { syncData });
  render(SyncProgressIndicator);

  await fireEvent.click(screen.getByRole("button", { name: /sync now/i }));

  await waitFor(() => expect(toastError).toHaveBeenCalledWith(message));
  expect(syncData).toHaveBeenCalledOnce();
});

it("runs a successful sync without an error message", async () => {
  const syncData = vi.fn().mockResolvedValue(undefined);
  getContainer().addSingleton(ISyncApi, { syncData });
  render(SyncProgressIndicator);

  await fireEvent.click(screen.getByRole("button", { name: /sync now/i }));

  expect(syncData).toHaveBeenCalledOnce();
  expect(toastError).not.toHaveBeenCalled();
});
