"use client";

import { Settings } from "lucide-react";
import { useState } from "react";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "../ui/sheet";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { EnvironmentEditor } from "./environment-editor";
import { Button } from "@/components/ui/button";

/** Header: active environment picker, plus a slide-over to edit environments and secrets. */
export const EnvironmentBar = () => {
  const { workspace, setActiveEnvironment } = useWorkspaceStore();
  const [open, setOpen] = useState(false);
  const envs = workspace?.environments ?? [];

  return (
    <div className="flex items-center gap-2">
      <Select
        value={workspace?.activeEnvironment ?? ""}
        disabled={!workspace}
        onValueChange={(v) => setActiveEnvironment(v || null)}
      >
        <SelectTrigger className="min-w-44">
          <SelectValue placeholder="No environment" />
        </SelectTrigger>
        <SelectContent>
          {envs.map((e) => (
            <SelectItem key={e.name} value={e.name}>
              {e.name}
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
        <SheetContent side="right" className="overflow-y-auto">
          <SheetHeader>
            <SheetTitle>Environments</SheetTitle>
            <SheetDescription>
              Variables and write-only secrets. The active environment overrides collection
              variables.
            </SheetDescription>
          </SheetHeader>
          <div className="px-4 pb-6">
            <EnvironmentEditor />
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
};
