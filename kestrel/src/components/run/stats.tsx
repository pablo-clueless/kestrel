"use client";

import { useRunStore } from "@/stores/run-store";

const Stat = ({ label, value }: { label: string; value: string }) => (
  <div>
    <p className="text-text-gray text-xs uppercase">{label}</p>
    <p className="font-mono text-2xl">{value}</p>
  </div>
);

/** Latest bucket's numbers. */
export const LatencyStats = () => {
  const last = useRunStore((s) => s.buckets.at(-1));
  const fmt = (ms?: number) => (ms === undefined ? "–" : `${ms.toFixed(1)}ms`);
  const errPct = last && last.requests > 0 ? (last.errors / last.requests) * 100 : undefined;
  const isLoad = useRunStore((s) => s.config?.kind === "load");

  return (
    <div className="grid grid-cols-2 gap-4">
      <Stat label="p50" value={fmt(last?.p50Ms)} />
      <Stat label="p99" value={fmt(last?.p99Ms)} />
      <Stat label="req/s" value={last ? Math.round(last.rps).toLocaleString() : "–"} />
      <Stat label="errors" value={errPct === undefined ? "–" : `${errPct.toFixed(1)}%`} />
      {isLoad && (
        <>
          <Stat label="in flight" value={last ? last.inFlight.toLocaleString() : "–"} />
          <Stat label="dropped" value={last ? last.dropped.toLocaleString() : "–"} />
          <Stat label="lag" value={fmt(last?.lagMs)} />
        </>
      )}
    </div>
  );
};

/** Final report, fetched over REST after `finished`. */
export const ReportSummary = () => {
  const report = useRunStore((s) => s.report);
  if (!report) return <p className="text-text-gray text-sm">Appears when a run finishes.</p>;

  const seconds = (report.finishedAtMs - report.startedAtMs) / 1000;
  const errorClasses = Object.entries(report.errorCounts).filter(([, n]) => n > 0);
  const firstError = report.samples.find((s) => s.error)?.error;

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-2 gap-4">
        <Stat label="status" value={report.status} />
        <Stat label="duration" value={`${seconds.toFixed(1)}s`} />
        <Stat label="requests" value={report.totalRequests.toLocaleString()} />
        <Stat label="errors" value={report.totalErrors.toLocaleString()} />
      </div>
      {report.latency && (
        <table className="w-full font-mono text-xs">
          <thead className="text-text-gray">
            <tr>
              <th className="text-left font-normal" />
              <th className="text-right font-normal">all</th>
              <th className="text-right font-normal">ok</th>
              <th className="text-right font-normal">ttfb</th>
            </tr>
          </thead>
          <tbody>
            {(["minMs", "p50Ms", "p90Ms", "p99Ms", "maxMs", "meanMs"] as const).map((k) => (
              <tr key={k}>
                <td className="text-text-gray">{k.replace("Ms", "")}</td>
                <td className="text-right">{ms(report.latency?.[k])}</td>
                <td className="text-right">{ms(report.latencySuccess?.[k])}</td>
                <td className="text-right">{ms(report.ttfb?.[k])}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {(report.dropped > 0 || report.generatorLag) && (
        <p className="text-xs">
          <span className="text-text-gray">dropped </span>
          <span className="font-mono">{report.dropped.toLocaleString()}</span>
          {report.generatorLag && (
            <>
              <span className="text-text-gray"> · generator lag p99 </span>
              <span className="font-mono">{ms(report.generatorLag.p99Ms)}</span>
            </>
          )}
        </p>
      )}
      {report.config.kind === "latency" && report.coldMs !== null && (
        <p className="text-xs">
          <span className="text-text-gray">cold (1st request) </span>
          <span className="font-mono">{ms(report.coldMs)}</span>
        </p>
      )}
      {report.statusCounts.length > 0 && (
        <p className="font-mono text-xs">{report.statusCounts.map((s) => `${s.status}×${s.count}`).join("  ")}</p>
      )}
      {errorClasses.length > 0 && (
        <p className="text-xs text-red-600">
          {errorClasses.map(([k, n]) => `${k}: ${n}`).join(" · ")}
          {firstError && <span className="block break-all">{firstError}</span>}
        </p>
      )}
      {report.notes.map((n) => (
        <p key={n} className="text-text-gray text-xs">
          {n}
        </p>
      ))}
    </div>
  );
};

const ms = (v: number | null | undefined) => (v == null ? "–" : v < 10 ? v.toFixed(2) : v.toFixed(1));
