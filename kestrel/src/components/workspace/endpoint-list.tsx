"use client";

import { ChevronRight, FolderPlus, Plus, Trash2 } from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import { useMemo, useState } from "react";

import { activeCollectionOf, byName, groupsOf, useWorkspaceStore } from "@/stores/workspace-store";
import { groupKey, useLayoutStore } from "@/stores/layout-store";
import { InlineNameForm } from "@/components/shared/inline-name-form";
import { MethodBadge } from "@/components/shared/method-badge";
import type { Collection } from "@/types/engine/Collection";
import type { Endpoint } from "@/types/engine/Endpoint";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/** Row actions that fade in on hover/focus instead of popping in (keeps row layout stable). */
const hoverAction =
  "text-muted-foreground pointer-events-none opacity-0 transition-[opacity,color] duration-150 group-hover:pointer-events-auto group-hover:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100";

/** Height + fade for a group opening or closing, like the collections around it. */
const collapse = {
  initial: { height: 0, opacity: 0 },
  animate: { height: "auto", opacity: 1 },
  exit: { height: 0, opacity: 0 },
  transition: { duration: 0.2, ease: [0.32, 0.72, 0, 1] },
} as const;

/** The endpoints of one collection, grouped by tag/folder, with filter and add, alphabetically
 * within each group. Ungrouped endpoints come first; groups made by hand are listed even while empty. Each group can be
 * collapsed; while filtering, every group with a match is shown open. */
export const EndpointList = ({ collection }: { collection: Collection }) => {
  const { selectedId, select, addEndpoint, removeEndpoint, addGroup, removeGroup } =
    useWorkspaceStore();
  const setActiveCollection = useWorkspaceStore((s) => s.setActiveCollection);
  const isActive = useWorkspaceStore((s) => activeCollectionOf(s.workspace)?.id === collection.id);
  // Several collections can be open in the sidebar, but the editor and "add endpoint" work on the
  // active one, so acting inside this list makes its collection active first.
  const activate = () => {
    if (!isActive) setActiveCollection(collection.id);
  };
  const [filter, setFilter] = useState("");
  const collapsedGroups = useLayoutStore((s) => s.collapsedGroups);
  const toggleGroup = useLayoutStore((s) => s.toggleGroup);
  const expandGroup = useLayoutStore((s) => s.expandGroup);
  // Whether the inline "new group" name field is open.
  const [addingGroup, setAddingGroup] = useState(false);

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
    // Alphabetical within each group, by what the row shows.
    const label = (e: Endpoint) => e.name || e.url;
    for (const eps of map.values()) eps.sort((a, b) => byName.compare(label(a), label(b)));
    // While filtering, only groups with matches; otherwise every group, empty ones included.
    return [...map.entries()].filter(([g, eps]) => eps.length > 0 || (!q && g !== ""));
  }, [collection, q]);

  return (
    <motion.div className="flex flex-col gap-2 px-3 py-2">
      {collection.endpoints.length > 5 && (
        <Input placeholder="Filter…" value={filter} onChange={(e) => setFilter(e.target.value)} />
      )}
      {groups.map(([group, endpoints]) => {
        const open = !group || !!q || !collapsedGroups.includes(groupKey(collection.id, group));
        return (
          <motion.div key={group}>
            {group && (
              <div className="group flex items-center gap-1.5 px-2 pb-1">
                <button
                  className="text-muted-foreground hover:text-foreground flex min-w-0 flex-1 items-center gap-1 text-left text-xs"
                  onClick={() => toggleGroup(collection.id, group)}
                  aria-expanded={open}
                  disabled={!!q}
                  title={q ? undefined : open ? "Collapse group" : "Expand group"}
                >
                  <ChevronRight
                    className={cn(
                      "size-3 shrink-0 transition-[rotate] duration-200 ease-out motion-reduce:transition-none",
                      open && "rotate-90",
                    )}
                  />
                  <span className="truncate">{group}</span>
                  <span className="text-muted-foreground/70 shrink-0">{endpoints.length}</span>
                </button>
                <button
                  className={cn(hoverAction, "hover:text-primary")}
                  onClick={() => {
                    activate();
                    expandGroup(collection.id, group);
                    addEndpoint(group);
                  }}
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
            <AnimatePresence initial={false}>
              {open && (
                <motion.div key="endpoints" {...collapse} className="overflow-hidden">
                  {group && endpoints.length === 0 && (
                    <p className="text-muted-foreground/70 px-2 py-1 text-xs italic">Empty</p>
                  )}
                  {endpoints.map((e) => (
                    <motion.div
                      key={e.id}
                      className={cn(
                        "group flex items-center gap-2 rounded-xs px-2 py-1 text-sm transition-colors duration-150",
                        e.id === selectedId
                          ? "bg-primary/15 border-primary border-l-2 font-medium"
                          : "hover:bg-muted-foreground/10",
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
              )}
            </AnimatePresence>
          </motion.div>
        );
      })}
      {addingGroup && (
        <InlineNameForm
          placeholder="Group name"
          onSubmit={(name) => {
            addGroup(collection.id, name);
            setAddingGroup(false);
          }}
          onCancel={() => setAddingGroup(false)}
        />
      )}
      <div className="flex items-center gap-3 px-2">
        <button
          className="text-muted-foreground hover:text-primary flex items-center gap-1 text-xs"
          onClick={() => {
            activate();
            addEndpoint();
          }}
        >
          <Plus className="size-3.5" /> Add endpoint
        </button>
        <button
          className="text-muted-foreground hover:text-primary flex items-center gap-1 text-xs"
          onClick={() => setAddingGroup(true)}
        >
          <FolderPlus className="size-3.5" /> New group
        </button>
      </div>
    </motion.div>
  );
};
