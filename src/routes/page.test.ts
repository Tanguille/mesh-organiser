// @vitest-environment jsdom
import { render } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { configuration } from "$lib/configuration.svelte";
import { configurationDefault } from "$lib/api/shared/settings_api";
import Page from "./+page.svelte";

const goto = vi.hoisted(() => vi.fn());

vi.mock("$app/navigation", () => ({ goto }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => `/base${path}` }));

describe("root +page startup redirect", () => {
  afterEach(() => {
    goto.mockReset();
    configuration.startup_page = configurationDefault().startup_page;
  });

  it.each([
    ["models", "/model"],
    ["import", "/import"],
    ["groups", "/group"],
    ["favorites", "/favorite"],
    ["print-history", "/printed"],
    ["projects", "/resource"],
  ] as const)("startup_page %s redirects to %s", (startupPage, route) => {
    configuration.startup_page = startupPage;

    render(Page);

    expect(goto).toHaveBeenCalledExactlyOnceWith(`/base${route}`);
  });

  it("does not redirect and renders the app header for the default startup page", () => {
    const { getByText } = render(Page);

    expect(goto).not.toHaveBeenCalled();
    expect(getByText("Mesh Organiser")).toBeTruthy();
  });
});
