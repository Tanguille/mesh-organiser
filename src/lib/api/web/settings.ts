import {
  configurationDefault,
  SettingSection,
  type Configuration,
  type ISettingsApi,
} from "../shared/settings_api";

export class WebSettingsApi implements ISettingsApi {
  async getConfiguration(): Promise<Configuration> {
    const config = localStorage.getItem("configuration");
    const defaultConfiguration = configurationDefault();

    if (config === null) {
      return defaultConfiguration;
    }

    const parsedConfig = JSON.parse(config) as Partial<Configuration>;

    for (const key of Object.keys(
      defaultConfiguration,
    ) as (keyof Configuration)[]) {
      if (!(key in parsedConfig)) {
        (
          parsedConfig as Record<
            keyof Configuration,
            Configuration[keyof Configuration]
          >
        )[key] = defaultConfiguration[key];
      }
    }

    if (parsedConfig.slicer === null || parsedConfig.slicer === undefined) {
      parsedConfig.slicer = "OrcaSlicer";
    }

    return parsedConfig as Configuration;
  }

  async saveConfiguration(config: Configuration): Promise<void> {
    localStorage.setItem("configuration", JSON.stringify(config));
  }

  availableSections(): SettingSection[] {
    return [
      SettingSection.ModelPreview,
      SettingSection.UserInterface,
      SettingSection.Users,
      SettingSection.ThumbnailGenerationColorSection,
      SettingSection.BehaviourSectionAllPlatforms,
      SettingSection.CurrentUser,
    ];
  }
}
