"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Play, Square } from "lucide-react";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import { MethodBadge } from "@/components/shared/method-badge";
import type { RunConfig } from "@/types/engine/RunConfig";
import { ConfirmHostDialog } from "./confirm-host-dialog";
import type { Endpoint } from "@/types/engine/Endpoint";
import { Label } from "@/components/workspace/fields";
import { useRunEvents } from "@/hooks/use-run-events";
import { useRunStore } from "@/stores/run-store";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Checkbox } from "@/components/ui/checkbox";
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
type LoadModeType = "closed" | "open" | "breakpoint" | "spike" | "ratelimit" | "soak";

const TESTS: { kind: TestKind; label: string }[] = [
  { kind: "latency", label: "Latency Probe" },
  { kind: "load", label: "Load Test" },
  { kind: "complexity", label: "Big-O (complexity)" },
  { kind: "payload", label: "Payload scaling" },
  { kind: "concurrency", label: "Concurrency (race)" },
  { kind: "timeout", label: "Timeout behaviour" },
  { kind: "fake", label: "Fake (no traffic)" },
];

/** The Select's value for "no token request chosen" (Base UI needs a non-empty value). */
const NO_TOKEN = "__kestrel_no_token__";

/** The Select's value for "no baseline" (Base UI needs a non-empty value). */
const NO_BASELINE = "__kestrel_no_baseline__";

/** The engine's limits on one concurrency run, whatever the in-flight cap. */
const MAX_BURST = 1_000;
const MAX_ROUNDS = 50;
/** The engine's limits on a timeout run's burst. */
const MAX_ABANDONED = 1_000;
const MAX_BURST_CONCURRENCY = 200;

/** Methods that change something, which is what a concurrency test is for. */
const WRITE_METHODS = ["POST", "PUT", "PATCH", "DELETE"];

/** Above this rate with keep-alive off, the client can run out of ephemeral ports (TIME_WAIT). */
const PORT_EXHAUSTION_RPS = 200;

/** Matches the engine's size generators: {{n}}, {{n:int_array}}, {{n:string}}, {{n:object_array}}. */
const SIZE_GENERATOR = /{{s*n(:(int_array|string|object_array))?s*}}/;

const usesSize = (e: Endpoint) =>
  [e.url, ...e.query.map((q) => q.value), ...e.headers.map((h) => h.value), ...bodyTexts(e)].some(
    (s) => SIZE_GENERATOR.test(s),
  );

/** Whether the request carries credentials: auth, or a header that usually holds them. */
const usesCredentials = (e: Endpoint) =>
  e.auth.type !== "none" ||
  e.headers.some(
    (h) => h.enabled && ["authorization", "cookie", "x-api-key"].includes(h.key.toLowerCase()),
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

/** The rate of each breakpoint step: start, then +percent (at least +1) up to max. Same as the engine. */
const breakpointRates = (start: number, percent: number, max: number) => {
  const rates = [Math.max(1, start)];
  while (rates.at(-1)! < max && rates.length < 1000) {
    const last = rates.at(-1)!;
    rates.push(Math.min(max, Math.max(last + 1, Math.ceil(last * (1 + percent / 100)))));
  }
  return rates;
};

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
  // Breakpoint
  startRate: 10,
  stepPercent: 50,
  stepS: 10,
  maxRate: 500,
  maxErrorPct: 5,
  /** 0 = no p99 limit. */
  maxP99Ms: 1000,
  // Spike
  baseRate: 20,
  spikeRate: 200,
  beforeS: 10,
  spikeS: 5,
  afterS: 20,
  // Concurrency
  burst: 10,
  burstRounds: 3,
  pauseMs: 500,
  // Multi-endpoint mix (load tests): endpoint id → weight; the selected endpoint is always in it.
  mixOn: false,
  mixWeights: {} as Record<string, number>,
  // Token refresh (load tests): an endpoint whose On Response rules pick out a token.
  refreshOn: false,
  refreshId: "",
  refreshMin: 5,
  // Soak
  soakMin: 30,
  // Timeout behaviour
  probes: 20,
  abandoned: 100,
  abandonConcurrency: 20,
  /** 0 = a quarter of the probes' median. */
  tightMs: 0,
  // Payload scaling (bytes; the Big-O settings below are shared)
  payloadMin: 1_000,
  payloadMax: 1_000_000,
  /** Big-O and payload scaling: an echo endpoint to send the same bodies to ("" = none). */
  baselineId: "",
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
  /** Endpoints that can supply a token: ones with an enabled On Response rule. */
  // Derived outside the selector: a selector that builds a new array on every call never settles,
  // so React re-renders forever.
  const collections = useWorkspaceStore((s) => s.workspace?.collections);
  const tokenEndpoints = useMemo(
    () =>
      (collections ?? [])
        .flatMap((c) => c.endpoints)
        .filter((e) => (e.extract ?? []).some((r) => r.enabled && r.name.trim() !== "")),
    [collections],
  );
  /** The selected endpoint's collection: where a load test's mix comes from. */
  const collection = useWorkspaceStore(
    (s) =>
      s.workspace?.collections.find((c) => c.endpoints.some((e) => e.id === endpoint?.id)) ?? null,
  );
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
    startRate,
    stepPercent,
    stepS,
    maxRate,
    maxErrorPct,
    maxP99Ms,
    baseRate,
    spikeRate,
    beforeS,
    spikeS,
    afterS,
    minN,
    maxN,
    sizes,
    rounds,
    slowMs,
    budgetS,
    burst,
    burstRounds,
    pauseMs,
    soakMin,
    mixOn,
    mixWeights,
    refreshOn,
    refreshId,
    refreshMin,
    payloadMin,
    payloadMax,
    baselineId,
    probes,
    abandoned,
    abandonConcurrency,
    tightMs,
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

  // Big-O and payload scaling share the sweep settings, except the size range: n for Big-O, bytes
  // for payload scaling, which start from different defaults.
  const [minKey, maxKey] =
    kind === "payload" ? (["payloadMin", "payloadMax"] as const) : (["minN", "maxN"] as const);
  const sweepMin = kind === "payload" ? payloadMin : minN;
  const sweepMax = kind === "payload" ? payloadMax : maxN;

  const buildConfig = (): RunConfig => {
    if (kind === "fake") return { kind, durationMs: Math.round(durationS * 1000) };
    if (!endpoint) throw new Error("Select an endpoint first.");
    if (kind === "timeout") {
      return {
        kind,
        endpointId: endpoint.id,
        environment,
        samples: probes,
        abandoned,
        concurrency: abandonConcurrency,
        tightMs,
        timeoutMs,
      };
    }
    if (kind === "concurrency") {
      return {
        kind,
        endpointId: endpoint.id,
        environment,
        requests: burst,
        rounds: burstRounds,
        pauseMs,
        timeoutMs,
      };
    }
    const common = {
      endpointId: endpoint.id,
      environment,
      timeoutMs,
      keepAlive,
      okStatuses: parseStatuses(okStatuses),
    };
    if (kind === "latency") return { kind, ...common, warmup, samples };
    if (kind === "complexity" || kind === "payload") {
      return {
        kind,
        ...common,
        minN: sweepMin,
        maxN: sweepMax,
        points: sizes,
        samples: rounds,
        baselineEndpointId:
          baselineId && collection?.endpoints.some((e) => e.id === baselineId) ? baselineId : null,
        warmup,
        slowMs,
        budgetMs: Math.round(budgetS * 1000),
      };
    }
    // Fetched at the start, every few minutes and on 401s; off unless a token request is chosen.
    const tokenRefresh =
      refreshOn && tokenEndpoints.some((e) => e.id === refreshId)
        ? { endpointId: refreshId, everyMs: Math.round(refreshMin * 60_000) }
        : null;
    // The selected endpoint plus the others ticked, by weight; none without a mix.
    const mixEntries =
      mixOn && collection
        ? collection.endpoints
            .filter((e) => e.id === endpoint.id || (mixWeights[e.id] ?? 0) > 0)
            .map((e) => ({ endpointId: e.id, weight: Math.max(1, mixWeights[e.id] ?? 1) }))
        : [];
    if (mode === "ratelimit") {
      const steps = breakpointRates(startRate, stepPercent, maxRate).length;
      return {
        kind,
        ...common,
        mix: mixEntries,
        tokenRefresh,
        mode: {
          type: "rateLimit",
          startRate,
          stepPercent,
          stepMs: Math.round(stepS * 1000),
          maxRate,
        },
        durationMs: Math.round(steps * stepS * 1000),
        rampUpMs: 0,
        maxInFlight,
      };
    }
    if (mode === "spike") {
      return {
        kind,
        ...common,
        mix: mixEntries,
        tokenRefresh,
        mode: {
          type: "spike",
          baseRate,
          spikeRate,
          beforeMs: Math.round(beforeS * 1000),
          spikeMs: Math.round(spikeS * 1000),
          afterMs: Math.round(afterS * 1000),
        },
        durationMs: Math.round((beforeS + spikeS + afterS) * 1000),
        rampUpMs: 0,
        maxInFlight,
      };
    }
    if (mode === "breakpoint") {
      const steps = breakpointRates(startRate, stepPercent, maxRate).length;
      return {
        kind,
        ...common,
        mix: mixEntries,
        tokenRefresh,
        mode: {
          type: "breakpoint",
          startRate,
          stepPercent,
          stepMs: Math.round(stepS * 1000),
          maxRate,
          maxErrorPct,
          maxP99Ms: maxP99Ms > 0 ? maxP99Ms : null,
        },
        // The engine stops at its duration cap if the steps would run longer.
        durationMs: Math.round(steps * stepS * 1000),
        rampUpMs: 0,
        maxInFlight,
      };
    }
    if (mode === "soak") {
      return {
        kind,
        ...common,
        mix: mixEntries,
        tokenRefresh,
        mode: { type: "soak", rate },
        durationMs: Math.round(soakMin * 60_000),
        rampUpMs: Math.round(rampS * 1000),
        maxInFlight,
      };
    }
    return {
      kind,
      ...common,
      mix: mixEntries,
      tokenRefresh,
      mode: mode === "closed" ? { type: "closed", concurrency } : { type: "open", rate },
      durationMs: Math.round(durationS * 1000),
      rampUpMs: Math.round(rampS * 1000),
      maxInFlight: mode === "open" ? maxInFlight : null,
    };
  };

  const plan = breakpointRates(startRate, stepPercent, maxRate);

  const caps = health.data?.caps;
  /** Settings over this engine's caps, which the engine would refuse. Empty until caps are known. */
  const overCaps: string[] = [];
  if (caps && kind !== "fake") {
    const check = (value: number, cap: number, what: string, unit = "") => {
      if (value > cap)
        overCaps.push(
          `${what} ${value.toLocaleString()}${unit} (cap ${cap.toLocaleString()}${unit})`,
        );
    };
    const shaped =
      kind === "load" && (mode === "breakpoint" || mode === "ratelimit" || mode === "spike");
    check(timeoutMs, caps.maxTimeoutMs, "timeout", " ms");
    if (kind === "timeout") {
      check(probes, caps.maxSamples, "probes");
      check(abandoned, MAX_ABANDONED, "requests to abandon");
      check(
        abandonConcurrency,
        Math.min(caps.maxInFlight, MAX_BURST_CONCURRENCY),
        "abandoned at once",
      );
    }
    if (kind === "concurrency") {
      check(burst, Math.min(caps.maxInFlight, MAX_BURST), "requests per round");
      check(burstRounds, MAX_ROUNDS, "rounds");
    }
    if (kind === "latency") {
      check(samples, caps.maxSamples, "samples");
      check(warmup, caps.maxWarmup, "warm-up");
    }
    if (kind === "complexity" || kind === "payload") {
      check(rounds, caps.maxSamples, "rounds");
      check(warmup, caps.maxWarmup, "warm-up");
      check(sweepMax, caps.maxN, kind === "payload" ? "largest size" : "largest n");
      check(sizes, caps.maxPoints, "sizes");
      check(budgetS, caps.maxSweepS, "time budget", " s");
      check(slowMs, caps.maxTimeoutMs, "slow limit", " ms");
    }
    if (kind === "load") {
      if (mode === "open" || mode === "soak") check(rate, caps.maxRps, "rate", " req/s");
      if (mode === "soak") check(soakMin * 60, caps.maxDurationS, "soak length", " s");
      if (mode === "closed") check(concurrency, caps.maxInFlight, "users");
      if (mode === "breakpoint" || mode === "ratelimit") {
        check(startRate, caps.maxRps, "start rate", " req/s");
        check(maxRate, caps.maxRps, "max rate", " req/s");
      }
      if (mode === "spike") {
        check(baseRate, caps.maxRps, "base rate", " req/s");
        check(spikeRate, caps.maxRps, "spike rate", " req/s");
        check(beforeS + spikeS + afterS, caps.maxDurationS, "total length", " s");
      }
      if (mode !== "closed") check(maxInFlight, caps.maxInFlight, "max in flight");
      if (!shaped && mode !== "soak") check(durationS, caps.maxDurationS, "duration", " s");
    }
  }
  if (caps && kind === "fake") {
    if (durationS > caps.maxDurationS)
      overCaps.push(`duration ${durationS} s (cap ${caps.maxDurationS} s)`);
  }
  /** Breakpoint and rate-limit runs stop at the duration cap rather than being refused. */
  const cappedSteps =
    caps &&
    kind === "load" &&
    (mode === "breakpoint" || mode === "ratelimit") &&
    plan.length * stepS > caps.maxDurationS
      ? Math.max(1, Math.floor(caps.maxDurationS / stepS))
      : null;

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
      <div className="flex items-center justify-between border-b px-5 py-2">
        <h2 className="text-sm font-medium">Run Test</h2>
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
                  <SelectItem value="breakpoint">Breakpoint: step up until it breaks</SelectItem>
                  <SelectItem value="spike">Spike: a burst, then see it recover</SelectItem>
                  <SelectItem value="ratelimit">Rate limit: find where 429s start</SelectItem>
                  <SelectItem value="soak">Soak: steady load for a long time</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            {mode === "spike" ? (
              <>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Base rate (req/s)">
                    <Input
                      type="number"
                      min={1}
                      value={baseRate}
                      disabled={running}
                      onChange={num("baseRate")}
                    />
                  </Field>
                  <Field label="Spike rate (req/s)">
                    <Input
                      type="number"
                      min={1}
                      value={spikeRate}
                      disabled={running}
                      onChange={num("spikeRate")}
                    />
                  </Field>
                  <Field label="Before (s)">
                    <Input
                      type="number"
                      min={2}
                      value={beforeS}
                      disabled={running}
                      onChange={num("beforeS")}
                    />
                  </Field>
                  <Field label="Spike (s)">
                    <Input
                      type="number"
                      min={1}
                      value={spikeS}
                      disabled={running}
                      onChange={num("spikeS")}
                    />
                  </Field>
                  <Field label="After (s)">
                    <Input
                      type="number"
                      min={1}
                      value={afterS}
                      disabled={running}
                      onChange={num("afterS")}
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
                </div>
                <p className="text-muted-foreground text-xs">
                  {beforeS} s at {baseRate} req/s, {spikeS} s at {spikeRate} req/s, then {afterS} s
                  at {baseRate} req/s ({beforeS + spikeS + afterS} s in all). Recovery is measured
                  in the last part, so give it enough time.
                </p>
              </>
            ) : mode === "breakpoint" || mode === "ratelimit" ? (
              <>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Start rate (req/s)">
                    <Input
                      type="number"
                      min={1}
                      value={startRate}
                      disabled={running}
                      onChange={num("startRate")}
                    />
                  </Field>
                  <Field label="Max rate (req/s)">
                    <Input
                      type="number"
                      min={1}
                      value={maxRate}
                      disabled={running}
                      onChange={num("maxRate")}
                    />
                  </Field>
                  <Field label="Step (+%)">
                    <Input
                      type="number"
                      min={1}
                      value={stepPercent}
                      disabled={running}
                      onChange={num("stepPercent")}
                    />
                  </Field>
                  <Field label="Step length (s)">
                    <Input
                      type="number"
                      min={1}
                      value={stepS}
                      disabled={running}
                      onChange={num("stepS")}
                    />
                  </Field>
                  {mode === "breakpoint" && (
                    <Field label="Max errors (%)">
                      <Input
                        type="number"
                        min={0}
                        max={100}
                        step="any"
                        value={maxErrorPct}
                        disabled={running}
                        onChange={num("maxErrorPct")}
                      />
                    </Field>
                  )}
                  {mode === "breakpoint" && (
                    <Field label="Max p99 (ms, 0 = off)">
                      <Input
                        type="number"
                        min={0}
                        value={maxP99Ms}
                        disabled={running}
                        onChange={num("maxP99Ms")}
                      />
                    </Field>
                  )}
                  <Field label="Max in flight">
                    <Input
                      type="number"
                      min={1}
                      value={maxInFlight}
                      disabled={running}
                      onChange={num("maxInFlight")}
                    />
                  </Field>
                </div>
                <p className="text-muted-foreground text-xs">
                  {plan.length} steps: {plan.slice(0, 4).join(" → ")}
                  {plan.length > 4 ? ` → … → ${plan.at(-1)}` : ""} req/s, up to{" "}
                  {Math.round(plan.length * stepS)} s.{" "}
                  {mode === "ratelimit"
                    ? "Stops at the first step where more than 1% of requests get 429."
                    : "Stops at the first step over a limit."}{" "}
                  The run also stops at the engine&apos;s duration cap.
                </p>
              </>
            ) : (
              <div className="grid grid-cols-2 gap-3">
                {mode === "open" || mode === "soak" ? (
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
                {mode === "soak" ? (
                  <Field label="Duration (min)">
                    <Input
                      type="number"
                      min={1}
                      value={soakMin}
                      disabled={running}
                      onChange={num("soakMin")}
                    />
                  </Field>
                ) : (
                  <Field label="Duration (s)">
                    <Input
                      type="number"
                      min={1}
                      max={caps?.maxDurationS}
                      value={durationS}
                      disabled={running}
                      onChange={num("durationS")}
                    />
                  </Field>
                )}
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
            )}
            <label className="flex items-center justify-between">
              <span>Keep a token fresh</span>
              <Switch
                checked={refreshOn}
                disabled={running}
                onCheckedChange={(checked) => set("refreshOn", !!checked)}
              />
            </label>
            {refreshOn &&
              (tokenEndpoints.length === 0 ? (
                <p className="bg-muted text-muted-foreground rounded-xs p-3 text-xs">
                  Add an On Response rule to the request that gets a token (for example, save{" "}
                  <code>access_token</code> from the body as the <code>token</code> secret), then
                  choose it here.
                </p>
              ) : (
                <div className="flex flex-col gap-1.5">
                  <div className="grid grid-cols-[1fr_auto] gap-3">
                    <Field label="Token request">
                      <Select
                        value={refreshId || NO_TOKEN}
                        disabled={running}
                        onValueChange={(v) => set("refreshId", v === NO_TOKEN ? "" : String(v))}
                        items={[
                          { value: NO_TOKEN, label: "Choose…" },
                          ...tokenEndpoints.map((e) => ({
                            value: e.id,
                            label: `${e.method} ${e.name || e.url}`,
                          })),
                        ]}
                      >
                        <SelectTrigger className="w-full text-xs">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={NO_TOKEN}>Choose…</SelectItem>
                          {tokenEndpoints.map((e) => (
                            <SelectItem key={e.id} value={e.id}>
                              {e.method} {e.name || e.url}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </Field>
                    <Field label="Every (min)">
                      <Input
                        type="number"
                        min={1}
                        className="w-20"
                        value={refreshMin}
                        disabled={running}
                        onChange={num("refreshMin")}
                      />
                    </Field>
                  </div>
                  <p className="text-muted-foreground text-xs">
                    Sent before the first request, then on this schedule, and straight away if the
                    run starts getting 401s. Its On Response rules save the new token, and the run
                    switches to it without stopping. A token saved as a variable lasts for this run
                    only; save it as a secret to keep it.
                  </p>
                </div>
              ))}
            {endpoint && collection && collection.endpoints.length > 1 && (
              <>
                <label className="flex items-center justify-between">
                  <span>Mix with other endpoints</span>
                  <Switch
                    checked={mixOn}
                    disabled={running}
                    onCheckedChange={(checked) => set("mixOn", !!checked)}
                  />
                </label>
                {mixOn && (
                  <div className="flex flex-col gap-1.5">
                    <p className="text-muted-foreground text-xs">
                      Each request goes to one of these, in proportion to its weight. They must all
                      be on the same host as this endpoint.
                    </p>
                    {collection.endpoints.map((e) => {
                      const self = e.id === endpoint.id;
                      const weight = mixWeights[e.id] ?? (self ? 1 : 0);
                      const setWeight = (w: number) =>
                        set("mixWeights", { ...mixWeights, [e.id]: w });
                      return (
                        <div key={e.id} className="flex min-w-0 items-center gap-2">
                          <Checkbox
                            checked={self || weight > 0}
                            disabled={self || running}
                            onCheckedChange={(checked) => setWeight(checked ? 1 : 0)}
                            aria-label={`Include ${e.name || e.url}`}
                          />
                          <MethodBadge method={e.method} short />
                          <span className="min-w-0 flex-1 truncate text-xs" title={e.url}>
                            {e.name || e.url}
                          </span>
                          {(self || weight > 0) && (
                            <Input
                              type="number"
                              min={1}
                              max={1000}
                              className="h-7 w-16 text-xs"
                              aria-label="Weight"
                              value={Math.max(1, weight)}
                              disabled={running}
                              onChange={(ev) => setWeight(Math.max(1, Number(ev.target.value)))}
                            />
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </>
            )}
          </>
        )}

        {kind === "fake" && (
          <Field label="Duration (s)">
            <Input
              type="number"
              min={1}
              max={caps?.maxDurationS}
              value={durationS}
              disabled={running}
              onChange={num("durationS")}
            />
          </Field>
        )}

        {(kind === "complexity" || kind === "payload") && (
          <>
            {endpoint && !usesSize(endpoint) && kind === "payload" && (
              <p className="bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                Size the payload with <code>{"{{n:string}}"}</code> in the body (n bytes of text),
                or <code>{"{{n}}"}</code> in a parameter that grows the response, such as a page
                size.
              </p>
            )}
            {endpoint && !usesSize(endpoint) && kind === "complexity" && (
              <p className="bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                Mark the input size in the request with <code>{"{{n}}"}</code> (e.g. a limit),{" "}
                <code>{"{{n:int_array}}"}</code>, <code>{"{{n:string}}"}</code> or{" "}
                <code>{"{{n:object_array}}"}</code> in the body. Each request gets fresh random
                contents.
              </p>
            )}
            <div className="grid grid-cols-2 gap-3">
              <Field label={kind === "payload" ? "Smallest size (bytes)" : "Smallest n"}>
                <Input
                  type="number"
                  min={1}
                  value={sweepMin}
                  disabled={running}
                  onChange={num(minKey)}
                />
              </Field>
              <Field label={kind === "payload" ? "Largest size (bytes)" : "Largest n"}>
                <Input
                  type="number"
                  min={1}
                  max={caps?.maxN}
                  value={sweepMax}
                  disabled={running}
                  onChange={num(maxKey)}
                />
              </Field>
              <Field label="Sizes">
                <Input
                  type="number"
                  min={3}
                  max={caps?.maxPoints}
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
                  max={caps?.maxSweepS}
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
            <Field label="Payload baseline (echo)">
              <Select
                value={baselineId || NO_BASELINE}
                disabled={running}
                onValueChange={(v) => set("baselineId", v === NO_BASELINE ? "" : String(v))}
                items={[
                  { value: NO_BASELINE, label: "None" },
                  ...(collection?.endpoints ?? [])
                    .filter((e) => e.id !== endpoint?.id)
                    .map((e) => ({ value: e.id, label: `${e.method} ${e.name || e.url}` })),
                ]}
              >
                <SelectTrigger className="w-full text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_BASELINE}>None</SelectItem>
                  {(collection?.endpoints ?? [])
                    .filter((e) => e.id !== endpoint?.id)
                    .map((e) => (
                      <SelectItem key={e.id} value={e.id}>
                        {e.method} {e.name || e.url}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>
              <p className="text-muted-foreground text-xs">
                Sends each body to this endpoint too (e.g. one that echoes it back), to show how
                much of the growth is just moving and parsing the bytes. Must be on the same host.
              </p>
            </Field>
            <p className="text-muted-foreground text-xs">
              Sizes are spaced geometrically and sampled in shuffled rounds. A size slower than the
              limit stops the sweep from growing further.
              {kind === "payload" &&
                " The results add MB/s and latency against the bytes sent and received, and the cost a request has whatever its size."}
            </p>
          </>
        )}

        {kind === "concurrency" && (
          <>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Requests per round">
                <Input
                  type="number"
                  min={2}
                  max={caps ? Math.min(caps.maxInFlight, MAX_BURST) : MAX_BURST}
                  value={burst}
                  disabled={running}
                  onChange={num("burst")}
                />
              </Field>
              <Field label="Rounds">
                <Input
                  type="number"
                  min={1}
                  max={MAX_ROUNDS}
                  value={burstRounds}
                  disabled={running}
                  onChange={num("burstRounds")}
                />
              </Field>
              <Field label="Pause between (ms)">
                <Input
                  type="number"
                  min={0}
                  value={pauseMs}
                  disabled={running}
                  onChange={num("pauseMs")}
                />
              </Field>
            </div>
            {endpoint && !WRITE_METHODS.includes(endpoint.method) && (
              <p className="bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                This test is for requests that change something (POST, PUT, PATCH, DELETE). It will
                still show whether simultaneous {endpoint.method} requests fail.
              </p>
            )}
            <p className="text-muted-foreground text-xs">
              Each round sends the same request {burst} times at once, then reads what came back:
              one success and the rest refused (409 and similar) means the server lets one write
              through; several successes or 5xx errors point to a race. Each round is rendered once,
              so put <code>{"{{uuid}}"}</code> or <code>{"{{seq}}"}</code> in a unique field to make
              each round a fresh attempt. This really sends these writes.
            </p>
          </>
        )}

        {kind === "timeout" && (
          <>
            <div className="grid grid-cols-2 gap-3">
              <Field label="Probes (before and after)">
                <Input
                  type="number"
                  min={1}
                  max={caps?.maxSamples}
                  value={probes}
                  disabled={running}
                  onChange={num("probes")}
                />
              </Field>
              <Field label="Requests to abandon">
                <Input
                  type="number"
                  min={1}
                  max={MAX_ABANDONED}
                  value={abandoned}
                  disabled={running}
                  onChange={num("abandoned")}
                />
              </Field>
              <Field label="Abandoned at once">
                <Input
                  type="number"
                  min={1}
                  max={
                    caps ? Math.min(caps.maxInFlight, MAX_BURST_CONCURRENCY) : MAX_BURST_CONCURRENCY
                  }
                  value={abandonConcurrency}
                  disabled={running}
                  onChange={num("abandonConcurrency")}
                />
              </Field>
              <Field label="Give up after (ms, 0 = auto)">
                <Input
                  type="number"
                  min={0}
                  value={tightMs}
                  disabled={running}
                  onChange={num("tightMs")}
                />
              </Field>
            </div>
            <p className="text-muted-foreground text-xs">
              Probes the endpoint one request at a time, then sends {abandoned} requests it gives up
              on mid-flight, then probes again. A probe that reaches the timeout below means the
              server hung; slower probes afterwards mean it kept working on requests nobody was
              waiting for. Auto gives up after a quarter of the probes&apos; median time.
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
              {kind !== "concurrency" && kind !== "timeout" && (
                <Field label="OK statuses">
                  <Input
                    placeholder="404, 409"
                    value={okStatuses}
                    disabled={running}
                    onChange={(e) => set("okStatuses", e.target.value)}
                  />
                </Field>
              )}
            </div>
            {kind !== "concurrency" && kind !== "timeout" && (
              <label className="flex items-center justify-between">
                <span>Keep-alive</span>
                <Switch
                  checked={keepAlive}
                  disabled={running}
                  onCheckedChange={(checked) => set("keepAlive", !!checked)}
                />
              </label>
            )}
          </>
        )}

        {overCaps.length > 0 && (
          <div
            role="alert"
            className="rounded-xs bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300"
          >
            Over this engine&apos;s limits: {overCaps.join(", ")}. Lower them, or raise the{" "}
            <code>KESTREL_MAX_*</code> settings where the engine runs.
          </div>
        )}
        {cappedSteps !== null && caps && (
          <p className="bg-muted text-muted-foreground rounded-xs p-3 text-xs">
            The {plan.length} steps would take {Math.round(plan.length * stepS)} s; this engine
            stops runs at {caps.maxDurationS} s, so it will get through about {cappedSteps} (up to{" "}
            {plan[cappedSteps - 1]} req/s) unless a step breaks first.
          </p>
        )}
        {caps && kind !== "fake" && (
          <p className="text-muted-foreground text-xs">
            This engine allows up to {caps.maxRps.toLocaleString()} req/s,{" "}
            {caps.maxInFlight.toLocaleString()} in flight, {caps.maxDurationS} s per run and{" "}
            {caps.maxTimeoutMs / 1000} s timeouts.
          </p>
        )}
        {kind === "load" && mode === "closed" && (
          <p className="bg-muted text-muted-foreground rounded-xs p-3 text-xs">
            Closed model: throughput falls when the server slows, so it understates how bad a stall
            is for real traffic. Use the open model to test a target rate.
          </p>
        )}
        {kind === "load" && mode === "soak" && caps && soakMin * 60 > caps.maxDurationS && (
          <p className="bg-muted text-muted-foreground rounded-xs p-3 text-xs">
            Soak runs usually last 30 minutes or more, and this engine stops runs at{" "}
            {caps.maxDurationS} s. Set <code>KESTREL_MAX_DURATION_S</code> (up to 7 days) where the
            engine runs and restart it, or shorten the soak.
          </p>
        )}
        {kind === "load" &&
          mode === "soak" &&
          !refreshOn &&
          endpoint &&
          usesCredentials(endpoint) && (
            <p className="bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
              This request sends credentials. A token that expires partway through a soak shows up
              as a wall of 401s: switch on <strong>Keep a token fresh</strong> above, or use one
              that outlives the run.
            </p>
          )}
        {kind === "load" &&
          (mode === "open" || mode === "soak") &&
          !keepAlive &&
          rate > PORT_EXHAUSTION_RPS && (
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
            overCaps.length > 0 ||
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
