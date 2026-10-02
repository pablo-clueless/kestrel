"use client";

import { FolderPlus, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { motion } from "framer-motion";

import { activeCollectionOf, groupsOf, useWorkspaceStore } from "@/stores/workspace-store";
import { MethodBadge } from "@/components/shared/method-badge";
import type { Collection } from "@/types/engine/Collection";
import type { Endpoint } from "@/types/engine/Endpoint";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/** Row actions that fade in on hover/focus instead of popping in (keeps row layout stable). */
const hoverAction =
  "text-muted-foreground pointer-events-none opacity-0 transition-[opacity,color] duration-150 group-hover:pointer-events-auto group-hover:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100";

/** The endpoints of one collection, grouped by tag/folder, with filter and add. Ungrouped
 * endpoints come first; groups made by hand are listed even while empty. */
export const EndpointList = ({ collection }: { collection: Collection }) => {
  const { selectedId, select, removeEndpoint, addGroup, removeGroup } = useWorkspaceStore();
  const setActiveCollection = useWorkspaceStore((s) => s.setActiveCollection);
  const isActive = useWorkspaceStore((s) => activeCollectionOf(s.workspace)?.id === collection.id);
  // Several collections can be open in the sidebar, but the editor and "add endpoint" work on the
  // active one, so acting inside this list makes its collection active first.
  const activate = () => {
    if (!isActive) setActiveCollection(collection.id);
  };
  const [filter, setFilter] = useState("");
  // Inline "new group" name field; null when closed.
  const [newGroup, setNewGroup] = useState<string | null>(null);

  const q = filter.trim().toLowerCase();
  const groups = useMemo(() => {
    const matches = (e: Endpoint) =>
      !q || `${e.method} ${e.name} ${e.url}`.toLowerCase().includes(q);
    const map = new Map<string, Endpoint[]>([
      ["", []],
      ...groupsOf(collection).map((g) => [g, []] as [string, Endpoint[]]),
    ]);
    for (const e of collection.endpoints) {
      if (matches(e)) map.get(e.group ?? "")!.push(e);
    }
    // While filtering, only groups with matches; otherwise every group, empty ones included.
    return [...map.entries()].filter(([g, eps]) => eps.length > 0 || (!q && g !== ""));
  }, [collection, q]);

  const createGroup = () => {
    const name = newGroup?.trim();
    if (name) addGroup(collection.id, name);
    setNewGroup(null);
  };

  return (
    <motion.div className="flex flex-col gap-2 px-3 py-2">
      {collection.endpoints.length > 5 && (
        <Input placeholder="Filter…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      )}
      {groups.map(([group, endpoints]) => (
        <motion.div key={group}>
          {group && (
            <div className="group flex items-center gap-1.5 px-2 pb-1">
              <p className="text-muted-foreground min-w-0 flex-1 truncate text-xs">{group}</p>
              <button
                className={cn(hoverAction, "hover:text-primary")}
                onClick={() => activate()}
                aria-label={`Add endpoint to ${group}`}
                title="Add endpoint to this group"
              >
                <Plus className="size-3.5" />
              </button>
              <button
                className={cn(hoverAction, "hover:text-red-600")}
                onClick={() => removeGroup(collection.id, group)}
                aria-label={`Delete group ${group}`}
                title="Delete group (its endpoints stay, ungrouped)"
              >
                <Trash2 className="size-3.5" />
              </button>
            </div>
          )}
          {group && endpoints.length === 0 && (
            <p className="text-muted-foreground/70 px-2 py-1 text-xs italic">Empty</p>
          )}
          {endpoints.map((e) => (
            <motion.div
              key={e.id}
              className={cn(
                "group flex items-center gap-2 rounded-xs px-2 py-1 text-sm transition-colors duration-150",
                e.id === selectedId ? "bg-primary/15 font-medium" : "hover:bg-muted-foreground/10",
              )}
            >
              <button
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
                onClick={() => {
                  activate();
                  select(e.id);
                }}
              >
                <MethodBadge method={e.method} short className="w-11" />
                <span className="truncate">{e.name || e.url}</span>
              </button>
              <button
                className={cn(hoverAction, "hover:text-red-600")}
                onClick={() => removeEndpoint(e.id)}
                aria-label={`Delete ${e.name}`}
              >
                <Trash2 className="size-3.5" />
              </button>
            </motion.div>
          ))}
        </motion.div>
      ))}
      {newGroup !== null && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            createGroup();
          }}
        >
          <Input
            autoFocus
            placeholder="Group name"
            value={newGroup}
            onChange={(e) => setNewGroup(e.target.value)}
            onBlur={createGroup}
            onKeyDown={(e) => e.key === "Escape" && setNewGroup(null)}
          />
        </form>
      )}
      <div className="flex items-center gap-3 px-2">
        <button
          className="text-muted-foreground hover:text-primary flex items-center gap-1 text-xs"
          onClick={() => activate()}
        >
          <Plus className="size-3.5" /> Add endpoint
        </button>
        <button
          className="text-muted-foreground hover:text-primary flex items-center gap-1 text-xs"
          onClick={() => setNewGroup("")}
        >
          <FolderPlus className="size-3.5" /> New group
        </button>
      </div>
    </motion.div>
  );
};
