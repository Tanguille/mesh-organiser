import { getContainer } from "#lib/api/dependency_injection";
import { IModelApi, type Model } from "#lib/api/shared/model_api";
import { updateSidebarState } from "#lib/sidebar_data.svelte";
import { countWriter } from "#lib/utils";
import { toast } from "svelte-sonner";
import { goto } from "$app/navigation";
import { resolve } from "#lib/paths";

export async function trashModels(
  models: Model[],
  onDelete: (models: Model[]) => void,
) {
  try {
    await getContainer().require<IModelApi>(IModelApi).deleteModels(models);
    onDelete(models);
    toast.success(`Moved ${countWriter("model", models)} to trash`, {
      action: { label: "View trash", onClick: () => goto(resolve("/trash")) },
    });
    await updateSidebarState().catch((error) =>
      console.error("Failed to refresh sidebar:", error),
    );
  } catch (error) {
    console.error("Failed to move models to trash:", error);
    toast.error("Could not move models to trash. Try again.");
  }
}
