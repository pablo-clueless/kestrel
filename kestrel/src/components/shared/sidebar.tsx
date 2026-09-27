"use client";

import { Feather } from "lucide-react";
import { cn } from "cn";

import { useLayoutStore } from "@/stores/layout-store";

import { CollectionList } from "../workspace";

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
  return (
    <aside className={cn("bg-card flex h-full w-68 shrink-0 flex-col border-r", !open && "hidden")}>
      <div className="flex h-14 shrink-0 items-center gap-2 border-b px-4 font-semibold">
        <Feather className="text-primary size-5" /> Kestrel
      </div>
      <div className="min-h-0 flex-1 p-3">
        <CollectionList />
      </div>
    </aside>
  );
};
