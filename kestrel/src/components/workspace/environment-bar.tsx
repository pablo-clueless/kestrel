"use client";

import { Settings } from "lucide-react";
import { useState } from "react";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../ui/sheet";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { useCanEdit } from "@/hooks/use-me";
import { Editable } from "../shared/editable";
import { EnvironmentEditor } from "./environment-editor";
import { Button } from "@/components/ui/button";

/** The picker's "no environment" entry. It stands for `null`; it's never stored as a name. */
const NONE = "__kestrel_no_environment__";

/** Header: active environment picker, plus a slide-over to edit environments and secrets. */
export const EnvironmentBar = () => {
  const { workspace, setActiveEnvironment } = useWorkspaceStore();
  const [open, setOpen] = useState(false);
  const canEdit = useCanEdit();
  const envs = workspace?.environments ?? [];
  const active = workspace?.activeEnvironment;
  const value = active && envs.some((e) => e.name === active) ? active : NONE;
  // Labels for the trigger, which otherwise shows the raw value (and NONE isn't for reading).
  const items = [
    { value: NONE, label: "No environment" },
    ...envs.map((e) => ({ value: e.name, label: e.name })),
  ];

  return (
    <div className="flex items-center gap-2">
      <Select
        value={value}
        items={items}
        disabled={!workspace}
        onValueChange={(v) => setActiveEnvironment(v && v !== NONE ? v : null)}
      >
        <SelectTrigger className="min-w-44">
          <SelectValue placeholder="No environment" />
        </SelectTrigger>
        <SelectContent>
          {items.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {item.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        variant="outline"
        size="icon"
        onClick={() => setOpen(true)}
        aria-label="Manage environments"
      >
        <Settings />
      </Button>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-125 overflow-y-auto">
          <SheetHeader>
            <SheetTitle>Environments</SheetTitle>
            <SheetDescription>
              Variables and write-only secrets. The active environment overrides collection
              variables.
              {!canEdit && " You have read access here, so you can look but not change them."}
            </SheetDescription>
          </SheetHeader>
          <Editable className="px-4 pb-6">
            <EnvironmentEditor />
          </Editable>
        </SheetContent>
      </Sheet>
    </div>
  );
};
