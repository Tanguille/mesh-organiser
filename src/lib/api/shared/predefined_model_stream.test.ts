import { describe, expect, it } from "vitest";
import { createBlobInstance } from "./blob_api";
import {
  createModelInstance,
  ModelOrderBy,
  PredefinedModelStreamManager,
} from "./model_api";

describe("PredefinedModelStreamManager", () => {
  it("continues paging without skipping models removed from an earlier page", async () => {
    const timestamp = "2026-10-24T12:00:00Z";
    const models = ["A", "B", "C", "D"].map((name, index) =>
      createModelInstance(
        index + 1,
        name,
        createBlobInstance(
          index + 1,
          `hash-${index}`,
          "stl.zip",
          1024,
          timestamp,
        ),
        null,
        null,
        timestamp,
        timestamp,
        null,
        [],
        [],
        `model-${index}`,
      ),
    );
    const stream = new PredefinedModelStreamManager(models, 2);
    stream.setOrderBy(ModelOrderBy.NameAsc);

    const first = await stream.fetch();
    expect(first.map((model) => model.name)).toEqual(["A", "B"]);
    stream.removeModels([first[0]]);

    expect((await stream.fetch()).map((model) => model.name)).toEqual([
      "C",
      "D",
    ]);
    expect(await stream.fetch()).toEqual([]);
  });
});
