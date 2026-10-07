"use client";

import { Check, ChevronsUpDown, Settings, Users } from "lucide-react";
import Link from "next/link";
import { useState } from "react";

import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { useCurrentWorkspace, useMe } from "@/hooks/use-me";
import type { Role } from "@/types/engine/Role";
import { switchWorkspace } from "@/lib/client";
import { cn } from "cn";

export const ROLE_LABELS: Record<Role, string> = {
  admin: "Admin",
  write: "Write",
  read: "Read",
};

/** The current workspace, and a menu to open another. Hidden when accounts are off: there's only
 * this browser's workspace then. */
export const WorkspaceSwitcher = () => {
  const [open, setOpen] = useState(false);
  const me = useMe().data;
  const current = useCurrentWorkspace();
  if (!me?.user || !current) return null;

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <button
            type="button"
            className="hover:bg-muted/60 flex w-full items-center gap-2 border-b px-4 py-2 text-left text-sm transition-colors"
            aria-label={`Workspace: ${current.name}. Switch workspace`}
          />
        }
      >
        <Users className="text-muted-foreground size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate font-medium" title={current.name}>
          {current.name}
        </span>
        {current.role !== "admin" && <RoleBadge role={current.role} />}
        <ChevronsUpDown className="text-muted-foreground size-3.5 shrink-0" />
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 gap-0 p-0">
        <p className="text-muted-foreground border-b px-2.5 py-2 text-xs">Your workspaces</p>
        <ul className="flex max-h-72 flex-col overflow-y-auto py-1">
          {me.workspaces.map((w) => (
            <li key={w.id}>
              <button
                type="button"
                className="hover:bg-muted flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-sm"
                onClick={() => (w.id === current.id ? setOpen(false) : switchWorkspace(w.id))}
              >
                <Check className={cn("size-3.5 shrink-0", w.id !== current.id && "invisible")} />
                <span className="min-w-0 flex-1 truncate">{w.name}</span>
                <RoleBadge role={w.role} />
              </button>
            </li>
          ))}
        </ul>
        <Link
          href="/settings"
          className="hover:bg-muted flex items-center gap-2 border-t px-2.5 py-2 text-sm"
          onClick={() => setOpen(false)}
        >
          <Settings className="size-3.5" /> Members and workspaces
        </Link>
      </PopoverContent>
    </Popover>
  );
};

export const RoleBadge = ({ role }: { role: Role }) => (
  <span
    className={cn(
      "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium",
      role === "read" ? "bg-warning/15 text-warning" : "bg-primary/15 text-primary",
    )}
  >
    {ROLE_LABELS[role]}
  </span>
);
