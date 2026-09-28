"use client";

import { useLayoutStore } from "@/stores/layout-store";
import { CollectionList } from "../workspace";
import { Feather } from "lucide-react";
import { cn } from "cn";

/**
 * user can paste or import endpoints from the API spec here.  (M3: OpenAPI import)
 * user can paste or import multiple endpoints at once.        (M3)
 * user can import endpoints from the API spec file.           (M3)
 * user can import endpoints from the API spec file in JSON/YAML format. (M3)
 * endpoints are stored in the store.                          (done: workspace store → kestrel.json)
 * endpoints are displayed in the sidebar.                     (done: grouped into collections)
 * user can click on the endpoint to view the details.         (done: Request card)
 * user can click on the "Run" button to run the endpoint.     (done: Run panel)
 */
export const Sidebar = () => {
  const open = useLayoutStore((s) => s.sidebarOpen);
  // Stays mounted when collapsed so CollectionList keeps the workspace loaded.
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
          <Feather className="text-primary size-5" /> Kestrel
        </div>
        <div className="min-h-0 flex-1 p-3">
          <CollectionList />
        </div>
      </div>
    </aside>
  );
};
