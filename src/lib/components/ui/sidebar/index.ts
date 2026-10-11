import { useSidebar } from "./context.svelte.js";
import Content from "./sidebar-content.svelte";
import Footer from "./sidebar-footer.svelte";
import GroupAction from "./sidebar-group-action.svelte";
import GroupContent from "./sidebar-group-content.svelte";
import GroupLabel from "./sidebar-group-label.svelte";
import Group from "./sidebar-group.svelte";
import Header from "./sidebar-header.svelte";
import MenuBadge from "./sidebar-menu-badge.svelte";
import MenuButton from "./sidebar-menu-button.svelte";
import MenuItem from "./sidebar-menu-item.svelte";
import MenuSub from "./sidebar-menu-sub.svelte";
import Menu from "./sidebar-menu.svelte";
import Provider from "./sidebar-provider.svelte";
import Trigger from "./sidebar-trigger.svelte";
import Root from "./sidebar.svelte";

export {
  Content,
  Footer,
  Group,
  GroupAction,
  GroupContent,
  GroupLabel,
  Header,
  Menu,
  MenuBadge,
  MenuButton,
  MenuItem,
  MenuSub,
  Provider,
  Root,
  //
  Root as Sidebar,
  Content as SidebarContent,
  Footer as SidebarFooter,
  Group as SidebarGroup,
  GroupAction as SidebarGroupAction,
  GroupContent as SidebarGroupContent,
  GroupLabel as SidebarGroupLabel,
  Header as SidebarHeader,
  Menu as SidebarMenu,
  MenuBadge as SidebarMenuBadge,
  MenuButton as SidebarMenuButton,
  MenuItem as SidebarMenuItem,
  MenuSub as SidebarMenuSub,
  Provider as SidebarProvider,
  Trigger as SidebarTrigger,
  Trigger,
  useSidebar,
};
