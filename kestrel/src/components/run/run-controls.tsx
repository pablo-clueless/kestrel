"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Play, Square } from "lucide-react";

import {
  errorMessage,
  getHealth,
  listRuns,
  startRun,
  stopRun,
  unconfirmedHost,
} from "@/lib/client";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { useSelectedEndpoint, useWorkspaceStore } from "@/stores/workspace-store";
import { MethodBadge } from "@/components/shared/method-badge";
import type { RunConfig } from "@/types/engine/RunConfig";
import { ConfirmHostDialog } from "./confirm-host-dialog";
import { Label } from "@/components/workspace/fields";
import { useRunEvents } from "@/hooks/use-run-events";
import { useRunStore } from "@/stores/run-store";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

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

/** The right-hand panel: picks a test, starts and stops runs, shows the current run's details, and
 * re-attaches to a run in progress after a reload. */
export const RunControls = () => {
  const { runId, status, error, config: lastConfig, report, attach, setError } = useRunStore();
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

  const num = (set: (n: number) => void) => (e: React.ChangeEvent<HTMLInputElement>) =>
    set(Number(e.target.value));
  const needsEndpoint = kind !== "fake";

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b px-5 py-4">
        <h2 className="text-[15px] font-semibold">Run test</h2>
        <span
          className={cn(
            "flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs",
            health.isSuccess
              ? "bg-green-50 text-green-700 dark:bg-green-950 dark:text-green-400"
              : "bg-muted text-muted-foreground",
          )}
        >
          <span
            className={cn(
              "size-1.5 rounded-full",
              health.isSuccess ? "bg-green-500" : "bg-red-500",
            )}
          />
          {health.isSuccess ? "Engine ready" : health.isError ? "Engine offline" : "Connecting…"}
        </span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto p-5 text-sm">
        <Field label="Test type">
          <Select value={kind} disabled={running} onValueChange={(v) => setKind(v as TestKind)}>
            <SelectTrigger className="w-full">
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
        </Field>

        {needsEndpoint && (
          <Field label="Target">
            {endpoint ? (
              <div className="bg-muted flex min-w-0 items-center gap-2 rounded-xs px-3 py-2">
                <MethodBadge method={endpoint.method} />
                <span className="truncate font-medium">{endpoint.name || endpoint.url}</span>
              </div>
            ) : (
              <p className="text-muted-foreground rounded-xs border border-dashed px-3 py-2">
                Select an endpoint in the sidebar.
              </p>
            )}
          </Field>
        )}

        {kind === "load" && (
          <>
            <Field label="Model">
              <Select
                value={mode}
                disabled={running}
                onValueChange={(v) => setMode(v as LoadModeType)}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="open">Open: fixed arrival rate</SelectItem>
                  <SelectItem value="closed">Closed: fixed number of users</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            <div className="grid grid-cols-2 gap-3">
              {mode === "open" ? (
                <>
                  <Field label="Rate (req/s)">
                    <Input
                      type="number"
                      min={1}
                      value={rate}
                      disabled={running}
                      onChange={num(setRate)}
                    />
                  </Field>
                  <Field label="Max in flight">
                    <Input
                      type="number"
                      min={1}
                      value={maxInFlight}
                      disabled={running}
                      onChange={num(setMaxInFlight)}
                    />
                  </Field>
                </>
              ) : (
                <Field label="Users">
                  <Input
                    type="number"
                    min={1}
                    value={concurrency}
                    disabled={running}
                    onChange={num(setConcurrency)}
                  />
                </Field>
              )}
              <Field label="Duration (s)">
                <Input
                  type="number"
                  min={1}
                  max={60}
                  value={durationS}
                  disabled={running}
                  onChange={num(setDurationS)}
                />
              </Field>
              <Field label="Ramp-up (s)">
                <Input
                  type="number"
                  min={0}
                  value={rampS}
                  disabled={running}
                  onChange={num(setRampS)}
                />
              </Field>
            </div>
          </>
        )}

        {kind === "fake" && (
          <Field label="Duration (s)">
            <Input
              type="number"
              min={1}
              max={60}
              value={durationS}
              disabled={running}
              onChange={num(setDurationS)}
            />
          </Field>
        )}

        {kind === "latency" && (
          <div className="grid grid-cols-2 gap-3">
            <Field label="Warm-up">
              <Input
                type="number"
                min={0}
                value={warmup}
                disabled={running}
                onChange={num(setWarmup)}
              />
            </Field>
            <Field label="Samples">
              <Input
                type="number"
                min={1}
                value={samples}
                disabled={running}
                onChange={num(setSamples)}
              />
            </Field>
          </div>
        )}

        {needsEndpoint && (
          <>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Timeout (ms)">
                <Input
                  type="number"
                  min={1}
                  value={timeoutMs}
                  disabled={running}
                  onChange={num(setTimeoutMs)}
                />
              </Field>
              <Field label="OK statuses">
                <Input
                  placeholder="404, 409"
                  value={okStatuses}
                  disabled={running}
                  onChange={(e) => setOkStatuses(e.target.value)}
                />
              </Field>
            </div>
            <label className="flex items-center justify-between">
              <span>Keep-alive</span>
              <Switch
                checked={keepAlive}
                disabled={running}
                onCheckedChange={(checked) => setKeepAlive(!!checked)}
              />
            </label>
          </>
        )}

        {kind === "load" && mode === "closed" && (
          <p className="bg-muted text-muted-foreground rounded-xs p-3 text-xs">
            Closed model: throughput falls when the server slows, so it understates how bad a stall
            is for real traffic. Use the open model to test a target rate.
          </p>
        )}
        {kind === "load" && mode === "open" && !keepAlive && rate > PORT_EXHAUSTION_RPS && (
          <p className="rounded-xs bg-amber-50 p-3 text-xs text-amber-700 dark:bg-amber-950 dark:text-amber-400">
            Keep-alive off at {rate} req/s opens a new connection per request and can exhaust
            ephemeral ports. Those failures are reported as client errors, not target errors.
          </p>
        )}

        {runId && (
          <dl className="flex flex-col gap-3 border-t pt-5">
            <Detail label="Status">
              <span className={cn("font-medium capitalize", running && "text-primary")}>
                {status}
              </span>
            </Detail>
            <Detail label="Test">
              {TESTS.find((t) => t.kind === lastConfig?.kind)?.label ?? "…"}
            </Detail>
            {report?.target && (
              <Detail label="Target host">
                {report.target.host}{" "}
                <span className="text-muted-foreground">({report.target.pinnedIp})</span>
              </Detail>
            )}
            {report && (
              <Detail label="Finished">{new Date(report.finishedAtMs).toLocaleTimeString()}</Detail>
            )}
            <Detail label="Run ID">
              <span className="font-mono text-xs">{runId.slice(0, 8)}</span>
            </Detail>
          </dl>
        )}
        {error && <p className="text-destructive">{error}</p>}
      </div>

      <div className="flex gap-2 border-t p-4">
        <Button
          className="flex-1"
          size="lg"
          onClick={() => start.mutate()}
          disabled={running || start.isPending || !health.isSuccess || (needsEndpoint && !endpoint)}
        >
          <Play /> Run
        </Button>
        <Button
          variant="outline"
          size="lg"
          onClick={() => stop.mutate()}
          disabled={!running || stop.isPending}
        >
          <Square /> Stop
        </Button>
      </div>

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

const Field = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <div className="flex min-w-0 flex-col gap-1.5">
    <Label>{label}</Label>
    {children}
  </div>
);

/** Label over value, like a details list. */
const Detail = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <div className="flex flex-col gap-0.5">
    <dt className="text-muted-foreground text-xs">{label}</dt>
    <dd>{children}</dd>
  </div>
);
