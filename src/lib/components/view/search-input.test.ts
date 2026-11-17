// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import SearchInput from "./search-input.svelte";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("gives the search field an accessible name", () => {
  render(SearchInput, { onsearch: vi.fn() });
  expect(
    screen.queryByRole("searchbox", { name: /search/i }) ??
      screen.queryByRole("textbox", { name: /search/i }),
  ).not.toBeNull();
});

it("debounces ordinary searches for 200 milliseconds", async () => {
  const onsearch = vi.fn();
  render(SearchInput, { onsearch });
  await fireEvent.input(screen.getByPlaceholderText("Search"), {
    target: { value: "bolts" },
  });
  vi.advanceTimersByTime(199);
  expect(onsearch).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(onsearch).toHaveBeenCalledExactlyOnceWith("bolts");
});

it("clears immediately and cancels an earlier pending search", async () => {
  const onsearch = vi.fn();
  render(SearchInput, { onsearch });
  const input = screen.getByPlaceholderText("Search") as HTMLInputElement;
  await fireEvent.input(input, { target: { value: "bolts" } });
  vi.advanceTimersByTime(50);
  const clear = screen.queryByRole("button", { name: /clear search/i });
  expect(clear).not.toBeNull();
  await fireEvent.click(clear!);
  expect(input.value).toBe("");
  expect(onsearch).toHaveBeenCalledExactlyOnceWith(null);
  vi.advanceTimersByTime(500);
  expect(onsearch).toHaveBeenCalledTimes(1);
});
