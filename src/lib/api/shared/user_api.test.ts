import { describe, expect, it } from "vitest";
import { createUserInstance, linkableUsers } from "./user_api";

const user = (id: number, perms: string[], token: string | null) =>
  createUserInstance(id, `u${id}`, "", "2026-01-01", perms, null, token, null);

describe("linkableUsers", () => {
  it("keeps a plain local user", () => {
    expect(linkableUsers([user(1, [], null)]).map((u) => u.id)).toEqual([1]);
  });

  it("keeps a local user that has a stale sync token", () => {
    expect(
      linkableUsers([user(1, ["Sync"], "stale")]).map((u) => u.id),
    ).toEqual([1]);
  });

  it("hides online-only users", () => {
    expect(linkableUsers([user(2, ["OnlineAccount"], "tok")])).toEqual([]);
  });
});
