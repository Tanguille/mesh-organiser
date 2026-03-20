import type { IconProps } from "@lucide/svelte";
import type { Component } from "svelte";

import PrinterCheck from "@lucide/svelte/icons/printer-check";
import Star from "@lucide/svelte/icons/star";
import type { ClassValue } from "svelte/elements";
import type { ModelFlags } from "./api/shared/model_api";

export type Glyph = Component<IconProps, Record<string, unknown>, "">;

export interface GlyphObject {
  /** Stable key for `{#each ...}` (order matches flag checks). */
  id: string;
  glyph: Glyph;
  badgeClasses?: ClassValue;
  glyphClasses?: ClassValue;
}

export function flagsToGlyphObjects(flags: ModelFlags): GlyphObject[] {
  const glyphs: GlyphObject[] = [];

  if (flags.printed) {
    glyphs.push({ id: "printed", glyph: PrinterCheck });
  }

  if (flags.favorite) {
    glyphs.push({ id: "favorite", glyph: Star });
  }

  return glyphs;
}
