/**
 * Regression tests for shared raw model types and convertModelFlagsToRaw,
 * after moving this code from tauri-specific to shared (used by web, web_share, tauri).
 */

import { describe, it, expect } from "vitest";
import { FileType } from "./blob_api";
import type { ResourceFlags } from "./resource_api";
import type { ModelFlags } from "./model_api";
import {
  convertModelFlagsToRaw,
  convertResourceFlagsToRaw,
  parseRawBlob,
  parseRawGroup,
  parseRawGroupMeta,
  parseRawLabel,
  parseRawLabelMeta,
  parseRawModel,
  parseRawResourceMeta,
  type RawBlob,
  type RawGroup,
  type RawGroupMeta,
  type RawLabel,
  type RawLabelMeta,
  type RawModel,
  type RawResourceMeta,
} from "./raw_model";

describe("convertModelFlagsToRaw", () => {
  it("returns null when flags is null", () => {
    // Arrange
    const flags: ModelFlags | null = null;
    // Act
    const result = convertModelFlagsToRaw(flags);
    // Assert
    expect(result).toBeNull();
  });

  it("returns null when flags is empty (no printed or favorite)", () => {
    // Arrange
    const flags: ModelFlags = { printed: false, favorite: false };
    // Act
    const result = convertModelFlagsToRaw(flags);
    // Assert
    expect(result).toBeNull();
  });

  it('returns ["Printed"] when printed is true and favorite is false', () => {
    // Arrange
    const flags: ModelFlags = { printed: true, favorite: false };
    // Act
    const result = convertModelFlagsToRaw(flags);
    // Assert
    expect(result).toEqual(["Printed"]);
  });

  it('returns ["Favorite"] when printed is false and favorite is true', () => {
    // Arrange
    const flags: ModelFlags = { printed: false, favorite: true };
    // Act
    const result = convertModelFlagsToRaw(flags);
    // Assert
    expect(result).toEqual(["Favorite"]);
  });

  it('returns array containing both "Printed" and "Favorite" when both are true', () => {
    // Arrange
    const flags: ModelFlags = { printed: true, favorite: true };
    // Act
    const result = convertModelFlagsToRaw(flags);
    // Assert
    expect(result).toHaveLength(2);
    expect(result).toContain("Printed");
    expect(result).toContain("Favorite");
  });
});

describe("convertResourceFlagsToRaw", () => {
  it("returns empty array when completed is false", () => {
    // Arrange
    const flags: ResourceFlags = { completed: false };
    // Act
    const result = convertResourceFlagsToRaw(flags);
    // Assert
    expect(result).toEqual([]);
  });

  it('returns ["Completed"] when completed is true', () => {
    // Arrange
    const flags: ResourceFlags = { completed: true };
    // Act
    const result = convertResourceFlagsToRaw(flags);
    // Assert
    expect(result).toEqual(["Completed"]);
  });
});

describe("parseRawBlob", () => {
  it("maps raw fields and converts filetype and added date", () => {
    // Arrange
    const raw: RawBlob = {
      id: 1,
      sha256: "abc",
      filetype: "stl",
      size: 42,
      added: "2024-01-02T03:04:05Z",
    };
    // Act
    const blob = parseRawBlob(raw);
    // Assert
    expect(blob.id).toBe(1);
    expect(blob.sha256).toBe("abc");
    expect(blob.filetype).toBe(FileType.STL);
    expect(blob.size).toBe(42);
    expect(blob.added).toEqual(new Date("2024-01-02T03:04:05Z"));
  });
});

describe("parseRawGroupMeta", () => {
  it("maps raw fields and converts created/last_modified to dates", () => {
    // Arrange
    const raw: RawGroupMeta = {
      id: 7,
      name: "group",
      created: "2024-01-01T00:00:00Z",
      last_modified: "2024-02-01T00:00:00Z",
      resource_id: null,
      unique_global_id: "gid",
    };
    // Act
    const meta = parseRawGroupMeta(raw);
    // Assert
    expect(meta.id).toBe(7);
    expect(meta.name).toBe("group");
    expect(meta.created).toEqual(new Date("2024-01-01T00:00:00Z"));
    expect(meta.lastModified).toEqual(new Date("2024-02-01T00:00:00Z"));
    expect(meta.uniqueGlobalId).toBe("gid");
  });
});

describe("parseRawLabelMeta", () => {
  it("maps raw fields and converts numeric color to hex string", () => {
    // Arrange
    const raw: RawLabelMeta = {
      id: 3,
      name: "label",
      color: 0xff0000,
      unique_global_id: "lid",
      last_modified: "2024-03-01T00:00:00Z",
    };
    // Act
    const meta = parseRawLabelMeta(raw);
    // Assert
    expect(meta.id).toBe(3);
    expect(meta.name).toBe("label");
    expect(meta.color).toBe("#ff0000");
    expect(meta.lastModified).toEqual(new Date("2024-03-01T00:00:00Z"));
    expect(meta.uniqueGlobalId).toBe("lid");
  });
});

describe("parseRawResourceMeta", () => {
  it("maps raw fields, converts flags and dates", () => {
    // Arrange
    const raw: RawResourceMeta = {
      id: 9,
      name: "resource",
      flags: ["Completed"],
      created: "2024-04-01T00:00:00Z",
      unique_global_id: "rid",
      last_modified: "2024-05-01T00:00:00Z",
    };
    // Act
    const meta = parseRawResourceMeta(raw);
    // Assert
    expect(meta.id).toBe(9);
    expect(meta.name).toBe("resource");
    expect(meta.flags).toEqual({ completed: true });
    expect(meta.created).toEqual(new Date("2024-04-01T00:00:00Z"));
    expect(meta.lastModified).toEqual(new Date("2024-05-01T00:00:00Z"));
    expect(meta.uniqueGlobalId).toBe("rid");
  });
});

describe("parseRawLabel", () => {
  it("parses nested label metas and copies counts", () => {
    // Arrange
    const childMeta: RawLabelMeta = {
      id: 11,
      name: "child",
      color: 0x00ff00,
      unique_global_id: "child-gid",
      last_modified: "2024-06-01T00:00:00Z",
    };
    const raw: RawLabel = {
      meta: {
        id: 10,
        name: "parent",
        color: 0x0000ff,
        unique_global_id: "parent-gid",
        last_modified: "2024-06-02T00:00:00Z",
      },
      children: [childMeta],
      effective_labels: [childMeta],
      has_parent: true,
      model_count: 5,
      group_count: 2,
      self_model_count: 3,
      self_group_count: 1,
    };
    // Act
    const label = parseRawLabel(raw);
    // Assert
    expect(label.meta.color).toBe("#0000ff");
    expect(label.children).toHaveLength(1);
    expect(label.children[0].color).toBe("#00ff00");
    expect(label.effectiveLabels).toHaveLength(1);
    expect(label.hasParent).toBe(true);
    expect(label.modelCount).toBe(5);
    expect(label.groupCount).toBe(2);
    expect(label.selfModelCount).toBe(3);
    expect(label.selfGroupCount).toBe(1);
  });
});

const sampleRawBlob: RawBlob = {
  id: 100,
  sha256: "sha",
  filetype: "stl",
  size: 1024,
  added: "2024-07-01T00:00:00Z",
};

const sampleRawModel: RawModel = {
  id: 50,
  name: "model",
  blob: sampleRawBlob,
  link: "https://example.com",
  description: "a model",
  added: "2024-07-02T00:00:00Z",
  last_modified: "2024-07-03T00:00:00Z",
  group: null,
  labels: [],
  flags: ["Printed"],
  unique_global_id: "model-gid",
};

describe("parseRawModel", () => {
  it("maps raw fields, parses blob and converts flags", () => {
    // Act
    const model = parseRawModel(sampleRawModel);
    // Assert
    expect(model.id).toBe(50);
    expect(model.name).toBe("model");
    expect(model.blob.filetype).toBe(FileType.STL);
    expect(model.link).toBe("https://example.com");
    expect(model.description).toBe("a model");
    expect(model.added).toEqual(new Date("2024-07-02T00:00:00Z"));
    expect(model.lastModified).toEqual(new Date("2024-07-03T00:00:00Z"));
    expect(model.group).toBeNull();
    expect(model.labels).toEqual([]);
    expect(model.flags).toEqual({ printed: true, favorite: false });
    expect(model.uniqueGlobalId).toBe("model-gid");
  });
});

describe("parseRawGroup", () => {
  it("parses meta, models and resource, converts group flags", () => {
    // Arrange
    const raw: RawGroup = {
      meta: {
        id: 60,
        name: "group",
        created: "2024-08-01T00:00:00Z",
        last_modified: "2024-08-02T00:00:00Z",
        resource_id: 70,
        unique_global_id: "group-gid",
      },
      models: [sampleRawModel],
      labels: [],
      resource: {
        id: 70,
        name: "resource",
        flags: [],
        created: "2024-08-03T00:00:00Z",
        unique_global_id: "res-gid",
        last_modified: "2024-08-04T00:00:00Z",
      },
      flags: ["Favorite"],
    };
    // Act
    const group = parseRawGroup(raw);
    // Assert
    expect(group.meta.id).toBe(60);
    expect(group.models).toHaveLength(1);
    expect(group.models[0].id).toBe(50);
    // createGroupInstance assigns the group meta back onto each model
    expect(group.models[0].group).toBe(group.meta);
    expect(group.resource?.name).toBe("resource");
    expect(group.flags).toEqual({ printed: false, favorite: true });
  });
});
