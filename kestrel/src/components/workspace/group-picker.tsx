"use client";

import { Check, ChevronDown, Folder, Plus } from "lucide-react";
import { useState } from "react";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

const item =
  "flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-xs transition-colors hover:bg-muted";

/** Picks an endpoint's group: one of the collection's existing groups, a new one, or none. */
export const GroupPicker = ({
  value,
  groups,
  onChange,
  className,
}: {
  value: string | null;
  groups: string[];
  onChange: (group: string | null) => void;
  className?: string;
}) => {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const q = query.trim();
  const shown = groups
    .filter((g) => g.toLowerCase().includes(q.toLowerCase()))
    .sort((a, b) => a.localeCompare(b));
  // Case-insensitive, so "users" picks the existing "Users" instead of making a near-duplicate.
  const existing = groups.find((g) => g.toLowerCase() === q.toLowerCase());

  const pick = (group: string | null) => {
    onChange(group);
    setOpen(false);
  };

  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (o) setQuery("");
      }}
    >
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            className={cn(
              "text-muted-foreground w-60 justify-between gap-1.5 px-2 text-xs",
              className,
            )}
            aria-label="Group"
          />
        }
      >
        <span className="flex items-center gap-1">
          <Folder className="size-3.5" />
          <span className="truncate">{value ?? "No group"}</span>
        </span>
        <ChevronDown className="size-3" />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56 gap-0 p-0">
        <Input
          autoFocus
          className="rounded-none border-0 border-b text-xs shadow-none focus-visible:ring-0 md:text-xs"
          placeholder="Find or create a group…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && q) {
              e.preventDefault();
              pick(existing ?? q);
            }
          }}
        />
        <div className="flex max-h-60 flex-col overflow-y-auto py-1">
          {!q && (
            <button className={item} onClick={() => pick(null)}>
              <Check className={cn("size-3.5", value !== null && "invisible")} />
              <span className="text-muted-foreground">No group</span>
            </button>
          )}
          {shown.map((g) => (
            <button key={g} className={item} onClick={() => pick(g)}>
              <Check className={cn("size-3.5", g !== value && "invisible")} />
              <span className="truncate">{g}</span>
            </button>
          ))}
          {q && !existing && (
            <button className={cn(item, "text-primary")} onClick={() => pick(q)}>
              <Plus className="size-3.5" />
              <span className="truncate">Create “{q}”</span>
            </button>
          )}
          {!q && groups.length === 0 && (
            <p className="text-muted-foreground px-2.5 py-1.5 text-xs">
              No groups yet. Type a name to create one.
            </p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
};
