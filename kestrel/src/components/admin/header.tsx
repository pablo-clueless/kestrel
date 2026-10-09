"use client";

import { Moon, PanelLeft, Sun } from "lucide-react";
import { useTheme } from "next-themes";

import { useLayoutStore } from "@/stores/layout-store";
import { LoggUser } from "../shared/user";
import { Button } from "../ui/button";
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
  // Resolved, so "system" toggles from whatever it's showing now.
  const { setTheme, resolvedTheme: theme } = useTheme();

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
      </div>
      <div className="flex items-center gap-2">
        <Theme theme={theme} toggleTheme={toggleTheme} />
        <LoggUser />
      </div>
    </header>
  );
};
