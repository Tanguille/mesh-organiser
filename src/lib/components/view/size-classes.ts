import type { SizeOptionModels } from "$lib/api/shared/settings_api";

// Tailwind size classes per grid/list item size. group-grid layers extra
// `[&_.imglist]:w-[...]` widths on top of the List_* entries for its image strip.
export const SizeOptionClasses: Record<SizeOptionModels, string> = {
  Grid_Small: "w-32 text-sm",
  Grid_Medium: "w-40",
  Grid_Large: "w-60",
  List_Small: "h-10 text-sm hidden-if-small",
  List_Medium: "h-14",
  List_Large: "h-20 text-lg",
};
