"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Play, Square } from "lucide-react";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { MethodBadge } from "@/components/shared/method-badge";
import type { RunConfig } from "@/types/engine/RunConfig";
import type { Endpoint } from "@/types/engine/Endpoint";
import { ConfirmHostDialog } from "./confirm-host-dialog";
import { Label } from "@/components/workspace/fields";
import { useRunEvents } from "@/hooks/use-run-events";
import { useRunStore } from "@/stores/run-store";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useValues } from "@/hooks/use-values";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import {
  useSelectedEndpoint,
  useSelectedIsDraft,
  useWorkspaceStore,
} from "@/stores/workspace-store";
import {
  errorMessage,
  getHealth,
  listRuns,
  startRun,
  stopRun,
  unconfirmedHost,
} from "@/lib/client";

type TestKind = RunConfig["kind"];
type LoadModeType = "closed" | "open";

const TESTS: { kind: TestKind; label: string }[] = [
  { kind: "latency", label: "Latency Probe" },
  { kind: "load", label: "Load Test" },
  { kind: "complexity", label: "Big-O (complexity)" },
  { kind: "fake", label: "Fake (no traffic)" },
];

/** Above this rate with keep-alive off, the client can run out of ephemeral ports (TIME_WAIT). */
const PORT_EXHAUSTION_RPS = 200;

/** Matches the engine's size generators: {{n}}, {{n:int_array}}, {{n:string}}, {{n:object_array}}. */
const SIZE_GENERATOR = /{{s*n(:(int_array|string|object_array))?s*}}/;

const usesSize = (e: Endpoint) =>
  [e.url, ...e.query.map((q) => q.value), ...e.headers.map((h) => h.value), ...bodyTexts(e)].some(
    (s) => SIZE_GENERATOR.test(s),
  );

/** Every template string in the body. */
const bodyTexts = ({ body }: Endpoint): string[] => {
  switch (body.type) {
    case "none":
      return [];
    case "json":
    case "raw":
      return [body.content];
    case "form":
      return body.fields.filter((f) => f.enabled).map((f) => f.value);
    case "multipart":
      return body.fields.filter((f) => f.enabled && f.kind === "text").map((f) => f.value);
  }
};

/** Methods that don't send a body, so there's no input to grow for a Big-O sweep. */
const BODYLESS_METHODS: Endpoint["method"][] = ["GET", "HEAD", "OPTIONS", "DELETE"];

/** "404, 409" → [404, 409] */
const parseStatuses = (s: string) =>
  s
    .split(/[\s,]+/)
    .map(Number)
    .filter((n) => Number.isInteger(n) && n >= 100 && n <= 599);

/** Form defaults. Sizes and durations are what the UI shows (seconds, counts), not engine units. */
const DEFAULTS = {
  kind: "latency" as TestKind,
  durationS: 20,
  warmup: 10,
  samples: 100,
  timeoutMs: 10_000,
  keepAlive: true,
  mode: "open" as LoadModeType,
  concurrency: 10,
  rate: 100,
  rampS: 0,
  maxInFlight: 1000,
  okStatuses: "",
  // Big-O
  minN: 1,
  maxN: 16_384,
  sizes: 15,
  rounds: 20,
  slowMs: 2_000,
  budgetS: 120,
};

type Settings = typeof DEFAULTS;
/** Settings edited through number inputs. */
type NumberSetting = {
  [K in keyof Settings]: Settings[K] extends number ? K : never;
}[keyof Settings];

/** The right-hand panel: picks a test, starts and stops runs, shows the current run's details, and
 * re-attaches to a run in progress after a reload. */
export const RunControls = () => {
  const {
    runId,
    status,
    error,
    config: lastConfig,
    report,
    attach,
    setError,
    clear,
  } = useRunStore();
  const endpoint = useSelectedEndpoint();
  const isDraft = useSelectedIsDraft();
  const environment = useWorkspaceStore((s) => s.workspace?.activeEnvironment ?? null);
  const flush = useWorkspaceStore((s) => s.flush);
  const running = status === "running";

  const { values, set } = useValues({ initialValue: DEFAULTS });
  const {
    kind,
    durationS,
    warmup,
    samples,
    timeoutMs,
    keepAlive,
    mode,
    concurrency,
    rate,
    rampS,
    maxInFlight,
    okStatuses,
    minN,
    maxN,
    sizes,
    rounds,
    slowMs,
    budgetS,
  } = values;
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

  // A different endpoint makes the run results stale, unless the shown run is of that endpoint (a
  // run opened from history selects its endpoint). Responses are kept per tab.
  const endpointId = endpoint?.id ?? null;
  const prevEndpointId = useRef(endpointId);
  useEffect(() => {
    if (prevEndpointId.current === endpointId) return;
    prevEndpointId.current = endpointId;
    const shown = useRunStore.getState().config;
    if (shown && shown.kind !== "fake" && shown.endpointId === endpointId) return;
    clear();
  }, [endpointId, clear]);

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
    if (kind === "complexity") {
      return {
        kind,
        ...common,
        minN,
        maxN,
        points: sizes,
        samples: rounds,
        warmup,
        slowMs,
        budgetMs: Math.round(budgetS * 1000),
      };
    }
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

  // onChange for a number input bound to one setting.
  const num = (key: NumberSetting) => (e: React.ChangeEvent<HTMLInputElement>) =>
    set(key, Number(e.target.value));
  const needsEndpoint = kind !== "fake";
  const bodyless = !!endpoint && BODYLESS_METHODS.includes(endpoint.method);

  // Big-O needs a body; switching to a bodyless request drops back to the default test.
  useEffect(() => {
    if (bodyless && kind === "complexity" && !running) {
      set("kind", DEFAULTS.kind);
      clear();
    }
  }, [bodyless, kind, running, set, clear]);

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
          <Select
            value={kind}
            disabled={running}
            onValueChange={(v) => {
              set("kind", v as TestKind);
              clear();
            }}
          >
            <SelectTrigger className="w-full font-mono text-xs capitalize">
              <SelectValue placeholder="Select a test" />
            </SelectTrigger>
            <SelectContent>
              {TESTS.map((t) => (
                <SelectItem
                  key={t.kind}
                  value={t.kind}
                  disabled={t.kind === "complexity" && bodyless}
                >
                  {t.label}
                  {t.kind === "complexity" && bodyless && (
                    <span className="text-muted-foreground normal-case">
                      (needs a request body)
                    </span>
                  )}
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
            ) : null}
            {endpoint && isDraft && (
              <p className="text-muted-foreground text-xs">
                Unsaved request. Save it to a collection to run tests against it.
              </p>
            )}
            {!endpoint && (
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
                onValueChange={(v) => set("mode", v as LoadModeType)}
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
                      onChange={num("rate")}
                    />
                  </Field>
                  <Field label="Max in flight">
                    <Input
                      type="number"
                      min={1}
                      value={maxInFlight}
                      disabled={running}
                      onChange={num("maxInFlight")}
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
                    onChange={num("concurrency")}
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
                  onChange={num("durationS")}
                />
              </Field>
              <Field label="Ramp-up (s)">
                <Input
                  type="number"
                  min={0}
                  value={rampS}
                  disabled={running}
                  onChange={num("rampS")}
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
              onChange={num("durationS")}
            />
          </Field>
        )}

        {kind === "complexity" && (
          <>
            {endpoint && !usesSize(endpoint) && (
              <p className="rounded-lg bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                Mark the input size in the request with <code>{"{{n}}"}</code> (e.g. a limit),{" "}
                <code>{"{{n:int_array}}"}</code>, <code>{"{{n:string}}"}</code> or{" "}
                <code>{"{{n:object_array}}"}</code> in the body. Each request gets fresh random
                contents.
              </p>
            )}
            <div className="grid grid-cols-2 gap-3">
              <Field label="Smallest n">
                <Input
                  type="number"
                  min={1}
                  value={minN}
                  disabled={running}
                  onChange={num("minN")}
                />
              </Field>
              <Field label="Largest n">
                <Input
                  type="number"
                  min={1}
                  value={maxN}
                  disabled={running}
                  onChange={num("maxN")}
                />
              </Field>
              <Field label="Sizes">
                <Input
                  type="number"
                  min={3}
                  max={40}
                  value={sizes}
                  disabled={running}
                  onChange={num("sizes")}
                />
              </Field>
              <Field label="Samples per size">
                <Input
                  type="number"
                  min={1}
                  value={rounds}
                  disabled={running}
                  onChange={num("rounds")}
                />
              </Field>
              <Field label="Slow limit (ms)">
                <Input
                  type="number"
                  min={1}
                  value={slowMs}
                  disabled={running}
                  onChange={num("slowMs")}
                />
              </Field>
              <Field label="Time budget (s)">
                <Input
                  type="number"
                  min={1}
                  max={300}
                  value={budgetS}
                  disabled={running}
                  onChange={num("budgetS")}
                />
              </Field>
              <Field label="Warm-up">
                <Input
                  type="number"
                  min={0}
                  value={warmup}
                  disabled={running}
                  onChange={num("warmup")}
                />
              </Field>
            </div>
            <p className="text-muted-foreground text-xs">
              Sizes are spaced geometrically and sampled in shuffled rounds. A size slower than the
              limit stops the sweep from growing further.
            </p>
          </>
        )}

        {kind === "latency" && (
          <div className="grid grid-cols-2 gap-3">
            <Field label="Warm-up">
              <Input
                type="number"
                min={0}
                value={warmup}
                disabled={running}
                onChange={num("warmup")}
              />
            </Field>
            <Field label="Samples">
              <Input
                type="number"
                min={1}
                value={samples}
                disabled={running}
                onChange={num("samples")}
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
                  onChange={num("timeoutMs")}
                />
              </Field>
              <Field label="OK statuses">
                <Input
                  placeholder="404, 409"
                  value={okStatuses}
                  disabled={running}
                  onChange={(e) => set("okStatuses", e.target.value)}
                />
              </Field>
            </div>
            <label className="flex items-center justify-between">
              <span>Keep-alive</span>
              <Switch
                checked={keepAlive}
                disabled={running}
                onCheckedChange={(checked) => set("keepAlive", !!checked)}
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
          disabled={
            running ||
            start.isPending ||
            !health.isSuccess ||
            (needsEndpoint && (!endpoint || isDraft))
          }
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
