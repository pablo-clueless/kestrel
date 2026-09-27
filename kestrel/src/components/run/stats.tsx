"use client";

import { StatGrid, StatTile } from "@/components/shared/stat-tile";
import { StatusBadge } from "@/components/shared/method-badge";
import { useRunStore } from "@/stores/run-store";

import { DistributionChart, LiveChart } from "./charts";

const ms = (v: number | null | undefined) => (v == null ? "–" : v < 10 ? v.toFixed(2) : v.toFixed(1));
const int = (v: number | null | undefined) => (v == null ? "–" : Math.round(v).toLocaleString());

/** Latest window's numbers above the live chart. */
export const LiveSummary = () => {
  const last = useRunStore((s) => s.buckets.at(-1));
  const isLoad = useRunStore((s) => s.config?.kind === "load");
  const errPct = last && last.requests > 0 ? (last.errors / last.requests) * 100 : undefined;

  return (
    <div className="flex flex-col gap-6">
      <StatGrid cols={3}>
        <StatTile label="p50 latency" value={ms(last?.p50Ms)} unit="ms" info="Median of the last 250 ms window" />
        <StatTile label="p99 latency" value={ms(last?.p99Ms)} unit="ms" info="99th percentile of the last window" />
        <StatTile label="Throughput" value={int(last?.rps)} unit="req/s" />
        <StatTile
          label="Errors"
          value={errPct === undefined ? "–" : errPct.toFixed(1)}
          unit="%"
          tone={errPct ? "bad" : "default"}
        />
        {isLoad ? (
          <>
            <StatTile label="In flight" value={int(last?.inFlight)} />
            <StatTile
              label="Dropped"
              value={int(last?.dropped)}
              tone={last?.dropped ? "bad" : "default"}
              info="Scheduled sends skipped at the in-flight cap (open model)"
            />
          </>
        ) : (
          <StatTile label="Requests" value={int(last?.requests)} info="In the last window" />
        )}
      </StatGrid>
      <div className="border-t pt-5">
        <LiveChart />
      </div>
    </div>
  );
};

/** Final report, fetched over REST after `finished`. */
export const ResultsSummary = () => {
  const report = useRunStore((s) => s.report);
  if (!report) {
    return (
      <p className="text-muted-foreground text-sm">
        Percentiles, the distribution, status codes and error classes appear when a run finishes.
      </p>
    );
  }

  const seconds = (report.finishedAtMs - report.startedAtMs) / 1000;
  const errorClasses = Object.entries(report.errorCounts).filter(([, n]) => n > 0);
  const firstError = report.samples.find((s) => s.error)?.error;
  const errPct = report.totalRequests ? (report.totalErrors / report.totalRequests) * 100 : 0;

  return (
    <div className="flex flex-col gap-6">
      <StatGrid cols={4}>
        <StatTile label="Requests" value={int(report.totalRequests)} />
        <StatTile label="Errors" value={errPct.toFixed(1)} unit="%" tone={errPct ? "bad" : "default"} />
        <StatTile label="Mean throughput" value={int(report.meanRps)} unit="req/s" />
        <StatTile label="Duration" value={seconds.toFixed(1)} unit="s" />
        <StatTile label="p50" value={ms(report.latency?.p50Ms)} unit="ms" />
        <StatTile label="p90" value={ms(report.latency?.p90Ms)} unit="ms" />
        <StatTile label="p99" value={ms(report.latency?.p99Ms)} unit="ms" />
        {report.config.kind === "load" ? (
          <StatTile
            label="Dropped"
            value={int(report.dropped)}
            tone={report.dropped ? "bad" : "default"}
            info="Open model: sends skipped at the in-flight cap. Not in the latency numbers."
          />
        ) : (
          <StatTile label="Cold request" value={ms(report.coldMs)} unit="ms" info="The very first request of the run" />
        )}
      </StatGrid>

      {report.latency && (
        <div className="grid grid-cols-2 gap-6 border-t pt-5">
          <table className="w-full self-start text-sm tabular-nums">
            <thead className="text-muted-foreground text-xs">
              <tr className="border-b">
                <th className="py-2 text-left font-normal">Percentile</th>
                <th className="py-2 text-right font-normal">All</th>
                <th className="py-2 text-right font-normal">Successful</th>
                <th className="py-2 text-right font-normal">TTFB</th>
              </tr>
            </thead>
            <tbody>
              {(["minMs", "p50Ms", "p90Ms", "p99Ms", "maxMs", "meanMs"] as const).map((k) => (
                <tr key={k} className="border-b last:border-0">
                  <td className="text-muted-foreground py-1.5">{k.replace("Ms", "")}</td>
                  <td className="py-1.5 text-right font-medium">{ms(report.latency?.[k])}</td>
                  <td className="py-1.5 text-right">{ms(report.latencySuccess?.[k])}</td>
                  <td className="py-1.5 text-right">{ms(report.ttfb?.[k])}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <DistributionChart />
        </div>
      )}

      {(report.statusCounts.length > 0 || errorClasses.length > 0) && (
        <div className="flex flex-col gap-3 border-t pt-5 text-sm">
          {report.statusCounts.length > 0 && (
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-muted-foreground mr-1 text-xs">Status codes</span>
              {report.statusCounts.map((s) => (
                <StatusBadge key={s.status} status={s.status}>
                  <span className="ml-1 font-normal opacity-70">×{s.count.toLocaleString()}</span>
                </StatusBadge>
              ))}
            </div>
          )}
          {errorClasses.length > 0 && (
            <div className="text-destructive text-xs">
              {errorClasses.map(([k, n]) => `${k}: ${n}`).join(" · ")}
              {firstError && <span className="text-muted-foreground mt-1 block break-all">{firstError}</span>}
            </div>
          )}
        </div>
      )}

      {report.notes.length > 0 && (
        <ul className="bg-muted text-muted-foreground flex flex-col gap-1.5 rounded-lg p-3 text-xs">
          {report.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
};
