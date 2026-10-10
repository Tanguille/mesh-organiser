import { ContextMenu as ContextMenuPrimitive } from "bits-ui";

import Item from "./context-menu-item.svelte";
import Content from "./context-menu-content.svelte";

const Root = ContextMenuPrimitive.Root;
const Trigger = ContextMenuPrimitive.Trigger;

export {
  Root,
  Item,
  Trigger,
  Content,
  //
  Root as ContextMenu,
  Item as ContextMenuItem,
  Content as ContextMenuContent,
  Trigger as ContextMenuTrigger,
};
