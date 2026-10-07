"use client";

import { ArrowLeft, ChevronRight, Moon, PanelLeft, Sun } from "lucide-react";
import { usePathname } from "next/navigation";
import { useTheme } from "next-themes";
import Link from "next/link";

import { WORKSPACE_PATH } from "@/hooks/use-to-workspace";

import { useLayoutStore } from "@/stores/layout-store";
import { EnvironmentBar } from "../workspace";
import { MethodBadge } from "./method-badge";
import { Button } from "../ui/button";
import {
  useActiveCollection,
  useSelectedEndpoint,
  useSelectedIsDraft,
} from "@/stores/workspace-store";
import { LoggUser } from "./user";
import { cn } from "cn";

const Theme = ({
  theme,
  toggleTheme,
}: {
  theme: string | undefined;
  toggleTheme: (theme: string) => void;
}) => {
  const ThemeIcon = theme === "light" ? Moon : Sun;
  return (
    <Button
      onClick={() => toggleTheme(theme === "dark" ? "light" : "dark")}
      variant="outline"
      size="icon"
      aria-label="Toggle theme"
    >
      <ThemeIcon className="size-4" />
    </Button>
  );
};

/** Breadcrumb (collection / endpoint) on the left, environment on the right. */
export const Header = () => {
  const toggleSidebar = useLayoutStore((s) => s.toggleSidebar);
  const sidebarOpen = useLayoutStore((s) => s.sidebarOpen);
  const collection = useActiveCollection();
  const endpoint = useSelectedEndpoint();
  const { setTheme, theme } = useTheme();
  const isDraft = useSelectedIsDraft();
  // Settings and Profile share this header; there, the way back replaces the breadcrumb.
  const onWorkspace = usePathname() === WORKSPACE_PATH;

  const toggleTheme = () => setTheme(theme === "dark" ? "light" : "dark");

  return (
    <header className="bg-card flex h-14 w-full shrink-0 items-center justify-between gap-4 border-b px-4">
      <div className="flex min-w-0 items-center gap-3 text-sm">
        <button
          onClick={toggleSidebar}
          className="text-muted-foreground hover:text-foreground"
          aria-label="Toggle sidebar"
        >
          <PanelLeft
            className={cn(
              "size-4 transform transition-transform duration-300",
              !sidebarOpen ? "rotate-180" : "",
            )}
          />
        </button>
        {!onWorkspace ? (
          <Link
            href={WORKSPACE_PATH}
            className="text-muted-foreground hover:text-foreground flex items-center gap-1.5 transition-colors"
          >
            <ArrowLeft className="size-4" /> Back to workspace
          </Link>
        ) : (
          endpoint && (
            <nav className="flex min-w-0 items-center gap-1.5" aria-label="Breadcrumb">
              <span className="text-muted-foreground truncate">
                {isDraft ? "Unsaved" : (collection?.name ?? "No collection")}
              </span>
              <ChevronRight className="text-muted-foreground size-3.5 shrink-0" />
              <MethodBadge method={endpoint.method} />
              <span className="truncate font-medium">{endpoint.name || endpoint.url}</span>
            </nav>
          )
        )}
      </div>
      <div className="flex items-center gap-2">
        <EnvironmentBar />
        <Theme theme={theme} toggleTheme={toggleTheme} />
        <LoggUser />
      </div>
    </header>
  );
};
