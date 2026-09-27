import { AppWindow, FlaskConical, Folder, LayoutDashboard, LucideIcon } from "lucide-react";

interface SidebarEntry {
  icon: LucideIcon;
  label: string;
}

export const SIDEBAR_CONFIG = (): SidebarEntry[] => [
  { icon: LayoutDashboard, label: "Overview" },
  { icon: Folder, label: "Collections" },
  { icon: AppWindow, label: "Environments" },
  { icon: FlaskConical, label: "Tests" },
];
