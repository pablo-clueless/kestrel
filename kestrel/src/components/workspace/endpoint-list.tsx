"use client";

import { Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { motion } from "framer-motion";

import { MethodBadge } from "@/components/shared/method-badge";
import { useWorkspaceStore } from "@/stores/workspace-store";
import type { Collection } from "@/types/engine/Collection";
import type { Endpoint } from "@/types/engine/Endpoint";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/** The endpoints of one collection, grouped by tag/folder, with filter and add. */
export const EndpointList = ({ collection }: { collection: Collection }) => {
  const { selectedId, select, addEndpoint, removeEndpoint } = useWorkspaceStore();
  const [filter, setFilter] = useState("");

  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const matches = (e: Endpoint) =>
      !q || `${e.method} ${e.name} ${e.url}`.toLowerCase().includes(q);
    const map = new Map<string, Endpoint[]>();
    for (const e of collection.endpoints) {
      if (!matches(e)) continue;
      const key = e.group ?? "";
      map.set(key, [...(map.get(key) ?? []), e]);
    }
    return [...map.entries()];
  }, [collection.endpoints, filter]);

  return (
    <motion.div className="flex flex-col gap-2 px-3 py-2">
      {collection.endpoints.length > 5 && (
        <Input placeholder="Filter…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      )}
      {groups.map(([group, endpoints]) => (
        <motion.div key={group}>
          {group && <p className="text-muted-foreground px-2 pb-1 text-xs">{group}</p>}
          {endpoints.map((e) => (
            <motion.div
              key={e.id}
              className={cn(
                "group flex items-center gap-2 rounded-xs px-2 py-1 text-sm transition-colors duration-150",
                e.id === selectedId ? "bg-muted font-medium" : "hover:bg-muted/60",
              )}
            >
              <button
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
                onClick={() => select(e.id)}
              >
                <MethodBadge method={e.method} short className="w-11" />
                <span className="truncate">{e.name || e.url}</span>
              </button>
              <button
                className="text-muted-foreground pointer-events-none opacity-0 transition-[opacity,color] duration-150 group-hover:pointer-events-auto group-hover:opacity-100 hover:text-red-600 focus-visible:pointer-events-auto focus-visible:opacity-100"
                onClick={() => removeEndpoint(e.id)}
                aria-label={`Delete ${e.name}`}
              >
                <Trash2 className="size-3.5" />
              </button>
            </motion.div>
          ))}
        </motion.div>
      ))}
      <button
        className="text-muted-foreground hover:text-primary flex items-center gap-1 self-start px-2 text-xs"
        onClick={addEndpoint}
      >
        <Plus className="size-3.5" /> Add endpoint
      </button>
    </motion.div>
  );
};
