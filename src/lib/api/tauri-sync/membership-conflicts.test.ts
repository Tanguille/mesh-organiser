import { describe, expect, it } from "vitest";
import {
  planMemberships,
  resolveMembershipChoice,
} from "./membership-conflicts";

function item(modelId: string, groupId: string | null, revision = 1) {
  return {
    modelId,
    groupId,
    incarnation: `incarnation-${modelId}`,
    revision,
    deletedAt: null,
  };
}

describe("three-way membership planning", () => {
  it.each([
    ["equal unchanged", "a", "a", "a", "a"],
    ["same concurrent move", "a", "b", "b", "b"],
    ["local move", "a", "b", "a", "b"],
    ["remote move", "a", "a", "b", "b"],
    ["local ungroup", "a", null, "a", null],
    ["remote ungroup", "a", "a", null, null],
    ["move from ungrouped", null, null, "b", "b"],
    ["equal ungrouped", null, null, null, null],
  ])("resolves %s", (_description, base, local, remote, expected) => {
    const plan = planMemberships(
      new Map([["model", base]]),
      [item("model", local)],
      [item("model", remote)],
    );
    expect(plan.resolved).toEqual([{ modelId: "model", groupId: expected }]);
    expect(plan.conflicts).toEqual([]);
    expect(plan.blocked).toEqual([]);
  });

  it.each([
    ["different group moves", "a", "b", "c"],
    ["move versus ungroup", "a", "b", null],
    ["two moves from ungrouped", null, "a", "b"],
  ])("requests a choice for %s", (_description, base, local, remote) => {
    const plan = planMemberships(
      new Map([["model", base]]),
      [item("model", local)],
      [item("model", remote)],
    );
    expect(plan.resolved).toEqual([]);
    expect(plan.blocked).toEqual([]);
    expect(plan.conflicts).toEqual([
      {
        modelId: "model",
        base,
        local: item("model", local),
        remote: item("model", remote),
      },
    ]);
    const conflict = plan.conflicts[0];
    expect(
      resolveMembershipChoice(
        conflict,
        local,
        item("model", local),
        item("model", remote),
      ),
    ).toEqual({ modelId: "model", groupId: local });
    expect(
      resolveMembershipChoice(
        conflict,
        remote,
        item("model", local),
        item("model", remote),
      ),
    ).toEqual({ modelId: "model", groupId: remote });
    const ungrouped = resolveMembershipChoice(
      conflict,
      null,
      item("model", local),
      item("model", remote),
    );
    if (local === null || remote === null)
      expect(ungrouped).toEqual({ modelId: "model", groupId: null });
    else expect(ungrouped).toBeNull();
  });

  it("does not treat an unknown baseline as ungrouped", () => {
    const local = item("model", null);
    const remote = item("model", "b");
    const plan = planMemberships(new Map(), [local], [remote]);
    expect(plan.resolved).toEqual([]);
    expect(plan.conflicts).toEqual([
      { modelId: "model", base: undefined, local, remote },
    ]);
    expect(plan.blocked).toEqual([]);
  });

  it("accepts equal current assignments even without a baseline", () => {
    const plan = planMemberships(
      new Map(),
      [item("model", "a")],
      [item("model", "a")],
    );
    expect(plan.resolved).toEqual([{ modelId: "model", groupId: "a" }]);
    expect(plan.conflicts).toEqual([]);
  });

  it.each(["local", "remote"])(
    "blocks duplicate %s model identities",
    (side) => {
      const local = [item("model", "a")];
      const remote = [item("model", "b")];
      (side === "local" ? local : remote).push(item("model", "c"));
      const plan = planMemberships(new Map([["model", "a"]]), local, remote);
      expect(plan.resolved).toEqual([]);
      expect(plan.conflicts).toEqual([]);
      expect(plan.blocked).toEqual([
        { modelId: "model", reason: "duplicate-identity" },
      ]);
    },
  );

  it.each(["local", "remote"])(
    "blocks a model missing from the %s snapshot",
    (side) => {
      const plan = planMemberships(
        new Map([["model", "a"]]),
        side === "local" ? [] : [item("model", "a")],
        side === "remote" ? [] : [item("model", "a")],
      );
      expect(plan.resolved).toEqual([]);
      expect(plan.conflicts).toEqual([]);
      expect(plan.blocked).toEqual([
        { modelId: "model", reason: "missing-model" },
      ]);
    },
  );

  it.each([
    ["empty model id", new Map(), [item("", "a")], [item("", "a")]],
    [
      "empty local group id",
      new Map(),
      [item("model", "")],
      [item("model", "a")],
    ],
    [
      "empty remote group id",
      new Map(),
      [item("model", "a")],
      [item("model", "")],
    ],
    [
      "empty baseline group id",
      new Map([["model", ""]]),
      [item("model", "a")],
      [item("model", "a")],
    ],
  ])("blocks %s", (_description, base, local, remote) => {
    const plan = planMemberships(base, local, remote);
    expect(plan.resolved).toEqual([]);
    expect(plan.conflicts).toEqual([]);
    expect(plan.blocked).toHaveLength(1);
    expect(plan.blocked[0].reason).toBe("invalid-identity");
  });

  it("rejects a choice when either snapshot or the model incarnation has changed", () => {
    const local = item("model", "b");
    const remote = item("model", "c");
    const conflict = planMemberships(
      new Map([["model", "a"]]),
      [local],
      [remote],
    ).conflicts[0];
    for (const changed of [
      { revision: 2 },
      { incarnation: "replacement" },
      { groupId: "different" },
      { deletedAt: "2026-01-01T00:00:00Z" },
      { modelId: "different" },
    ]) {
      expect(
        resolveMembershipChoice(
          conflict,
          "b",
          { ...local, ...changed },
          remote,
        ),
      ).toBeNull();
      expect(
        resolveMembershipChoice(conflict, "b", local, {
          ...remote,
          ...changed,
        }),
      ).toBeNull();
    }
    expect(
      resolveMembershipChoice(conflict, "unoffered-group", local, remote),
    ).toBeNull();
  });
});

describe("unsupported model lifecycle states", () => {
  it.each(["local", "remote", "both"])(
    "blocks a trashed model on %s",
    (side) => {
      const live = item("model", "a");
      const trashed = { ...live, deletedAt: "2026-01-01T00:00:00Z" };
      const plan = planMemberships(
        new Map([["model", "a"]]),
        [side === "remote" ? live : trashed],
        [side === "local" ? live : trashed],
      );
      expect(plan.resolved).toEqual([]);
      expect(plan.conflicts).toEqual([]);
      expect(plan.blocked).toEqual([
        { modelId: "model", reason: "model-deleted" },
      ]);
    },
  );

  it.each([
    ["empty incarnation", { incarnation: "" }],
    ["negative revision", { revision: -1 }],
    ["fractional revision", { revision: 1.5 }],
    ["unsafe revision", { revision: Number.MAX_SAFE_INTEGER + 1 }],
    ["non-finite revision", { revision: Number.NaN }],
  ])("blocks %s on either side", (_description, invalid) => {
    const valid = item("model", "a");
    for (const [local, remote] of [
      [{ ...valid, ...invalid }, valid],
      [valid, { ...valid, ...invalid }],
    ]) {
      const plan = planMemberships(
        new Map([["model", "a"]]),
        [local],
        [remote],
      );
      expect(plan.resolved).toEqual([]);
      expect(plan.conflicts).toEqual([]);
      expect(plan.blocked).toEqual([
        { modelId: "model", reason: "invalid-identity" },
      ]);
    }
  });

  it("blocks baseline models missing from both current snapshots", () => {
    const plan = planMemberships(new Map([["model", null]]), [], []);
    expect(plan.resolved).toEqual([]);
    expect(plan.conflicts).toEqual([]);
    expect(plan.blocked).toEqual([
      { modelId: "model", reason: "missing-model" },
    ]);
  });
});
