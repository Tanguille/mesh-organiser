// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { getContainer, resetContainer } from "#lib/api/dependency_injection";
import {
  configurationDefault,
  ISettingsApi,
  SettingSection,
} from "#lib/api/shared/settings_api";
import { configuration } from "#lib/configuration.svelte";
import Page from "./+page.svelte";

function retentionInput(savedDays = 30) {
  configuration.trash_retention_days = savedDays;
  getContainer().addSingleton(ISettingsApi, {
    availableSections: () => [SettingSection.Behaviour],
  });
  render(Page);

  return screen.getByRole("spinbutton", {
    name: "Days to keep deleted models",
  }) as HTMLInputElement;
}

describe("settings trash retention", () => {
  afterEach(() => {
    resetContainer();
    configuration.trash_retention_days =
      configurationDefault().trash_retention_days;
  });

  it("keeps typed prefixes out of configuration until the value is committed", async () => {
    const input = retentionInput();
    expect(input.value).toBe("30");

    for (const value of ["3", "36", "365"]) {
      await fireEvent.input(input, { target: { value } });

      expect(configuration.trash_retention_days).toBe(30);
      expect(input.value).toBe(value);
    }

    await fireEvent.change(input);

    expect(configuration.trash_retention_days).toBe(365);
    expect(input.value).toBe("365");
  });

  it.each([1, 36500])("accepts the committed boundary %i", async (days) => {
    const input = retentionInput();
    await fireEvent.input(input, { target: { value: String(days) } });
    await fireEvent.change(input);

    expect(configuration.trash_retention_days).toBe(days);
    expect(input.value).toBe(String(days));
  });

  it.each(["", "0", "1.5", "-1", "36501"])(
    "restores the saved value after an invalid committed value %j",
    async (value) => {
      const input = retentionInput(90);
      await fireEvent.input(input, { target: { value } });
      expect(configuration.trash_retention_days).toBe(90);

      await fireEvent.change(input);

      expect(configuration.trash_retention_days).toBe(90);
      expect(input.value).toBe("90");
    },
  );
});
