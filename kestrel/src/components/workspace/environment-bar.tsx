"use client";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { useWorkspaceStore } from "@/stores/workspace-store";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

const SAVE_LABELS = { idle: "", saving: "Saving…", saved: "Saved", error: "Save failed" } as const;

/** Header: active environment + autosave status. */
export const EnvironmentBar = () => {
  const { workspace, saveState, saveError, setActiveEnvironment } = useWorkspaceStore();
  const envs = workspace?.environments ?? [];

  return (
    <div className="flex items-center gap-3">
      <Label>Environment</Label>
      <Select
        value={workspace?.activeEnvironment ?? ""}
        disabled={!workspace}
        onValueChange={(v) => setActiveEnvironment(v || null)}
      >
        <SelectTrigger>
          <SelectValue placeholder="Select an environment" />
        </SelectTrigger>
        <SelectContent>
          {envs.map((e) => (
            <SelectItem key={e.name} value={e.name}>
              {e.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <span
        className={cn("text-xs", saveState === "error" ? "text-red-600" : "text-text-gray")}
        title={saveError ?? undefined}
      >
        {SAVE_LABELS[saveState]}
      </span>
    </div>
  );
};
