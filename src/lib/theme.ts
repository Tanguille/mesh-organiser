import { configuration } from "./configuration.svelte";
import defaultCss from "../themes/default.css?raw";

const availableThemes = {
  default: "Default",
  shadcnred: "Shadcn Red",
  shadcnrose: "Shadcn Rose",
  shadcnorange: "Shadcn Orange",
  shadcngreen: "Shadcn Green",
  shadcnblue: "Shadcn Blue",
  shadcnyellow: "Shadcn Yellow",
  shadcnviolet: "Shadcn Violet",
  nord: "Nord",
  vintageretro: "Vintage Retro",
  synthwave: "Synthwave",
  quantumrose: "Quantum Rose",
  graphite: "Graphite",
  perpetuity: "Perpetuity",
  nature: "Nature",
  sunsethorizon: "Sunset Horizon",
  amethysthaze: "Amethyst Haze",
  custom: "Custom",
};

export function getThemeName(theme: string): string {
  const index = theme as keyof typeof availableThemes;
  if (index in availableThemes) {
    return availableThemes[index];
  }

  return "(Unknown)";
}

export function getAvailableThemes(): string[] {
  return Object.keys(availableThemes);
}

export async function setTheme(theme: string) {
  if (!(theme in availableThemes)) {
    theme = "default";
  }

  const existingCustomTheme = document.getElementById("custom-theme");
  if (existingCustomTheme) {
    existingCustomTheme.remove();
  }

  if (theme === "custom") {
    console.log("Loading custom theme");

    if (configuration.custom_css.length <= 0) {
      configuration.custom_css = defaultCss;
    }

    const custom_css = configuration.custom_css.replaceAll(
      '[data-theme="default"]',
      '[data-theme="custom"]',
    );

    const style = document.createElement("style");
    style.textContent = custom_css;
    style.id = "custom-theme";
    document.head.appendChild(style);
  }

  document.documentElement.setAttribute("data-theme", theme);
}
