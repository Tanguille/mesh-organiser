// @vitest-environment jsdom
import { render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import CurrentUserEditCard from "./current-user-edit-card.svelte";

describe("CurrentUserEditCard", () => {
  it("labels the password confirmation input with 'Verify new password'", () => {
    render(CurrentUserEditCard);

    expect(screen.getByLabelText("Verify new password").id).toBe("password2");
  });
});
