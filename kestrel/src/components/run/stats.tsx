"use client";

import { ShieldAlert, ShieldCheck } from "lucide-react";

import type { ContractSummary } from "@/types/engine/ContractSummary";
import type { PhaseSummary } from "@/types/engine/PhaseSummary";
import type { BreakpointResult } from "@/types/engine/BreakpointResult";
import type { SpikeResult } from "@/types/engine/SpikeResult";
import type { RateLimitResult } from "@/types/engine/RateLimitResult";
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
  const isComplexity = useRunStore((s) => s.config?.kind === "complexity");
  if (isComplexity) return <ComplexityLive />;

  return (
    <div className="flex flex-col gap-6">
      <StatGrid cols={3}>
        <StatTile
          label="p50 latency"
          value={ms(last?.p50Ms)}
          unit="ms"
          info="Median of the last 250 ms window"
        />
        <StatTile
          label="p99 latency"
          value={ms(last?.p99Ms)}
          unit="ms"
          info="99th percentile of the last window"
        />
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
