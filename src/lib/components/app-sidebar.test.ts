// @vitest-environment jsdom
import { render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Label, LabelMeta } from "$lib/api/shared/label_api";
import { defaultSidebarState } from "$lib/api/shared/sidebar_state_api";
import { sidebarState } from "$lib/sidebar_data.svelte";
import AppSidebar from "./app-sidebar.svelte";

const page = vi.hoisted(() => {
  // jsdom has no matchMedia; svelte/reactivity touches it at import time.
  window.matchMedia ??= (query: string) =>
    ({
      matches: false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    }) as unknown as MediaQueryList;

  return { url: new URL("http://localhost/") };
});

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page }));

const ACTIVE_CLASS = "border-secondary";

function meta(id: number, name: string): LabelMeta {
  return {
    id,
    name,
    color: "#ff0000",
    lastModified: new Date(0),
    uniqueGlobalId: `label-${id}`,
  };
}

function entry(
  labelMeta: LabelMeta,
  children: LabelMeta[] = [],
  hasParent = false,
): Label {
  return {
    meta: labelMeta,
    children,
    effectiveLabels: [labelMeta, ...children],
    hasParent,
    modelCount: 5,
    groupCount: 0,
    selfModelCount: 2,
    selfGroupCount: 0,
  };
}

const leaf = meta(1, "Tools");
const parent = meta(2, "Hardware");
const child = meta(3, "Bolts");

// Stand-in for Sidebar.Provider's SidebarState context (desktop, expanded).
const sidebarContext = new Map([
  [
    Symbol.for("scn-sidebar"),
    {
      open: true,
      openMobile: false,
      isMobile: false,
      state: "expanded",
      setOpen: vi.fn(),
      setOpenMobile: vi.fn(),
      toggle: vi.fn(),
      handleShortcutKeydown: vi.fn(),
    },
  ],
]);

function renderAt(path: string) {
  page.url = new URL(`http://localhost${path}`);
  Object.assign(sidebarState, {
    ...defaultSidebarState(),
    labels: [entry(leaf), entry(parent, [child]), entry(child, [], true)],
  });

  return render(AppSidebar, { context: sidebarContext });
}

function anchorFor(name: string, href: string): HTMLAnchorElement {
  const anchor = screen
    .getAllByText(name)
    .map((span) => span.closest("a"))
    .find((a) => a?.getAttribute("href") === href);
  expect(anchor, `anchor ${name} -> ${href}`).toBeTruthy();

  return anchor!;
}

describe("AppSidebar LabelTree", () => {
  afterEach(() => {
    Object.assign(sidebarState, defaultSidebarState());
  });

  it("renders top-level leaf and parent labels linking to /label/<id>, parent collapsed", () => {
    renderAt("/model");

    expect(anchorFor("Tools", "/label/1").className).not.toContain(
      ACTIVE_CLASS,
    );
    expect(anchorFor("Hardware", "/label/2").className).not.toContain(
      ACTIVE_CLASS,
    );
    // The child only renders nested under its parent (hasParent skips the top level),
    // inside the collapsed Collapsible.Content.
    expect(screen.getAllByText("Bolts")).toHaveLength(1);
    expect(
      anchorFor("Bolts", "/label/3?parentId=2").closest(
        '[data-state="closed"]',
      ),
    ).not.toBeNull();
  });

  it("marks the leaf label active on its own url", () => {
    renderAt("/label/1");

    expect(anchorFor("Tools", "/label/1").className).toContain(ACTIVE_CLASS);
    expect(anchorFor("Hardware", "/label/2").className).not.toContain(
      ACTIVE_CLASS,
    );
  });

  it("marks the parent label active on its url without thisLabelOnly", () => {
    renderAt("/label/2");

    expect(anchorFor("Hardware", "/label/2").className).toContain(ACTIVE_CLASS);
  });

  it("marks the 'this label only' entry active, not the parent, with thisLabelOnly", () => {
    renderAt("/label/2?thisLabelOnly=true");

    expect(
      anchorFor("Hardware", "/label/2?thisLabelOnly=true").className,
    ).toContain(ACTIVE_CLASS);
    expect(anchorFor("Hardware", "/label/2").className).not.toContain(
      ACTIVE_CLASS,
    );
  });

  it("expands the parent when a child is current: child link carries parentId and is active", () => {
    renderAt("/label/3");

    const childAnchor = anchorFor("Bolts", "/label/3?parentId=2");
    expect(childAnchor.className).toContain(ACTIVE_CLASS);
    expect(childAnchor.closest('[data-state="closed"]')).toBeNull();
    // Parent with own models gets a "this label only" entry.
    expect(
      anchorFor("Hardware", "/label/2?thisLabelOnly=true").className,
    ).not.toContain(ACTIVE_CLASS);
    expect(anchorFor("Hardware", "/label/2").className).not.toContain(
      ACTIVE_CLASS,
    );
  });
});
