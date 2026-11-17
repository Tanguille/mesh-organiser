export type Membership = string | null;

export interface Assignment {
  modelId: string;
  groupId: Membership;
  incarnation: string;
  revision: number;
  deletedAt: string | null;
}

export interface MembershipConflict {
  modelId: string;
  base: Membership | undefined;
  local: Assignment;
  remote: Assignment;
}

export interface MembershipPlan {
  resolved: { modelId: string; groupId: Membership }[];
  conflicts: MembershipConflict[];
  blocked: {
    modelId: string;
    reason:
      | "duplicate-identity"
      | "missing-model"
      | "invalid-identity"
      | "model-deleted";
  }[];
}

export function planMemberships(
  base: ReadonlyMap<string, Membership>,
  local: readonly Assignment[],
  remote: readonly Assignment[],
): MembershipPlan {
  const plan: MembershipPlan = { resolved: [], conflicts: [], blocked: [] };
  const index = (items: readonly Assignment[]) => {
    const byId = new Map<string, Assignment[]>();
    for (const item of items) {
      const matches = byId.get(item.modelId) ?? [];
      matches.push(item);
      byId.set(item.modelId, matches);
    }
    return byId;
  };
  const localById = index(local);
  const remoteById = index(remote);
  const modelIds = new Set([
    ...base.keys(),
    ...local.map((item) => item.modelId),
    ...remote.map((item) => item.modelId),
  ]);
  for (const modelId of modelIds) {
    const left = localById.get(modelId) ?? [];
    const right = remoteById.get(modelId) ?? [];
    const previous = base.get(modelId);
    if (
      !modelId ||
      previous === "" ||
      [...left, ...right].some(
        (item) =>
          item.groupId === "" ||
          !item.incarnation ||
          !Number.isSafeInteger(item.revision) ||
          item.revision < 0,
      )
    ) {
      plan.blocked.push({ modelId, reason: "invalid-identity" });
    } else if (left.length > 1 || right.length > 1) {
      plan.blocked.push({ modelId, reason: "duplicate-identity" });
    } else if (!left.length || !right.length) {
      plan.blocked.push({ modelId, reason: "missing-model" });
    } else if (left[0].deletedAt !== null || right[0].deletedAt !== null) {
      plan.blocked.push({ modelId, reason: "model-deleted" });
    } else if (left[0].groupId === right[0].groupId) {
      plan.resolved.push({ modelId, groupId: left[0].groupId });
    } else if (base.has(modelId) && left[0].groupId === previous) {
      plan.resolved.push({ modelId, groupId: right[0].groupId });
    } else if (base.has(modelId) && right[0].groupId === previous) {
      plan.resolved.push({ modelId, groupId: left[0].groupId });
    } else {
      plan.conflicts.push({
        modelId,
        base: previous,
        local: left[0],
        remote: right[0],
      });
    }
  }
  return plan;
}

export function resolveMembershipChoice(
  conflict: MembershipConflict,
  choice: Membership,
  currentLocal: Assignment,
  currentRemote: Assignment,
): { modelId: string; groupId: Membership } | null {
  const unchanged = (a: Assignment, b: Assignment) =>
    a.modelId === b.modelId &&
    a.groupId === b.groupId &&
    a.incarnation === b.incarnation &&
    a.revision === b.revision &&
    a.deletedAt === b.deletedAt;
  if (
    (choice !== conflict.local.groupId && choice !== conflict.remote.groupId) ||
    !unchanged(conflict.local, currentLocal) ||
    !unchanged(conflict.remote, currentRemote)
  )
    return null;
  return { modelId: conflict.modelId, groupId: choice };
}
