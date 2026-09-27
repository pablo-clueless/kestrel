"use client";

import { Plus, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import { useWorkspaceStore } from "@/stores/workspace-store";
import type { Endpoint } from "@/types/engine/Endpoint";

import { Input } from "./fields";

const METHOD_COLORS: Record<string, string> = {
  GET: "text-green-600",
  POST: "text-primary",
  PUT: "text-blue-600",
  PATCH: "text-purple-600",
  DELETE: "text-red-600",
};

/** Sidebar: endpoints grouped by tag/folder, with filter and add. Also loads the workspace. */
export const EndpointList = () => {
  const { workspace, selectedId, loadError, load, select, addEndpoint, removeEndpoint } =
    useWorkspaceStore();
  const [filter, setFilter] = useState("");

  useEffect(() => {
    void load();
  }, [load]);

  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const matches = (e: Endpoint) =>
      !q || `${e.method} ${e.name} ${e.url}`.toLowerCase().includes(q);
    const map = new Map<string, Endpoint[]>();
    for (const e of workspace?.endpoints ?? []) {
      if (!matches(e)) continue;
      const key = e.group ?? "";
      map.set(key, [...(map.get(key) ?? []), e]);
    }
    return [...map.entries()];
  }, [workspace, filter]);

  if (loadError) {
    return (
      <div className="flex flex-col gap-2 text-sm">
        <p className="text-red-600">{loadError}</p>
        <button className="text-primary self-start" onClick={() => void load()}>
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex items-center justify-between">
        <span className="text-text-gray text-xs uppercase">Endpoints</span>
        <button
          className="text-text-gray hover:text-primary"
          onClick={addEndpoint}
          disabled={!workspace}
          aria-label="Add endpoint"
        >
          <Plus className="size-4" />
        </button>
      </div>
      <Input placeholder="Filter…" value={filter} onChange={(e) => setFilter(e.target.value)} />

      <nav className="-mx-2 flex-1 overflow-y-auto">
        {groups.map(([group, endpoints]) => (
          <div key={group} className="mb-3">
            {group && <p className="text-text-gray px-2 pb-1 text-xs">{group}</p>}
            {endpoints.map((e) => (
              <div
                key={e.id}
                className={cn(
                  "group flex items-center gap-2 px-2 py-1 text-sm",
                  e.id === selectedId ? "bg-primary/10" : "hover:bg-secondary-3/5",
                )}
              >
                <button className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => select(e.id)}>
                  <span className={cn("w-12 shrink-0 font-mono text-xs font-bold", METHOD_COLORS[e.method])}>
                    {e.method}
                  </span>
                  <span className="truncate">{e.name || e.url}</span>
                </button>
                <button
                  className="text-text-gray hidden hover:text-red-600 group-hover:block"
                  onClick={() => removeEndpoint(e.id)}
                  aria-label={`Delete ${e.name}`}
                >
                  <Trash2 className="size-3.5" />
                </button>
              </div>
            ))}
          </div>
        ))}
        {workspace && workspace.endpoints.length === 0 && (
          <p className="text-text-gray px-2 text-sm">No endpoints yet. Press + to add one.</p>
        )}
      </nav>
    </div>
  );
};
