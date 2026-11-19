// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import {
  createGroupInstance,
  createGroupMetaInstance,
} from "#lib/api/shared/group_api";
import GroupTiny from "./group-tiny.svelte";

afterEach(cleanup);

it("keeps a long group name accessible while constraining its title to the card", () => {
  const name =
    "A long group name that must not overflow into neighbouring cards";
  const timestamp = "2026-10-24T12:00:00Z";
  const group = createGroupInstance(
    createGroupMetaInstance(7, name, timestamp, timestamp, "group-7"),
    [],
    [],
    null,
    [],
  );
  render(GroupTiny, { group, class: "w-40" });
  const title = screen.getByRole("heading", { name });
  expect(title.textContent?.trim()).toBe(name);
  // jsdom cannot lay out text: guard the fixed-width class that caused the overflow.
  expect(title.classList.contains("w-100")).toBe(false);
  expect(title.classList.contains("truncate")).toBe(true);
});
