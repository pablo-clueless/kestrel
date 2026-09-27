"use client";

import { ChevronRight, PanelLeft } from "lucide-react";

import { useLayoutStore } from "@/stores/layout-store";
import { useActiveCollection, useSelectedEndpoint } from "@/stores/workspace-store";

import { EnvironmentBar } from "../workspace";
import { MethodBadge } from "./method-badge";

/** Breadcrumb (collection / endpoint) on the left, environment on the right. */
export const Header = () => {
  const toggleSidebar = useLayoutStore((s) => s.toggleSidebar);
  const collection = useActiveCollection();
  const endpoint = useSelectedEndpoint();

  return (
    <header className="bg-card flex h-14 w-full shrink-0 items-center justify-between gap-4 border-b px-4">
      <div className="flex min-w-0 items-center gap-3 text-sm">
        <button
          onClick={toggleSidebar}
          className="text-muted-foreground hover:text-foreground"
          aria-label="Toggle sidebar"
        >
          <PanelLeft className="size-4" />
        </button>
        <nav className="flex min-w-0 items-center gap-1.5" aria-label="Breadcrumb">
          <span className="text-muted-foreground truncate">{collection?.name ?? "No collection"}</span>
          {endpoint && (
            <>
              <ChevronRight className="text-muted-foreground size-3.5 shrink-0" />
              <MethodBadge method={endpoint.method} />
              <span className="truncate font-medium">{endpoint.name || endpoint.url}</span>
            </>
          )}
        </nav>
      </div>
      <EnvironmentBar />
    </header>
  );
};
