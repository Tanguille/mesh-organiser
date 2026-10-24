// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { resetContainer, getContainer } from "#lib/api/dependency_injection";
import { IInternalBrowserApi } from "#lib/api/shared/internal_browser_api";
import { configuration } from "#lib/configuration.svelte";
import { buttonVariants } from "#lib/components/ui/button/index.js";
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

  it.each([
    ["  https://example.com/model  ", LINK],
    ["www.printables.com/model/1", "https://www.printables.com/model/1"],
    ["printables.com/x", "https://printables.com/x"],
    [" printables.com ", "https://printables.com"],
  ])("links %j to %s", (rawLink, expectedHref) => {
    render(LinkButton, { link: rawLink });

    expect(screen.getByRole("link").getAttribute("href")).toBe(expectedHref);
  });

  it("opens a bare domain in the internal browser over https", async () => {
    const openInternalBrowser = vi.fn().mockResolvedValue(undefined);
    getContainer().addSingleton(IInternalBrowserApi, { openInternalBrowser });

    render(LinkButton, { link: " www.printables.com/model/1 " });
    await fireEvent.click(screen.getByRole("button", { name: "Open Link" }));

    expect(openInternalBrowser).toHaveBeenCalledWith(
      "https://www.printables.com/model/1",
    );
  });

  describe.each([
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "file:///etc/passwd",
    "not a link",
    "   ",
  ])("with the unusable link %j", (unusableLink) => {
    it("renders nothing by default", () => {
      const { container } = render(LinkButton, { link: unusableLink });

      expect(container.textContent?.trim()).toBe("");
    });

    it("renders nothing even when visible is forced", () => {
      const { container } = render(LinkButton, {
        link: unusableLink,
        visible: true,
      });

      expect(container.textContent?.trim()).toBe("");
    });

    it("renders no button that could open the internal browser when visible is forced", () => {
      const openInternalBrowser = vi.fn().mockResolvedValue(undefined);
      getContainer().addSingleton(IInternalBrowserApi, {
        openInternalBrowser,
      });

      render(LinkButton, { link: unusableLink, visible: true });

      expect(screen.queryByRole("button")).toBeNull();
      expect(screen.queryByRole("link")).toBeNull();
    });

    it("renders only the disabled fallback button when withFallback is set", () => {
      render(LinkButton, {
        link: unusableLink,
        visible: true,
        withFallback: true,
      });

      const button = screen.getByRole("button", { name: "Open Link" });
      expect((button as HTMLButtonElement).disabled).toBe(true);
      expect(screen.getByText("Open Link").closest("a")).toBeNull();
    });
  });

  it("renders nothing when visible is forced without a link", () => {
    const { container } = render(LinkButton, { link: null, visible: true });

    expect(container.textContent?.trim()).toBe("");
  });
});
