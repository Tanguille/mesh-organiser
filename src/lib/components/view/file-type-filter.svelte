<script lang="ts">
  import { FileType } from "$lib/api/shared/blob_api";
  import * as Select from "$lib/components/ui/select/index.js";
  import FileBox from "@lucide/svelte/icons/file-box";

  let {
    value = $bindable([]),
    onchange = () => {},
  }: { value?: FileType[]; onchange?: (fileTypes: FileType[]) => void } =
    $props();

  const options: [FileType, string][] = [
    [FileType.STL, "Stl"],
    [FileType.OBJ, "Obj"],
    [FileType.THREEMF, "3mf"],
    [FileType.STEP, "Step"],
    [FileType.GCODE, "Gcode"],
  ];
</script>

<Select.Root
  type="multiple"
  name="Filetypes"
  bind:value
  onValueChange={(x) => onchange(x as FileType[])}
>
  <Select.Trigger
    class="w-auto border-primary {value.length > 0 ? 'text-primary' : ''}"
    hideArrow={true}
    title="File type filter"
  >
    <FileBox />
  </Select.Trigger>
  <Select.Content>
    <Select.Group>
      <Select.GroupHeading>File type filter</Select.GroupHeading>
      {#each options as [fileType, label] (fileType)}
        <Select.Item value={fileType} {label}>{label}</Select.Item>
      {/each}
    </Select.Group>
  </Select.Content>
</Select.Root>
