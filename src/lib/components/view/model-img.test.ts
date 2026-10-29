// @vitest-environment jsdom
import { render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import { IBlobApi, createBlobInstance } from "#lib/api/shared/blob_api";
import { createModelInstance } from "#lib/api/shared/model_api";
import ModelImg from "./model-img.svelte";

const timestamp = "2026-10-29T12:00:00Z";

// Dev builds wrap `await` in reactivity-loss tracking, which adds microtasks, so wait a macrotask.
async function settle() {
  await new Promise((resolve) => setTimeout(resolve));
  await tick();
}

function model(id: number) {
  return createModelInstance(
    id,
    `Part ${id}`,
    createBlobInstance(id, `hash-${id}`, "stl", 1024, timestamp),
    null,
    null,
    timestamp,
    timestamp,
    null,
    [],
    [],
    `model-${id}`,
  );
}

describe("ModelImg", () => {
  afterEach(() => {
    resetContainer();
  });

  it("shows the latest model's thumbnail when an older request resolves last", async () => {
    const resolvers: ((url: string) => void)[] = [];
    getContainer().addSingleton(IBlobApi, {
      getBlobThumbnailUrl: vi.fn(
        () => new Promise<string>((resolve) => resolvers.push(resolve)),
      ),
    });
    const { rerender } = render(ModelImg, { model: model(1) });

    rerender({ model: model(2) });
    expect(resolvers).toHaveLength(2);
    resolvers[1]("http://localhost/thumb-2.png");
    await settle();
    resolvers[0]("http://localhost/thumb-1.png");
    await settle();

    expect(screen.getByRole("img").getAttribute("src")).toBe(
      "http://localhost/thumb-2.png",
    );
  });
});
