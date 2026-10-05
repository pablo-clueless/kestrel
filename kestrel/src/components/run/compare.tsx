"use client";

import { useQuery } from "@tanstack/react-query";
import { TriangleAlert } from "lucide-react";

import { StatusBadge } from "@/components/shared/method-badge";
import { useWorkspaceStore } from "@/stores/workspace-store";
import type { RunReport } from "@/types/engine/RunReport";
import type { RunConfig } from "@/types/engine/RunConfig";
import { errorMessage, getReport } from "@/lib/client";
import type { RunKind } from "@/types/engine/RunKind";
import { CircleLoader } from "@/components/shared";
import { int, ms } from "@/lib/format";
import { cn } from "@/lib/utils";

const KIND_LABEL: Record<RunKind, string> = {
  latency: "Latency probe",
  load: "Load test",
  complexity: "Big-O",
  concurrency: "Concurrency",
  timeout: "Timeout behaviour",
  payload: "Payload scaling",
  fake: "Fake",
};

/** A change smaller than this (relative) is within run-to-run noise and shown as "≈". */
const NOISE = 0.05;
/** …unless it's also under this many ms, for latencies: 0.2 → 0.3 ms is +50% and still nothing. */
const NOISE_MS = 1;

const when = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  hour: "2-digit",
  minute: "2-digit",
});

/** Two runs side by side, older first ("before"), so a change reads as what the newer one did. */
export const CompareRuns = ({ runIds }: { runIds: [string, string] }) => {
  const a = useQuery({ queryKey: ["report", runIds[0]], queryFn: () => getReport(runIds[0]) });
  const b = useQuery({ queryKey: ["report", runIds[1]], queryFn: () => getReport(runIds[1]) });

  if (a.isPending || b.isPending) {
    return (
      <div className="grid h-40 place-items-center">
        <CircleLoader />
      </div>
    );
  }
  if (a.isError || b.isError) {
    return <p className="text-destructive text-sm">{errorMessage(a.error ?? b.error)}</p>;
  }
  const [before, after] =
    a.data.startedAtMs <= b.data.startedAtMs ? [a.data, b.data] : [b.data, a.data];
  return <Comparison before={before} after={after} />;
};

const Comparison = ({ before, after }: { before: RunReport; after: RunReport }) => {
  const caveats = differences(before, after);
  const rows = metrics(before, after);
  const keys = [...keyResults(before), ...keyResults(after)]
    .map(([label]) => label)
    .filter((label, i, all) => all.indexOf(label) === i);
  const beforeKeys = new Map(keyResults(before));
  const afterKeys = new Map(keyResults(after));

  return (
    <div className="flex flex-col gap-5 text-sm">
      <div className="grid grid-cols-2 gap-4">
        <RunCard label="Before" report={before} />
        <RunCard label="After" report={after} />
      </div>

      {caveats.length > 0 && (
        <div className="flex gap-2 rounded-xs bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-950 dark:text-amber-300">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          <span>
            These runs differ in {caveats.join(", ")}, so some of the change may come from that
            rather than the target.
          </span>
        </div>
      )}

      <table className="w-full tabular-nums">
        <thead className="text-muted-foreground text-xs">
          <tr className="border-b">
            <th className="py-2 text-left font-normal">Metric</th>
            <th className="py-2 text-right font-normal">Before</th>
            <th className="py-2 text-right font-normal">After</th>
            <th className="py-2 text-right font-normal">Change</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.label} className="border-b last:border-0">
              <td className="text-muted-foreground py-1.5">{row.label}</td>
              <td className="py-1.5 text-right">{row.format(row.before)}</td>
              <td className="py-1.5 text-right font-medium">{row.format(row.after)}</td>
              <td className="py-1.5 text-right">
                <Change row={row} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      <div className="grid grid-cols-2 gap-4">
        <Statuses report={before} />
        <Statuses report={after} />
      </div>

      {keys.length > 0 && (
        <table className="w-full text-sm">
          <thead className="text-muted-foreground text-xs">
            <tr className="border-b">
              <th className="py-2 text-left font-normal">Result</th>
              <th className="py-2 text-left font-normal">Before</th>
              <th className="py-2 text-left font-normal">After</th>
            </tr>
          </thead>
          <tbody>
            {keys.map((label) => (
              <tr key={label} className="border-b align-top last:border-0">
                <td className="text-muted-foreground py-1.5 pr-3">{label}</td>
                <td className="py-1.5 pr-3">{beforeKeys.get(label) ?? "–"}</td>
                <td className="py-1.5">{afterKeys.get(label) ?? "–"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
};

const RunCard = ({ label, report }: { label: string; report: RunReport }) => {
  const endpointId = "endpointId" in report.config ? report.config.endpointId : null;
  const endpoint = useWorkspaceStore((s) =>
    endpointId
      ? (s.workspace?.collections.flatMap((c) => c.endpoints).find((e) => e.id === endpointId) ??
        null)
      : null,
  );
  const seconds = (report.finishedAtMs - report.startedAtMs) / 1000;
  return (
    <div className="bg-muted flex min-w-0 flex-col gap-0.5 rounded-xs p-3 text-xs">
      <span className="text-muted-foreground uppercase">{label}</span>
      <span className="truncate text-sm font-medium">
        {KIND_LABEL[report.config.kind]}
        {endpoint ? ` · ${endpoint.name || endpoint.url}` : ""}
      </span>
      <span className="text-muted-foreground">
        {when.format(report.startedAtMs)} · {seconds.toFixed(1)} s · {settings(report.config)}
      </span>
    </div>
  );
};

/** The settings that shape the numbers, in a few words. */
const settings = (c: RunConfig): string => {
  switch (c.kind) {
    case "fake":
      return "no traffic";
    case "latency":
      return `${c.samples} samples`;
    case "load":
      switch (c.mode.type) {
        case "closed":
          return `${c.mode.concurrency} users`;
        case "open":
        case "soak":
          return `${c.mode.rate} req/s${c.mode.type === "soak" ? " soak" : ""}`;
        case "breakpoint":
        case "rateLimit":
          return `${c.mode.startRate}→${c.mode.maxRate} req/s steps`;
        case "spike":
          return `${c.mode.baseRate}→${c.mode.spikeRate} req/s spike`;
      }
      return "load";
    case "complexity":
    case "payload":
      return `n ${int(c.minN)}–${int(c.maxN)}`;
    case "concurrency":
      return `${c.requests} at once × ${c.rounds}`;
    case "timeout":
      return `${c.abandoned} abandoned`;
  }
};

/** What makes the two runs not like for like. */
const differences = (a: RunReport, b: RunReport): string[] => {
  const out: string[] = [];
  if (a.config.kind !== b.config.kind) out.push("test type");
  const endpoint = (r: RunReport) => ("endpointId" in r.config ? r.config.endpointId : null);
  if (endpoint(a) !== endpoint(b)) out.push("endpoint");
  if (a.config.kind === b.config.kind && settings(a.config) !== settings(b.config)) {
    out.push("settings");
  }
  if (a.target && b.target && a.target.pinnedIp !== b.target.pinnedIp) out.push("target address");
  return out;
};

interface Row {
  label: string;
  before: number | null;
  after: number | null;
  format: (v: number | null) => string;
  /** Whether a bigger number is better (throughput) or worse (latency, errors). */
  higherIsBetter: boolean;
  /** Latencies: changes under `NOISE_MS` are noise whatever their percentage. */
  isMs?: boolean;
}

const errorPct = (r: RunReport) =>
  r.totalRequests ? (r.totalErrors / r.totalRequests) * 100 : null;
const msCell = (v: number | null) => (v === null ? "–" : `${ms(v)} ms`);

const metrics = (a: RunReport, b: RunReport): Row[] => {
  const lat = (k: "p50Ms" | "p90Ms" | "p99Ms" | "maxMs" | "meanMs", label: string): Row => ({
    label,
    before: a.latency?.[k] ?? null,
    after: b.latency?.[k] ?? null,
    format: msCell,
    higherIsBetter: false,
    isMs: true,
  });
  const rows: Row[] = [
    {
      label: "Requests",
      before: a.totalRequests,
      after: b.totalRequests,
      format: (v) => int(v),
      higherIsBetter: true,
    },
    {
      label: "Errors",
      before: errorPct(a),
      after: errorPct(b),
      format: (v) => (v === null ? "–" : `${v.toFixed(1)}%`),
      higherIsBetter: false,
    },
    {
      label: "Throughput",
      before: a.meanRps || null,
      after: b.meanRps || null,
      format: (v) => (v === null ? "–" : `${int(v)} req/s`),
      higherIsBetter: true,
    },
    lat("p50Ms", "p50"),
    lat("p90Ms", "p90"),
    lat("p99Ms", "p99"),
    lat("maxMs", "Max"),
    lat("meanMs", "Mean"),
    {
      label: "TTFB p50",
      before: a.ttfb?.p50Ms ?? null,
      after: b.ttfb?.p50Ms ?? null,
      format: msCell,
      higherIsBetter: false,
      isMs: true,
    },
  ];
  return rows.filter((r) => r.before !== null || r.after !== null);
};

const Change = ({ row }: { row: Row }) => {
  if (row.before === null || row.after === null)
    return <span className="text-muted-foreground">–</span>;
  const delta = row.after - row.before;
  const relative = row.before !== 0 ? delta / Math.abs(row.before) : delta === 0 ? 0 : Infinity;
  const noise =
    Math.abs(relative) < NOISE || (row.isMs === true && Math.abs(delta) < NOISE_MS) || delta === 0;
  if (noise) return <span className="text-muted-foreground">≈</span>;
  const better = row.higherIsBetter ? delta > 0 : delta < 0;
  const sign = delta > 0 ? "+" : "−";
  const pct = Number.isFinite(relative) ? ` (${sign}${Math.abs(relative * 100).toFixed(0)}%)` : "";
  const amount = row.isMs ? `${ms(Math.abs(delta))} ms` : row.format(Math.abs(delta));
  return (
    <span
      className={cn(
        "font-medium",
        better ? "text-green-700 dark:text-green-400" : "text-red-600 dark:text-red-400",
      )}
      title={better ? "Better" : "Worse"}
    >
      {sign}
      {amount}
      {pct}
    </span>
  );
};

const Statuses = ({ report }: { report: RunReport }) =>
  report.statusCounts.length > 0 ? (
    <div className="flex flex-wrap gap-1">
      {report.statusCounts.map((s) => (
        <StatusBadge key={s.status} status={s.status}>
          <span className="ml-1 font-normal opacity-70">×{s.count.toLocaleString()}</span>
        </StatusBadge>
      ))}
    </div>
  ) : (
    <span className="text-muted-foreground text-xs">No responses</span>
  );

/** The few results particular to a test type, as (label, text) pairs lined up by label. */
const keyResults = (r: RunReport): [string, string][] => {
  const out: [string, string][] = [];
  const rate = (v: number | null) => (v === null ? "–" : `${int(v)} req/s`);
  if (r.complexity) {
    const { analysis, payload } = r.complexity;
    out.push(["Big-O verdict", `${analysis.verdict} (${analysis.confidence})`]);
    if (analysis.slope !== null) out.push(["Log-log slope", analysis.slope.toFixed(2)]);
    if (payload?.mbPerS != null)
      out.push(["Throughput (payload)", `${payload.mbPerS.toFixed(1)} MB/s`]);
    if (payload?.fixedMs != null) out.push(["Fixed cost (payload)", `${ms(payload.fixedMs)} ms`]);
  }
  if (r.breakpoint) {
    out.push(["Broke at", rate(r.breakpoint.brokeAtRate)]);
    out.push(["Last step that held", rate(r.breakpoint.heldRate)]);
  }
  if (r.rateLimit) {
    out.push(["429s from", rate(r.rateLimit.limitedAtRate)]);
    out.push(["Went through", rate(r.rateLimit.heldRate)]);
  }
  if (r.spike) {
    out.push([
      "Recovery after the spike",
      r.spike.recoveryMs === null ? "not recovered" : `${(r.spike.recoveryMs / 1000).toFixed(1)} s`,
    ]);
  }
  const top = (findings: { message: string }[] | undefined) => findings?.[0]?.message;
  const finding = top(r.concurrency?.findings) ?? top(r.timeout?.findings) ?? top(r.soak?.findings);
  if (finding) out.push(["Main finding", finding]);
  if (r.timeout?.recoveredAfterMs != null) {
    out.push(["Recovered after", `${(r.timeout.recoveredAfterMs / 1000).toFixed(1)} s`]);
  }
  return out;
};
