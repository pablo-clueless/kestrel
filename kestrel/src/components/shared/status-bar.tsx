"use client";

import { useQuery } from "@tanstack/react-query";
import { Activity, Cloud, CloudOff, Save } from "lucide-react";

import { getHealth } from "@/lib/client";
import { cn } from "@/lib/utils";
import { useRunStore } from "@/stores/run-store";
import { useWorkspaceStore } from "@/stores/workspace-store";

const SAVE_LABELS = { idle: "All changes saved", saving: "Saving…", saved: "All changes saved", error: "Save failed" };

/** Bottom strip: engine connection, autosave, current run. */
export const StatusBar = () => {
  const health = useQuery({ queryKey: ["health"], queryFn: getHealth, refetchInterval: 5000 });
  const { saveState, saveError } = useWorkspaceStore();
  const { runId, status, config } = useRunStore();

  return (
    <footer className="bg-card text-muted-foreground flex h-8 shrink-0 items-center gap-5 border-t px-4 text-xs">
      <span className="flex items-center gap-1.5">
        {health.isSuccess ? (
          <>
            <Cloud className="text-success size-3.5" /> Engine online · v{health.data.version}
          </>
        ) : (
          <>
            <CloudOff className="text-destructive size-3.5" /> {health.isError ? "Engine offline" : "Connecting…"}
          </>
        )}
      </span>
      <span className={cn("flex items-center gap-1.5", saveState === "error" && "text-destructive")} title={saveError ?? undefined}>
        <Save className="size-3.5" /> {SAVE_LABELS[saveState]}
      </span>
      {runId && (
        <span className="flex items-center gap-1.5">
          <Activity className={cn("size-3.5", status === "running" && "text-primary animate-pulse")} />
          {config?.kind ?? "run"} {runId.slice(0, 8)} · {status}
        </span>
      )}
    </footer>
  );
};
