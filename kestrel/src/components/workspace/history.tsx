"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { toast } from "sonner";

import { useSelectedEndpoint, useWorkspaceStore } from "@/stores/workspace-store";
import { errorMessage, getReport, listRuns } from "@/lib/client";
import { MethodBadge } from "@/components/shared/method-badge";
import type { RunSummary } from "@/types/engine/RunSummary";
import type { RunStatus } from "@/types/engine/RunStatus";
import type { RunKind } from "@/types/engine/RunKind";
import type { Endpoint } from "@/types/engine/Endpoint";
import { useRunStore } from "@/stores/run-store";
import { Switch } from "@/components/ui/switch";
import { int, ms } from "@/lib/format";
import { cn } from "@/lib/utils";

const KIND_LABEL: Record<RunKind, string> = {
  latency: "Latency",
  load: "Load",
  complexity: "Big-O",
  concurrency: "Race",
  fake: "Fake",
};

const STATUS_DOT: Record<RunStatus, string> = {
  running: "bg-primary animate-pulse",
  completed: "bg-green-500",
  cancelled: "bg-amber-500",
  failed: "bg-red-500",
};

const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const date = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
});

/** "Today", "Yesterday", else the date. */
const dayLabel = (startedAtMs: number) => {
  const day = new Date(startedAtMs).setHours(0, 0, 0, 0);
  const today = new Date().setHours(0, 0, 0, 0);
  if (day === today) return "Today";
  if (today - day === 86_400_000) return "Yesterday";
  return date.format(startedAtMs);
};

const duration = (run: RunSummary) => {
  if (!run.result) return null;
  const s = (run.result.finishedAtMs - run.startedAtMs) / 1000;
  if (s < 1) return `${Math.round(s * 1000)} ms`;
  return s < 60 ? `${s.toFixed(1)} s` : `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
};

/** Sidebar panel: past runs, newest first — the ones in the engine's memory plus the last 500
 * saved. Opening one selects its endpoint and shows its results in the Live and Results cards.
 * `active` is whether the panel is on screen; runs are only fetched while it is. */
export const History = ({ active }: { active: boolean }) => {
  const [thisEndpoint, setThisEndpoint] = useState(false);
  const [opening, setOpening] = useState<string | null>(null);
  const workspace = useWorkspaceStore((s) => s.workspace);
  const selected = useSelectedEndpoint();
  const shownRunId = useRunStore((s) => s.runId);

  const runs = useQuery({
    queryKey: ["runs"],
    queryFn: listRuns,
    enabled: active,
    // Keep a live run's row current while the panel is shown.
    refetchInterval: (q) => (q.state.data?.some((r) => r.status === "running") ? 2000 : false),
  });

  const endpoints = useMemo(
    () =>
      new Map<string, Endpoint>(
        workspace?.collections.flatMap((c) => c.endpoints.map((e) => [e.id, e] as const)) ?? [],
      ),
    [workspace],
  );

  const groups = useMemo(() => {
    const shown = (runs.data ?? []).filter(
      (r) => !thisEndpoint || (selected && r.endpointId === selected.id),
    );
    const byDay = new Map<string, RunSummary[]>();
    for (const r of shown) {
      const key = dayLabel(r.startedAtMs);
      byDay.set(key, [...(byDay.get(key) ?? []), r]);
    }
    return [...byDay.entries()];
  }, [runs.data, thisEndpoint, selected]);

  const openRun = async (run: RunSummary) => {
    const { select } = useWorkspaceStore.getState();
    const { attach, showReport } = useRunStore.getState();
    // Select first: the Run panel keeps a shown run only if it belongs to the selected endpoint.
    if (run.endpointId && endpoints.has(run.endpointId)) select(run.endpointId);
    if (run.status === "running") {
      attach(run.runId);
      return;
    }
    setOpening(run.runId);
    try {
      showReport(await getReport(run.runId));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setOpening(null);
    }
  };

  return (
    <div className="bg-background flex h-full flex-col gap-3 p-3">
      <div className="flex flex-col gap-2">
        <span
          className="text-muted-foreground text-xs uppercase"
          title="Finished runs are kept on the engine (the last 500)."
        >
          Run history
        </span>
        <label className="text-muted-foreground flex min-w-0 items-center gap-2 text-xs">
          <Switch
            size="sm"
            checked={thisEndpoint}
            onCheckedChange={setThisEndpoint}
            disabled={!selected}
          />
          <span className="truncate">
            Only {selected ? selected.name || selected.url || "this endpoint" : "this endpoint"}
          </span>
        </label>
      </div>

      <div className="-mx-2 min-h-0 flex-1 overflow-y-auto pb-3">
        {runs.isPending ? (
          <p className="text-muted-foreground px-2 text-xs">Loading…</p>
        ) : runs.isError ? (
          <p className="text-destructive px-2 text-xs">{errorMessage(runs.error)}</p>
        ) : groups.length === 0 ? (
          <p className="text-muted-foreground px-2 text-xs">
            {thisEndpoint ? "No runs of this endpoint yet." : "No runs yet."}
          </p>
        ) : (
          groups.map(([day, dayRuns]) => (
            <section key={day}>
              <h3 className="text-muted-foreground bg-card sticky top-0 z-10 px-2 pt-2 pb-1.5 text-[11px] font-medium tracking-wide uppercase">
                {day}
              </h3>
              <ul className="flex flex-col gap-0.5">
                {dayRuns.map((run) => (
                  <li key={run.runId}>
                    <RunRow
                      run={run}
                      endpoint={run.endpointId ? endpoints.get(run.endpointId) : undefined}
                      active={run.runId === shownRunId}
                      loading={opening === run.runId}
                      onOpen={() => openRun(run)}
                    />
                  </li>
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
    </div>
  );
};

const RunRow = ({
  run,
  endpoint,
  active,
  loading,
  onOpen,
}: {
  run: RunSummary;
  endpoint: Endpoint | undefined;
  active: boolean;
  loading: boolean;
  onOpen: () => void;
}) => {
  const r = run.result;
  const errorPct = r && r.totalRequests ? (r.totalErrors / r.totalRequests) * 100 : 0;

  return (
    <button
      onClick={onOpen}
      disabled={loading}
      className={cn(
        "flex w-full flex-col gap-1.5 rounded-xs px-2 py-2 text-left text-xs transition-colors",
        active ? "bg-muted" : "hover:bg-muted/60",
        loading && "opacity-60",
      )}
    >
      <div className="flex items-center gap-2">
        <span
          className={cn("size-1.5 shrink-0 rounded-full", STATUS_DOT[run.status])}
          title={run.status}
        />
        <span className="w-12 shrink-0 font-medium">{KIND_LABEL[run.kind]}</span>
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          {run.kind === "fake" ? (
            <span className="text-muted-foreground truncate">No traffic</span>
          ) : endpoint ? (
            <>
              <MethodBadge method={endpoint.method} short />
              <span className="truncate">{endpoint.name || endpoint.url}</span>
            </>
          ) : (
            <span className="text-muted-foreground truncate italic">Deleted endpoint</span>
          )}
        </span>
        <span className="text-muted-foreground shrink-0 tabular-nums">
          {time.format(run.startedAtMs)}
        </span>
      </div>
      <div className="text-muted-foreground flex flex-wrap items-center gap-x-2.5 gap-y-0.5 pl-3.5 font-mono text-[11px] tabular-nums">
        {r ? (
          <>
            <span>{int(r.totalRequests)} req</span>
            <span className={cn(r.totalErrors > 0 && "text-red-600 dark:text-red-400")}>
              {errorPct.toFixed(errorPct > 0 && errorPct < 1 ? 1 : 0)}% err
            </span>
            {r.p50Ms != null && (
              <span>
                p50 {ms(r.p50Ms)} · p99 {ms(r.p99Ms)} ms
              </span>
            )}
            <span className="ml-auto">{duration(run)}</span>
          </>
        ) : (
          <span className="capitalize">{run.status}…</span>
        )}
      </div>
    </button>
  );
};
