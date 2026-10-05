"use client";

import {
  CircleAlert,
  CircleCheck,
  Info,
  ShieldAlert,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";

import type { ContractSummary } from "@/types/engine/ContractSummary";
import type { PhaseSummary } from "@/types/engine/PhaseSummary";
import type { BreakpointResult } from "@/types/engine/BreakpointResult";
import type { SpikeResult } from "@/types/engine/SpikeResult";
import type { RateLimitResult } from "@/types/engine/RateLimitResult";
import type { ConcurrencyResult } from "@/types/engine/ConcurrencyResult";
import type { TimeoutResult } from "@/types/engine/TimeoutResult";
import type { SoakResult } from "@/types/engine/SoakResult";
import type { PayloadResult } from "@/types/engine/PayloadResult";
import type { MixStats } from "@/types/engine/MixStats";
import type { Finding } from "@/types/engine/Finding";
import type { FindingLevel } from "@/types/engine/FindingLevel";
import { StatGrid, StatTile } from "@/components/shared/stat-tile";
import { ComplexityLive, ComplexityResults } from "./complexity";
import { StatusBadge } from "@/components/shared/method-badge";
import { DistributionChart, LiveChart } from "./charts";
import { useRunStore } from "@/stores/run-store";
import { int, ms } from "@/lib/format";
import { cn } from "@/lib/utils";

/** Latest window's numbers above the live chart. */
export const LiveSummary = () => {
  const last = useRunStore((s) => s.buckets.at(-1));
  const isLoad = useRunStore((s) => s.config?.kind === "load");
  const errPct = last && last.requests > 0 ? (last.errors / last.requests) * 100 : undefined;
  const isComplexity = useRunStore(
    (s) => s.config?.kind === "complexity" || s.config?.kind === "payload",
  );
  const isConcurrency = useRunStore((s) => s.config?.kind === "concurrency");
  const isTimeout = useRunStore((s) => s.config?.kind === "timeout");
  const roundsDone = useRunStore((s) => s.buckets.length);
  if (isComplexity) return <ComplexityLive />;

  return (
    <div className="flex flex-col gap-6">
      <StatGrid cols={3}>
        <StatTile
          label="p50 latency"
          value={ms(last?.p50Ms)}
          unit="ms"
          info={isConcurrency ? "Median of the last round" : "Median of the last 250 ms window"}
        />
        <StatTile
          label="p99 latency"
          value={ms(last?.p99Ms)}
          unit="ms"
          info={
            isConcurrency
              ? "99th percentile of the last round"
              : "99th percentile of the last window"
          }
        />
        {isConcurrency || isTimeout ? (
          <StatTile
            label={isTimeout ? "Probe phases done" : "Rounds done"}
            value={int(roundsDone)}
          />
        ) : (
          <StatTile label="Throughput" value={int(last?.rps)} unit="req/s" />
        )}
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
      <div className="h-fit border-t pt-5">
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

  if (report.complexity) {
    return (
      <div className="flex flex-col gap-6">
        {report.complexity.payload && <PayloadSection result={report.complexity.payload} />}
        <ComplexityResults result={report.complexity} />
        <Notes notes={report.notes} />
      </div>
    );
  }

  const seconds = (report.finishedAtMs - report.startedAtMs) / 1000;
  const errorClasses = Object.entries(report.errorCounts).filter(([, n]) => n > 0);
  const firstError = report.samples.find((s) => s.error)?.error;
  const errPct = report.totalRequests ? (report.totalErrors / report.totalRequests) * 100 : 0;

  const breakpointLimit =
    report.config.kind === "load" && report.config.mode.type === "breakpoint"
      ? report.config.mode.maxP99Ms
      : null;

  return (
    <div className="flex flex-col gap-6">
      {report.mix && <MixSection mix={report.mix} />}
      {report.concurrency && <ConcurrencySection result={report.concurrency} />}
      {report.timeout && <TimeoutSection result={report.timeout} />}
      {report.soak && <SoakSection result={report.soak} />}
      {report.spike && <SpikeSection result={report.spike} />}
      {report.rateLimit && <RateLimitSection result={report.rateLimit} />}
      {report.breakpoint && (
        <BreakpointSection result={report.breakpoint} p99Limit={breakpointLimit} />
      )}
      <StatGrid cols={4}>
        <StatTile label="Requests" value={int(report.totalRequests)} />
        <StatTile
          label="Errors"
          value={errPct.toFixed(1)}
          unit="%"
          tone={errPct ? "bad" : "default"}
        />
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
          <StatTile
            label="Cold request"
            value={ms(report.coldMs)}
            unit="ms"
            info="The very first request of the run"
          />
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

      {report.phases && (
        <PhasesSection
          phases={report.phases}
          isLoad={report.config.kind === "load"}
          keepAlive={"keepAlive" in report.config ? report.config.keepAlive : undefined}
        />
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
              {firstError && (
                <span className="text-muted-foreground mt-1 block break-all">{firstError}</span>
              )}
            </div>
          )}
        </div>
      )}

      {report.contract && <ContractSection contract={report.contract} />}

      {report.notes.length > 0 && (
        <ul className="bg-muted text-muted-foreground flex flex-col gap-1.5 rounded-xs p-3 text-xs">
          {report.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
    </div>
  );
};

const FINDING_STYLE: Record<FindingLevel, { icon: typeof Info; className: string }> = {
  bad: { icon: CircleAlert, className: "bg-red-50 text-red-800 dark:bg-red-950 dark:text-red-300" },
  warn: {
    icon: TriangleAlert,
    className: "bg-amber-50 text-amber-800 dark:bg-amber-950 dark:text-amber-300",
  },
  good: {
    icon: CircleCheck,
    className: "bg-green-50 text-green-800 dark:bg-green-950 dark:text-green-300",
  },
  info: { icon: Info, className: "bg-muted text-muted-foreground" },
};

/** A test's plain-language findings, worst first, coloured by level. */
const Findings = ({ findings }: { findings: Finding[] }) => (
  <ul className="flex flex-col gap-2">
    {findings.map((f) => {
      const { icon: Icon, className } = FINDING_STYLE[f.level];
      return (
        <li key={f.message} className={cn("flex gap-2 rounded-xs p-3 text-xs", className)}>
          <Icon className="mt-0.5 size-3.5 shrink-0" />
          {f.message}
        </li>
      );
    })}
  </ul>
);

/** Multi-endpoint load runs: each endpoint's share and numbers. The rest of the report is combined. */
const MixSection = ({ mix }: { mix: MixStats[] }) => {
  const total = mix.reduce((sum, m) => sum + m.requests, 0);
  return (
    <table className="w-full text-sm tabular-nums">
      <thead className="text-muted-foreground text-xs">
        <tr className="border-b">
          <th className="py-2 text-left font-normal">Endpoint</th>
          <th className="py-2 text-right font-normal" title="Share of requests (weight)">
            Share
          </th>
          <th className="py-2 text-right font-normal">Errors</th>
          <th className="py-2 text-right font-normal">p50</th>
          <th className="py-2 text-right font-normal">p99</th>
          <th className="py-2 pl-4 text-left font-normal">Statuses</th>
        </tr>
      </thead>
      <tbody>
        {mix.map((m) => (
          <tr key={m.endpointId} className="border-b last:border-0">
            <td className="max-w-48 truncate py-1.5 font-medium" title={m.name}>
              {m.name}
            </td>
            <td className="py-1.5 text-right">
              {total ? ((m.requests / total) * 100).toFixed(0) : 0}%
              <span className="text-muted-foreground"> ({m.weight})</span>
            </td>
            <td className={cn("py-1.5 text-right", m.errors > 0 && "text-destructive")}>
              {m.requests ? ((m.errors / m.requests) * 100).toFixed(1) : "0.0"}%
            </td>
            <td className="py-1.5 text-right">{ms(m.latency?.p50Ms)}</td>
            <td className="py-1.5 text-right">{ms(m.latency?.p99Ms)}</td>
            <td className="py-1.5 pl-4">
              <span className="flex flex-wrap gap-1">
                {m.statusCounts.map((s) => (
                  <StatusBadge key={s.status} status={s.status}>
                    <span className="ml-1 font-normal opacity-70">×{s.count.toLocaleString()}</span>
                  </StatusBadge>
                ))}
              </span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};

const bytes = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n >= 1e3 ? `${(n / 1e3).toFixed(1)} kB` : `${n} B`;

/** Payload scaling: the Big-O points read as bytes, with throughput at each size. */
const PayloadSection = ({ result }: { result: PayloadResult }) => (
  <div className="flex flex-col gap-4">
    <Findings findings={result.findings} />
    <StatGrid cols={3}>
      <StatTile
        label="Fixed cost"
        value={ms(result.fixedMs ?? undefined)}
        unit="ms"
        info="What a request costs whatever its size"
      />
      <StatTile
        label="Effective throughput"
        value={result.mbPerS === null ? "–" : result.mbPerS.toFixed(1)}
        unit={result.mbPerS === null ? undefined : "MB/s"}
        info="For the bytes on top of the fixed cost, from a straight line through the sizes"
      />
      <StatTile
        label="Best at one size"
        value={result.peakMbPerS === null ? "–" : result.peakMbPerS.toFixed(1)}
        unit={result.peakMbPerS === null ? undefined : "MB/s"}
      />
    </StatGrid>
    <table className="w-full max-w-2xl text-sm tabular-nums">
      <thead className="text-muted-foreground text-xs">
        <tr className="border-b">
          <th className="py-2 text-left font-normal">n</th>
          <th className="py-2 text-right font-normal" title="Request plus response body, median">
            Bytes
          </th>
          <th className="py-2 text-right font-normal">Median</th>
          <th className="py-2 text-right font-normal">MB/s</th>
        </tr>
      </thead>
      <tbody>
        {result.points.map((p) => (
          <tr key={p.n} className="border-b last:border-0">
            <td className="py-1.5 font-medium">{int(p.n)}</td>
            <td className="py-1.5 text-right">{bytes(p.bytes)}</td>
            <td className="py-1.5 text-right">{ms(p.medianMs)} ms</td>
            <td className="py-1.5 text-right">{p.mbPerS.toFixed(1)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  </div>
);

/** Soak runs: what drifted. The windows are the report's timeline, so the chart covers the run. */
const SoakSection = ({ result }: { result: SoakResult }) => {
  const worst = Math.max(0, ...result.windows.map((w) => w.p99Ms));
  return (
    <div className="flex flex-col gap-4">
      <Findings findings={result.findings} />
      <StatGrid cols={3}>
        <StatTile label="Windows" value={int(result.windows.length)} />
        <StatTile label="Window" value={(result.windowMs / 1000).toFixed(0)} unit="s" />
        <StatTile label="Worst window p99" value={ms(worst)} unit="ms" />
      </StatGrid>
      <p className="text-muted-foreground text-xs">
        The timeline shows the whole run, one point per {(result.windowMs / 1000).toFixed(0)} s
        window.
      </p>
    </div>
  );
};

/** Timeout runs: probes before and after a burst of abandoned requests. */
const TimeoutSection = ({ result }: { result: TimeoutResult }) => (
  <div className="flex flex-col gap-4">
    <Findings findings={result.findings} />
    <StatGrid cols={4}>
      <StatTile
        label="p50 before → after"
        value={`${ms(result.before?.p50Ms)} → ${ms(result.after?.p50Ms)}`}
        unit="ms"
      />
      <StatTile
        label="p99 before → after"
        value={`${ms(result.before?.p99Ms)} → ${ms(result.after?.p99Ms)}`}
        unit="ms"
      />
      <StatTile
        label="Recovered after"
        value={result.recoveredAfterMs === null ? "–" : (result.recoveredAfterMs / 1000).toFixed(1)}
        unit={result.recoveredAfterMs === null ? undefined : "s"}
        info="From the end of the burst until probes were back to normal for good"
      />
      <StatTile
        label="Hung"
        value={int(result.hung)}
        tone={result.hung ? "bad" : "default"}
        info="Probes that got no answer before the timeout"
      />
      <StatTile
        label="Abandoned"
        value={int(result.abandoned - result.answeredInTime)}
        info={`Of ${result.abandoned} burst requests, given up on after ${result.tightMs} ms`}
      />
      <StatTile label="Give-up time" value={int(result.tightMs)} unit="ms" />
      <StatTile
        label="Fast failures"
        value={int(result.fastFailures)}
        info="Probes that failed well inside the timeout"
      />
    </StatGrid>
  </div>
);

/** Concurrency runs: what the server made of identical simultaneous requests, round by round. */
const ConcurrencySection = ({ result }: { result: ConcurrencyResult }) => (
  <div className="flex flex-col gap-4">
    <Findings findings={result.findings} />
    <table className="w-full max-w-2xl text-sm tabular-nums">
      <thead className="text-muted-foreground text-xs">
        <tr className="border-b">
          <th className="py-2 text-left font-normal">Round</th>
          <th className="py-2 text-left font-normal">Responses</th>
          <th className="py-2 text-right font-normal" title="2xx responses">
            Succeeded
          </th>
          <th
            className="py-2 text-right font-normal"
            title="Different bodies among the 2xx responses"
          >
            Distinct
          </th>
          <th
            className="py-2 text-right font-normal"
            title="How long every request was in flight at once"
          >
            Overlap
          </th>
        </tr>
      </thead>
      <tbody>
        {result.rounds.map((r) => (
          <tr key={r.round} className="border-b last:border-0">
            <td className="py-1.5 font-medium">{r.round}</td>
            <td className="py-1.5">
              <span className="flex flex-wrap gap-1">
                {r.statuses.map((s) => (
                  <StatusBadge key={s.status} status={s.status}>
                    <span className="ml-1 font-normal opacity-70">×{s.count}</span>
                  </StatusBadge>
                ))}
                {r.failed > 0 && (
                  <span className="text-destructive text-xs">{r.failed} no response</span>
                )}
              </span>
            </td>
            <td className="py-1.5 text-right">{int(r.succeeded)}</td>
            <td className="py-1.5 text-right">{int(r.distinctBodies)}</td>
            <td className={cn("py-1.5 text-right", r.overlapMs <= 0 && "text-amber-600")}>
              {ms(r.overlapMs)} ms
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  </div>
);

/** Rate-limit discovery: where 429s started, and what the API says about its limit. */
const RateLimitSection = ({ result }: { result: RateLimitResult }) => {
  const limited = result.limitedAtRate !== null;
  const headerList = (title: string, headers: [string, string][]) =>
    headers.length > 0 && (
      <div className="flex flex-col gap-1">
        <span className="text-muted-foreground text-xs">{title}</span>
        <dl className="bg-muted grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 rounded-xs p-3 font-mono text-xs">
          {headers.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted-foreground">{k}</dt>
              <dd className="break-all">{v}</dd>
            </div>
          ))}
        </dl>
      </div>
    );
  return (
    <div className="flex flex-col gap-4">
      <StatGrid cols={3}>
        <StatTile
          label={limited ? "429s from" : "No 429s up to"}
          value={limited ? int(result.limitedAtRate) : int(result.steps.at(-1)?.rate)}
          unit="req/s"
          tone={limited ? "bad" : "default"}
        />
        <StatTile
          label="Went through"
          value={result.heldRate === null ? "–" : int(result.heldRate)}
          unit="req/s"
          info="The highest step with (almost) no 429s"
        />
        <StatTile label="Steps" value={int(result.steps.length)} />
      </StatGrid>
      <table className="w-full max-w-2xl text-sm tabular-nums">
        <thead className="text-muted-foreground text-xs">
          <tr className="border-b">
            <th className="py-2 text-left font-normal">Rate</th>
            <th className="py-2 text-right font-normal">Achieved</th>
            <th className="py-2 text-right font-normal">429s</th>
            <th className="py-2 text-right font-normal">p99</th>
          </tr>
        </thead>
        <tbody>
          {result.steps.map((s) => (
            <tr key={s.rate} className="border-b last:border-0">
              <td className="py-1.5 font-medium">{int(s.rate)}</td>
              <td className="py-1.5 text-right">{s.achievedRps.toFixed(1)}</td>
              <td className={cn("py-1.5 text-right", s.errorPct > 0 && "text-destructive")}>
                {s.errorPct.toFixed(1)}%
              </td>
              <td className="py-1.5 text-right">{ms(s.p99Ms)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {headerList("Rate-limit headers on the first 429", result.limitedHeaders)}
      {headerList("Rate-limit headers on other responses", result.okHeaders)}
      {result.limitedHeaders.length === 0 && result.okHeaders.length === 0 && (
        <p className="text-muted-foreground text-xs">
          No rate-limit headers (Retry-After, X-RateLimit-*, RateLimit-*) were seen.
        </p>
      )}
    </div>
  );
};

/** Spike runs: the baseline, the burst and after it, and how long recovery took. */
const SpikeSection = ({ result }: { result: SpikeResult }) => {
  const rows = [
    { label: "Before", phase: result.baseline },
    { label: "Spike", phase: result.spike },
    { label: "After", phase: result.after },
  ];
  const recovered = result.recoveryMs !== null;
  return (
    <div className="flex flex-col gap-4">
      <StatGrid cols={3}>
        <StatTile
          label="Recovered after"
          value={recovered ? (result.recoveryMs! / 1000).toFixed(0) : "Not yet"}
          unit={recovered ? "s" : undefined}
          tone={recovered ? "default" : "bad"}
          info="Seconds after the burst until p99 and errors were back near the baseline, and stayed there"
        />
        <StatTile
          label="p99 during the spike"
          value={ms(result.spike?.p99Ms)}
          unit="ms"
          info={`Baseline p99 ${ms(result.baseline?.p99Ms)} ms`}
        />
        <StatTile
          label="Errors during the spike"
          value={result.spike ? result.spike.errorPct.toFixed(1) : "–"}
          unit="%"
          tone={result.spike?.errorPct ? "bad" : "default"}
        />
      </StatGrid>
      <table className="w-full max-w-2xl text-sm tabular-nums">
        <thead className="text-muted-foreground text-xs">
          <tr className="border-b">
            <th className="py-2 text-left font-normal">Part</th>
            <th className="py-2 text-right font-normal">Requests</th>
            <th className="py-2 text-right font-normal">Achieved</th>
            <th className="py-2 text-right font-normal">p50</th>
            <th className="py-2 text-right font-normal">p99</th>
            <th className="py-2 text-right font-normal">Errors</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ label, phase }) => (
            <tr key={label} className="border-b last:border-0">
              <td className="py-1.5">{label}</td>
              <td className="py-1.5 text-right">{int(phase?.requests)}</td>
              <td className="py-1.5 text-right">{phase ? phase.achievedRps.toFixed(1) : "–"}</td>
              <td className="py-1.5 text-right">{ms(phase?.p50Ms)}</td>
              <td className="py-1.5 text-right font-medium">{ms(phase?.p99Ms)}</td>
              <td className={cn("py-1.5 text-right", phase?.errorPct && "text-destructive")}>
                {phase ? `${phase.errorPct.toFixed(1)}%` : "–"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-muted-foreground text-xs">
        &quot;Before&quot; leaves out its first second (warm-up). The live chart above shows the
        whole run, second by second.
      </p>
    </div>
  );
};

/** Breakpoint runs: the rate that held, where it broke, and every step on the way. */
const BreakpointSection = ({
  result,
  p99Limit,
}: {
  result: BreakpointResult;
  p99Limit: number | null;
}) => {
  const broke = result.steps.find((s) => !s.passed);
  // Bars are scaled to the slowest step, or the limit if that's higher, so the limit line fits.
  const scale = Math.max(p99Limit ?? 0, ...result.steps.map((s) => s.p99Ms), 1);
  return (
    <div className="flex flex-col gap-4">
      <StatGrid cols={3}>
        <StatTile
          label="Held"
          value={result.heldRate === null ? "–" : int(result.heldRate)}
          unit="req/s"
          info="The highest step that stayed within the limits"
        />
        <StatTile
          label={broke ? "Broke at" : "Didn't break"}
          value={broke ? int(broke.rate) : int(result.steps.at(-1)?.rate)}
          unit="req/s"
          tone={broke ? "bad" : "default"}
          info={broke?.reason ?? "Every step stayed within the limits"}
        />
        <StatTile label="Steps" value={int(result.steps.length)} />
      </StatGrid>
      <table className="w-full text-sm tabular-nums">
        <thead className="text-muted-foreground text-xs">
          <tr className="border-b">
            <th className="py-2 text-left font-normal">Rate</th>
            <th className="py-2 text-right font-normal">Achieved</th>
            <th className="py-2 text-right font-normal">p50</th>
            <th className="py-2 text-right font-normal">p99</th>
            <th className="w-1/4 py-2 pl-4 text-left font-normal">
              p99{p99Limit ? ` (limit ${int(p99Limit)} ms)` : ""}
            </th>
            <th className="py-2 text-right font-normal">Errors</th>
            <th className="py-2 pl-4 text-left font-normal">Result</th>
          </tr>
        </thead>
        <tbody>
          {result.steps.map((s) => (
            <tr key={s.rate} className="border-b last:border-0">
              <td className="py-1.5 font-medium">{int(s.rate)}</td>
              <td className="py-1.5 text-right">{s.achievedRps.toFixed(1)}</td>
              <td className="py-1.5 text-right">{ms(s.p50Ms)}</td>
              <td className="py-1.5 text-right">{ms(s.p99Ms)}</td>
              <td className="py-1.5 pl-4">
                <div className="bg-muted relative h-2 rounded-xs">
                  <div
                    className={cn("h-2 rounded-xs", s.passed ? "bg-primary" : "bg-destructive")}
                    style={{ width: `${Math.min(100, (s.p99Ms / scale) * 100)}%` }}
                  />
                  {p99Limit && (
                    <div
                      className="bg-foreground/60 absolute -top-0.5 h-3 w-px"
                      style={{ left: `${(p99Limit / scale) * 100}%` }}
                    />
                  )}
                </div>
              </td>
              <td className={cn("py-1.5 text-right", s.errorPct > 0 && "text-destructive")}>
                {s.errorPct.toFixed(1)}%
              </td>
              <td
                className={cn(
                  "py-1.5 pl-4 text-xs",
                  s.passed ? "text-success" : "text-destructive",
                )}
              >
                {s.passed ? "Held" : s.reason}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

const PHASES = [
  { key: "connect", label: "Connect (TCP + TLS)", swatch: "bg-primary/35" },
  { key: "waiting", label: "Waiting for the server", swatch: "bg-primary" },
  { key: "download", label: "Download", swatch: "bg-primary/65" },
] as const;

/** Where a request's time goes: connecting, the server, downloading (latency and load runs). */
const PhasesSection = ({
  phases,
  keepAlive,
  isLoad,
}: {
  phases: PhaseSummary;
  keepAlive?: boolean;
  isLoad: boolean;
}) => {
  const share = phases.requests ? phases.newConnections / phases.requests : 0;
  // Means add up, so they make an honest "average request". Connect is spread over every request,
  // since only some of them opened a connection.
  const parts = {
    connect: (phases.connect?.meanMs ?? 0) * share,
    waiting: phases.waiting?.meanMs ?? 0,
    download: phases.download?.meanMs ?? 0,
  };
  const total = parts.connect + parts.waiting + parts.download;
  const connectP50 = phases.connect?.p50Ms ?? 0;
  const waitingP50 = phases.waiting?.p50Ms ?? 0;
  const tip =
    keepAlive === false && share > 0.9 && connectP50 > waitingP50
      ? "Every request opens its own connection, and that takes longer than the server does. Turn keep-alive on to measure the server rather than connection setup."
      : null;

  return (
    <div className="flex flex-col gap-3 border-t pt-5 text-sm">
      <span className="text-muted-foreground text-xs">Where the time went</span>
      {total > 0 && (
        <div className="flex flex-col gap-2">
          <div
            className="bg-muted flex h-3 overflow-hidden rounded-xs"
            role="img"
            aria-label={PHASES.map((p) => `${p.label} ${ms(parts[p.key])} ms`).join(", ")}
          >
            {PHASES.map((p) => (
              <div
                key={p.key}
                className={p.swatch}
                style={{ width: `${(parts[p.key] / total) * 100}%` }}
              />
            ))}
          </div>
          <div className="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs tabular-nums">
            {PHASES.map((p) => (
              <span key={p.key} className="flex items-center gap-1.5">
                <span className={cn("size-2.5 rounded-xs", p.swatch)} />
                {p.label} {ms(parts[p.key])} ms
              </span>
            ))}
            <span>(an average request)</span>
          </div>
        </div>
      )}
      <table className="w-full max-w-xl text-sm tabular-nums">
        <thead className="text-muted-foreground text-xs">
          <tr className="border-b">
            <th className="py-2 text-left font-normal">Phase</th>
            <th className="py-2 text-right font-normal">p50</th>
            <th className="py-2 text-right font-normal">p99</th>
          </tr>
        </thead>
        <tbody>
          {phases.dnsMs !== null && (
            <tr className="border-b">
              <td className="py-1.5">
                DNS lookup <span className="text-muted-foreground text-xs">(once per run)</span>
              </td>
              <td className="py-1.5 text-right" colSpan={2}>
                {ms(phases.dnsMs)}
              </td>
            </tr>
          )}
          {PHASES.map((p) => (
            <tr key={p.key} className="border-b last:border-0">
              <td className="py-1.5">{p.label}</td>
              <td className="py-1.5 text-right font-medium">{ms(phases[p.key]?.p50Ms)}</td>
              <td className="py-1.5 text-right">{ms(phases[p.key]?.p99Ms)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-muted-foreground text-xs">
        {int(phases.newConnections)} of {int(phases.requests)} requests opened a new connection
        {phases.newConnections < phases.requests ? "; the rest reused one" : ""}.
        {isLoad && " Times are from sending, so they leave out any wait behind the in-flight cap."}
      </p>
      {tip && <p className="bg-muted rounded-xs p-3 text-xs">{tip}</p>}
    </div>
  );
};

/** How responses compared with the spec (imported endpoints only). */
const ContractSection = ({ contract }: { contract: ContractSummary }) => {
  const violations = contract.undeclaredStatus + contract.schemaMismatch;
  return (
    <div className="flex flex-col gap-3 border-t pt-5 text-sm">
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
        <span className="flex items-center gap-1.5 font-medium">
          {violations === 0 ? (
            <ShieldCheck className="text-success size-4" />
          ) : (
            <ShieldAlert className="text-destructive size-4" />
          )}
          Spec contract
        </span>
        <span className="text-muted-foreground">
          {int(contract.checked)} checked{contract.sampled ? " (sampled, ≤ 50/s)" : ""}
        </span>
        <span
          className={cn(contract.schemaMismatch ? "text-destructive" : "text-muted-foreground")}
        >
          {int(contract.schemaMismatch)} schema mismatch{contract.schemaMismatch === 1 ? "" : "es"}
        </span>
        <span
          className={cn(contract.undeclaredStatus ? "text-destructive" : "text-muted-foreground")}
        >
          {int(contract.undeclaredStatus)} undeclared status
          {contract.undeclaredStatus === 1 ? "" : "es"}
        </span>
      </div>
      {contract.examples.length > 0 && (
        <ul className="bg-muted flex flex-col gap-1 rounded-xs p-3 font-mono text-xs break-all">
          {contract.examples.map((m) => (
            <li key={m}>{m}</li>
          ))}
        </ul>
      )}
    </div>
  );
};

const Notes = ({ notes }: { notes: string[] }) =>
  notes.length > 0 ? (
    <ul className="bg-muted text-muted-foreground flex flex-col gap-1.5 p-3 text-xs">
      {notes.map((n) => (
        <li key={n}>{n}</li>
      ))}
    </ul>
  ) : null;
