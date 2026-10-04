"use client";

import { Folder, RotateCcwClock, type LucideIcon } from "lucide-react";
import Image from "next/image";

import { useLayoutStore, type SidebarTab } from "@/stores/layout-store";
import { CollectionList, History } from "../workspace";
import { cn } from "cn";

const IMAGE = "/assets/logo.png";
const TABS: { id: SidebarTab; label: string; icon: LucideIcon }[] = [
  { id: "collections", label: "Collections", icon: Folder },
  { id: "history", label: "History", icon: RotateCcwClock },
];

export const Sidebar = () => {
  const open = useLayoutStore((s) => s.sidebarOpen);
  const tab = useLayoutStore((s) => s.sidebarTab);
  const setTab = useLayoutStore((s) => s.setSidebarTab);
  // Stays mounted when collapsed (and CollectionList stays mounted on the History tab) so it keeps
  // the workspace loaded.
  // Width animates on the outer shell; the inner panel keeps a fixed width so content doesn't reflow mid-slide.
  return (
    <aside
      inert={!open}
      aria-hidden={!open}
      className={cn(
        "bg-card h-full shrink-0 overflow-hidden border-r transition-[width,border-color] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none",
        open ? "w-68" : "w-0 border-r-transparent",
      )}
    >
      <div
        className={cn(
          "flex h-full w-68 flex-col transition-[opacity,translate] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none",
          open ? "translate-x-0 opacity-100" : "-translate-x-4 opacity-0",
        )}
      >
        <div className="flex h-14 shrink-0 items-center gap-2 border-b px-4 font-semibold">
          <div className="relative aspect-[4.2/1] w-1/3">
            <Image alt="Kestrel" className="" fill src={IMAGE} />
          </div>
        </div>
        <div role="tablist" aria-label="Sidebar" className="flex shrink-0">
          {TABS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              role="tab"
              id={`sidebar-tab-${id}`}
              aria-selected={tab === id}
              aria-controls={`sidebar-panel-${id}`}
              onClick={() => setTab(id)}
              className={cn(
                "flex flex-1 items-center justify-center gap-1.5 px-2 py-2.5 text-xs transition-colors duration-150",
                tab === id
                  ? "bg-background text-foreground font-medium"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
              )}
            >
              <Icon className="size-3.5" />
              {label}
            </button>
          ))}
        </div>
        {TABS.map(({ id }) => (
          <div
            key={id}
            role="tabpanel"
            id={`sidebar-panel-${id}`}
            aria-labelledby={`sidebar-tab-${id}`}
            hidden={tab !== id}
            className="min-h-0 flex-1"
          >
            {id === "collections" ? <CollectionList /> : <History active={open && tab === id} />}
          </div>
        ))}
      </div>
    </aside>
  );
};
