import { Select as SelectPrimitive } from "bits-ui";

import GroupHeading from "./select-group-heading.svelte";
import Item from "./select-item.svelte";
import Content from "./select-content.svelte";
import Trigger from "./select-trigger.svelte";
import Separator from "./select-separator.svelte";

const Root = SelectPrimitive.Root;
const Group = SelectPrimitive.Group;

export { Root, Group, GroupHeading, Item, Content, Trigger, Separator };
