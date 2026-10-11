// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "$lib/api/dependency_injection";
import { IResourceApi, type ResourceMeta } from "$lib/api/shared/resource_api";
import { ILabelApi, type Label } from "$lib/api/shared/label_api";
import EditResource from "./resource.svelte";
import EditLabel from "./label.svelte";

const goto = vi.hoisted(() => vi.fn());

vi.mock("$app/navigation", () => ({ goto }));
vi.mock("$app/paths", () => ({ resolve: (path: string) => path }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/") } }));

// Header action menu: Ellipsis DropdownMenu.Trigger -> single DropdownMenu.Item.
async function openHeaderMenu(container: HTMLElement) {
  const trigger = container.querySelector<HTMLElement>(
    '[aria-haspopup="menu"]',
  );
  expect(trigger).not.toBeNull();
  await fireEvent.pointerDown(trigger!, { button: 0, pointerType: "mouse" });
}

describe("edit card header action menu", () => {
  afterEach(() => {
    resetContainer();
    goto.mockReset();
  });

  it("resource: 'Delete project' deletes the resource and calls onDelete", async () => {
    const resourceApi = {
      deleteResource: vi.fn().mockResolvedValue(undefined),
    };
    getContainer().addSingleton(IResourceApi, resourceApi);
    const resource = {
      id: 7,
      name: "Printer upgrade",
      flags: { completed: false },
      created: new Date(0),
      lastModified: new Date(0),
      uniqueGlobalId: "resource-7",
    } as ResourceMeta;
    const onDelete = vi.fn();

    const { container } = render(EditResource, { resource, onDelete });
    expect(screen.getByText("Project 'Printer upgrade'")).toBeTruthy();
    await openHeaderMenu(container);
    await fireEvent.click(await screen.findByText("Delete project"));

    expect(resourceApi.deleteResource).toHaveBeenCalledWith(resource);
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith(resource));
  });

  it("label: 'Delete label' deletes the label and calls onDelete", async () => {
    const labelApi = {
      deleteLabel: vi.fn().mockResolvedValue(undefined),
      getKeywordsForLabel: vi.fn().mockResolvedValue([]),
    };
    getContainer().addSingleton(ILabelApi, labelApi);
    const label = {
      meta: {
        id: 3,
        name: "Tools",
        color: "#ff0000",
        lastModified: new Date(0),
        uniqueGlobalId: "label-3",
      },
      children: [],
      effectiveLabels: [],
      hasParent: false,
      modelCount: 0,
      groupCount: 0,
      selfModelCount: 0,
      selfGroupCount: 0,
    } as Label;
    const onDelete = vi.fn();

    const { container } = render(EditLabel, { label, onDelete });
    await openHeaderMenu(container);
    await fireEvent.click(await screen.findByText("Delete label"));

    expect(labelApi.deleteLabel).toHaveBeenCalledWith(label.meta);
    await waitFor(() => expect(onDelete).toHaveBeenCalled());
    // No ?parentId in the url, so no navigation after delete.
    expect(goto).not.toHaveBeenCalled();
  });
});
