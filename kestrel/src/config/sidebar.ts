import {
  AppWindow,
  FlaskConical,
  LayoutDashboard,
  LucideIcon,
  Settings,
  UserCog,
  Users,
} from "lucide-react";

interface SidebarEntry {
  label: string;
  routes: RouteConfig[];
}

interface RouteConfig {
  href: string;
  icon: LucideIcon;
  label: string;
  disabled?: boolean;
}

export const SIDEBAR_CONFIG: SidebarEntry[] = [
  {
    label: "main",
    routes: [
      { href: "/admin/overview", icon: LayoutDashboard, label: "Overview" },
      { href: "/admin/users", icon: Users, label: "Users" },
      { href: "/admin/workspaces", icon: AppWindow, label: "Workspaces" },
      // Not built yet: shown, but not clickable.
      {
        href: "/admin/experimentals",
        icon: FlaskConical,
        label: "Experimental Features",
        disabled: true,
      },
    ],
  },
  {
    label: "settings",
    routes: [
      { href: "/admin/settings", icon: Settings, label: "Settings" },
      { href: "/admin/roles", icon: UserCog, label: "Roles", disabled: true },
    ],
  },
];
