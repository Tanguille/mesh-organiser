// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { resetContainer, getContainer } from "$lib/api/dependency_injection";
import { IInternalBrowserApi } from "$lib/api/shared/internal_browser_api";
import { configuration } from "$lib/configuration.svelte";
import { buttonVariants } from "$lib/components/ui/button/index.js";
import LinkButton from "./link-button.svelte";

const LINK = "https://example.com/model";

describe("LinkButton", () => {
  afterEach(() => {
    resetContainer();
    configuration.open_links_in_external_browser = false;
  });

  it("renders an external anchor with 'Open Link' and default variant without an internal browser", () => {
    render(LinkButton, { link: LINK });

    const anchor = screen.getByRole("link");
    expect(anchor.getAttribute("href")).toBe(LINK);
    expect(anchor.getAttribute("target")).toBe("_blank");
    expect(anchor.textContent).toContain("Open Link");
    expect(anchor.className).toContain(buttonVariants({ variant: "default" }));
  });

  it("hides the text when withText is false", () => {
    render(LinkButton, { link: LINK, withText: false });

    expect(screen.getByRole("link").textContent).not.toContain("Open Link");
  });

  it("applies the requested variant", () => {
    render(LinkButton, { link: LINK, variant: "outline" });

    expect(screen.getByRole("link").className).toContain(
      buttonVariants({ variant: "outline" }),
    );
  });

  it("renders a button that opens the internal browser when one is available", async () => {
    const openInternalBrowser = vi.fn().mockResolvedValue(undefined);
    getContainer().addSingleton(IInternalBrowserApi, { openInternalBrowser });

    render(LinkButton, { link: LINK });

    expect(screen.queryByRole("link")).toBeNull();
    const button = screen.getByRole("button", { name: "Open Link" });
    await fireEvent.click(button);
    expect(openInternalBrowser).toHaveBeenCalledWith(LINK);
  });

  it("uses the external anchor when open_links_in_external_browser is set", () => {
    getContainer().addSingleton(IInternalBrowserApi, {
      openInternalBrowser: vi.fn(),
    });
    configuration.open_links_in_external_browser = true;

    render(LinkButton, { link: LINK });

    expect(screen.getByRole("link").getAttribute("href")).toBe(LINK);
  });

  it("renders nothing without a link", () => {
    const { container } = render(LinkButton, { link: null });

    expect(container.textContent?.trim()).toBe("");
  });

  it("renders a disabled fallback button without a link when withFallback is set", () => {
    render(LinkButton, { link: null, withFallback: true });

    const button = screen.getByRole("button", { name: "Open Link" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders the anchor when visible is forced even without a link", () => {
    render(LinkButton, { link: null, visible: true });

    // An <a> without href has no implicit link role.
    expect(screen.getByText("Open Link").closest("a")).not.toBeNull();
  });
});
