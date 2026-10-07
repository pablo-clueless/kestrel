"use client";

import { Activity, Cloud, CloudOff, Eye, Save } from "lucide-react";
import { useQuery } from "@tanstack/react-query";

import { useWorkspaceStore } from "@/stores/workspace-store";
import { useRunStore } from "@/stores/run-store";
import { getHealth } from "@/lib/client";
import { useCanEdit } from "@/hooks/use-me";
import { cn } from "@/lib/utils";

const SAVE_LABELS = {
  idle: "All changes saved",
  saving: "Saving…",
  saved: "All changes saved",
  error: "Save failed",
};

/** Bottom strip: engine connection, autosave, current run. */
export const StatusBar = () => {
  const health = useQuery({ queryKey: ["health"], queryFn: getHealth, refetchInterval: 5000 });
  const { saveState, saveError } = useWorkspaceStore();
  const { runId, status, config } = useRunStore();
  const canEdit = useCanEdit();

  return (
    <footer className="bg-card text-muted-foreground flex h-8 shrink-0 items-center gap-5 border-t px-4 text-xs">
      <span className="flex items-center gap-1.5">
        {health.isSuccess ? (
          <>
            <Cloud className="text-success size-3.5" /> Engine online · v{health.data.version}
          </>
        ) : (
          <>
            <CloudOff className="text-destructive size-3.5" />{" "}
            {health.isError ? "Engine offline" : "Connecting…"}
          </>
        )}
      </span>
      {canEdit ? (
        <span
          className={cn("flex items-center gap-1.5", saveState === "error" && "text-destructive")}
          title={saveError ?? undefined}
        >
          <Save className="size-3.5" /> {SAVE_LABELS[saveState]}
        </span>
      ) : (
        <span
          className="text-warning flex items-center gap-1.5"
          title="Changes you make here aren't saved, and you can't send requests or start runs. Ask one of its admins for write access."
        >
          <Eye className="size-3.5" /> Read-only: you have read access
        </span>
      )}
      {runId && (
        <span className="flex items-center gap-1.5">
          <Activity
            className={cn("size-3.5", status === "running" && "text-primary animate-pulse")}
          />
          {config?.kind ?? "run"} {runId.slice(0, 8)} · {status}
        </span>
      )}
    </footer>
  );
};
