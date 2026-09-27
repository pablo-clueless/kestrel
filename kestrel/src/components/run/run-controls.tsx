"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { Play, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { errorMessage, getHealth, listRuns, startRun, stopRun, unconfirmedHost } from "@/lib/client";
import { useSelectedEndpoint, useWorkspaceStore } from "@/stores/workspace-store";
import type { RunConfig } from "@/types/engine/RunConfig";
import { Label } from "@/components/workspace/fields";
import { useRunEvents } from "@/hooks/use-run-events";
import { Checkbox } from "@/components/ui/checkbox";
import { useRunStore } from "@/stores/run-store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

import { ConfirmHostDialog } from "./confirm-host-dialog";

type TestKind = RunConfig["kind"];
type LoadModeType = "closed" | "open";

const TESTS: { kind: TestKind; label: string }[] = [
  { kind: "latency", label: "Latency probe" },
  { kind: "load", label: "Load test" },
  { kind: "fake", label: "Fake (no traffic)" },
];

/** Above this rate with keep-alive off, the client can run out of ephemeral ports (TIME_WAIT). */
const PORT_EXHAUSTION_RPS = 200;

/** "404, 409" → [404, 409] */
const parseStatuses = (s: string) =>
  s
    .split(/[\s,]+/)
    .map(Number)
    .filter((n) => Number.isInteger(n) && n >= 100 && n <= 599);

/** Picks a test, starts and stops runs, and re-attaches to a run in progress after a reload. */
export const RunControls = () => {
  const { runId, status, error, attach, setError } = useRunStore();
  const endpoint = useSelectedEndpoint();
  const environment = useWorkspaceStore((s) => s.workspace?.activeEnvironment ?? null);
  const flush = useWorkspaceStore((s) => s.flush);
  const running = status === "running";

  const [kind, setKind] = useState<TestKind>("latency");
  const [durationS, setDurationS] = useState(20);
  const [warmup, setWarmup] = useState(10);
  const [samples, setSamples] = useState(100);
  const [timeoutMs, setTimeoutMs] = useState(10_000);
  const [keepAlive, setKeepAlive] = useState(true);
  const [mode, setMode] = useState<LoadModeType>("open");
  const [concurrency, setConcurrency] = useState(10);
  const [rate, setRate] = useState(100);
  const [rampS, setRampS] = useState(0);
  const [maxInFlight, setMaxInFlight] = useState(1000);
  const [okStatuses, setOkStatuses] = useState("");
  const [pendingHost, setPendingHost] = useState<string | null>(null);

  useRunEvents();

  const health = useQuery({ queryKey: ["health"], queryFn: getHealth, refetchInterval: 5000 });

  // Resume the newest in-progress run once, on first load.
  const resumed = useRef(false);
  const runs = useQuery({ queryKey: ["runs"], queryFn: listRuns, enabled: health.isSuccess });
  useEffect(() => {
    if (resumed.current || !runs.data) return;
    resumed.current = true;
    const active = runs.data.find((r) => r.status === "running");
    if (active && !useRunStore.getState().runId) attach(active.runId);
  }, [runs.data, attach]);

  const buildConfig = (): RunConfig => {
    if (kind === "fake") return { kind, durationMs: Math.round(durationS * 1000) };
    if (!endpoint) throw new Error("Select an endpoint first.");
    const common = {
      endpointId: endpoint.id,
      environment,
      timeoutMs,
      keepAlive,
      okStatuses: parseStatuses(okStatuses),
    };
    if (kind === "latency") return { kind, ...common, warmup, samples };
    return {
      kind,
      ...common,
      mode: mode === "closed" ? { type: "closed", concurrency } : { type: "open", rate },
      durationMs: Math.round(durationS * 1000),
      rampUpMs: Math.round(rampS * 1000),
      maxInFlight: mode === "open" ? maxInFlight : null,
    };
  };

  const start = useMutation({
    mutationFn: async () => {
      const config = buildConfig();
      // The engine runs what's saved, so push pending edits first.
      if (config.kind !== "fake") await flush();
      return startRun(config);
    },
    onSuccess: ({ runId }) => attach(runId),
    onError: (err) => {
      const host = unconfirmedHost(err);
      if (host) setPendingHost(host);
      else setError(errorMessage(err));
    },
  });

  const stop = useMutation({
    mutationFn: () => stopRun(runId!),
    onError: (err) => setError(errorMessage(err)),
  });

  const num = (set: (n: number) => void) => (e: React.ChangeEvent<HTMLInputElement>) => set(Number(e.target.value));
  const needsEndpoint = kind !== "fake";

  return (
    <div className="flex flex-col gap-3 text-sm">
      <div className="flex items-center gap-2">
        <span
          className={cn(
            "size-2 rounded-full",
            health.isSuccess ? "bg-green-500" : health.isError ? "bg-red-500" : "bg-secondary-4",
          )}
        />
        <span>
          {health.isSuccess
            ? `Engine v${health.data.version}`
            : health.isError
              ? errorMessage(health.error)
              : "Connecting…"}
        </span>
      </div>
      <div className="grid grid-cols-[7rem_1fr] items-center gap-x-3 gap-y-2">
        <Label>Test</Label>
        <Select value={kind} disabled={running} onValueChange={(v) => setKind(v as TestKind)}>
          <SelectTrigger className="w-full self-start font-mono">
            <SelectValue placeholder="Select a test" />
          </SelectTrigger>
          <SelectContent>
            {TESTS.map((t) => (
              <SelectItem key={t.kind} value={t.kind}>
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        {needsEndpoint && (
          <>
            <Label>Endpoint</Label>
            <span className="truncate font-mono text-xs">
              {endpoint ? `${endpoint.method} ${endpoint.name || endpoint.url}` : "none selected"}
            </span>
          </>
        )}

        {kind === "load" && (
          <>
            <Label>Model</Label>
            <Select value={mode} disabled={running} onValueChange={(v) => setMode(v as LoadModeType)}>
              <SelectTrigger className="w-1/2 self-start font-mono">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="open">Open (fixed rate)</SelectItem>
                <SelectItem value="closed">Closed (fixed users)</SelectItem>
              </SelectContent>
            </Select>
            {mode === "open" ? (
              <>
                <Label>Rate (req/s)</Label>
                <Input type="number" min={1} value={rate} disabled={running} onChange={num(setRate)} />
                <Label>Max in flight</Label>
                <Input type="number" min={1} value={maxInFlight} disabled={running} onChange={num(setMaxInFlight)} />
              </>
            ) : (
              <>
                <Label>Users</Label>
                <Input type="number" min={1} value={concurrency} disabled={running} onChange={num(setConcurrency)} />
              </>
            )}
            <Label>Ramp-up (s)</Label>
            <Input type="number" min={0} value={rampS} disabled={running} onChange={num(setRampS)} />
          </>
        )}

        {(kind === "fake" || kind === "load") && (
          <>
            <Label>Duration (s)</Label>
            <Input type="number" min={1} max={60} value={durationS} disabled={running} onChange={num(setDurationS)} />
          </>
        )}

        {kind === "latency" && (
          <>
            <Label>Warm-up</Label>
            <Input type="number" min={0} value={warmup} disabled={running} onChange={num(setWarmup)} />
            <Label>Samples</Label>
            <Input type="number" min={1} value={samples} disabled={running} onChange={num(setSamples)} />
          </>
        )}

        {needsEndpoint && (
          <>
            <Label>Timeout (ms)</Label>
            <Input type="number" min={1} value={timeoutMs} disabled={running} onChange={num(setTimeoutMs)} />
            <Label>Keep-alive</Label>
            <Checkbox checked={keepAlive} disabled={running} onCheckedChange={(checked) => setKeepAlive(!!checked)} />
            <Label>OK statuses</Label>
            <Input
              placeholder="e.g. 404, 409"
              value={okStatuses}
              disabled={running}
              onChange={(e) => setOkStatuses(e.target.value)}
            />
          </>
        )}
      </div>

      {kind === "load" && mode === "closed" && (
        <p className="text-text-gray text-xs">
          Closed model: throughput falls when the server slows, so it understates how bad a stall is for real traffic.
          Use the open model to test a target rate.
        </p>
      )}
      {kind === "load" && mode === "open" && !keepAlive && rate > PORT_EXHAUSTION_RPS && (
        <p className="text-xs text-amber-600">
          Keep-alive off at {rate} req/s opens a new connection per request and can exhaust ephemeral ports. Those
          failures are reported as client errors, not target errors.
        </p>
      )}

      <div className="flex gap-2">
        <Button
          onClick={() => start.mutate()}
          disabled={running || start.isPending || !health.isSuccess || (needsEndpoint && !endpoint)}
        >
          <Play /> Run
        </Button>
        <Button variant="outline" onClick={() => stop.mutate()} disabled={!running || stop.isPending}>
          <Square /> Stop
        </Button>
      </div>
      <p className="text-text-gray">{runId ? `Run ${runId.slice(0, 8)} · ${status}` : "No run yet."}</p>
      {error && <p className="text-red-600">{error}</p>}

      <ConfirmHostDialog
        host={pendingHost}
        onClose={() => setPendingHost(null)}
        onConfirmed={() => {
          setPendingHost(null);
          start.mutate();
        }}
      />
    </div>
  );
};
