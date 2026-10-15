import { DropdownMenu as DropdownMenuPrimitive } from "bits-ui";
import CheckboxItem from "./dropdown-menu-checkbox-item.svelte";
import Content from "./dropdown-menu-content.svelte";
import GroupHeading from "./dropdown-menu-group-heading.svelte";
import Item from "./dropdown-menu-item.svelte";
import Label from "./dropdown-menu-label.svelte";
import Separator from "./dropdown-menu-separator.svelte";

const Root = DropdownMenuPrimitive.Root;
const Trigger = DropdownMenuPrimitive.Trigger;
const Group = DropdownMenuPrimitive.Group;

export {
  CheckboxItem,
  Content,
  Group,
  GroupHeading,
  Item,
  Label,
  Root,
  Separator,
  Trigger,
};
