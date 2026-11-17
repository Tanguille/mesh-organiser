// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { describe, expect, it, vi } from "vitest";
import AsyncButton from "./async-button.svelte";

const children = createRawSnippet(() => ({ render: () => "<span>Run</span>" }));

describe("AsyncButton", () => {
  it("disables while an action is pending and re-enables after success", async () => {
    let complete!: () => void;
    const onclick = vi.fn(
      () => new Promise<void>((resolve) => (complete = resolve)),
    );
    render(AsyncButton, { children, onclick });
    const button = screen.getByRole("button", {
      name: "Run",
    }) as HTMLButtonElement;

    await fireEvent.click(button);
    expect(button.disabled).toBe(true);
    complete();
    await waitFor(() => expect(button.disabled).toBe(false));
  });

  it("re-enables after a rejected action so it can be retried", async () => {
    const failure = new Error("action failed");
    const onclick = vi
      .fn()
      .mockRejectedValueOnce(failure)
      .mockResolvedValueOnce(undefined);
    const listeners = process.listeners("unhandledRejection");
    const unhandledRejection = vi.fn();
    process.removeAllListeners("unhandledRejection");
    process.on("unhandledRejection", unhandledRejection);

    try {
      render(AsyncButton, { children, onclick });
      const button = screen.getByRole("button", {
        name: "Run",
      }) as HTMLButtonElement;
      await fireEvent.click(button);
      await waitFor(() =>
        expect(unhandledRejection).toHaveBeenCalledWith(
          failure,
          expect.anything(),
        ),
      );
      expect(button.disabled).toBe(false);
      await fireEvent.click(button);
      expect(onclick).toHaveBeenCalledTimes(2);
    } finally {
      process.removeAllListeners("unhandledRejection");
      listeners.forEach((listener) =>
        process.on("unhandledRejection", listener),
      );
    }
  });
});
