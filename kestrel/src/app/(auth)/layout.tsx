import { Feather } from "lucide-react";
import React from "react";

interface Props {
  children: React.ReactNode;
}

/** Light streaks on the art panel: [left %, top %, height %]. */
const STREAKS: [number, number, number][] = [
  [8, 6, 22],
  [17, 58, 30],
  [29, 12, 14],
  [44, 72, 20],
  [61, 4, 18],
  [73, 64, 26],
  [86, 18, 34],
  [93, 70, 16],
];

export default function AuthLayout({ children }: Props) {
  return (
    <div className="bg-card grid min-h-dvh w-full grid-cols-1 lg:h-dvh lg:grid-cols-2 lg:overflow-hidden">
      <div className="flex min-h-dvh flex-col px-6 py-10 lg:min-h-0">
        <div className="flex items-center justify-center gap-2 text-lg font-semibold">
          <Feather className="text-primary size-6" /> Kestrel
        </div>
        <main className="flex flex-1 items-center justify-center py-10">
          <div className="w-full max-w-sm">{children}</div>
        </main>
        <p className="text-muted-foreground mx-auto max-w-md text-center text-xs leading-relaxed">
          Load test your APIs from the browser. Sign in to pick up your collections, run history and
          reports on any device.
        </p>
      </div>

      <div
        aria-hidden
        className="from-accent via-primary/25 to-primary/70 relative hidden overflow-hidden bg-linear-to-b lg:block"
      >
        {STREAKS.map(([left, top, height]) => (
          <span
            key={`${left}-${top}`}
            className="absolute w-0.5 rounded-full bg-linear-to-b from-white/0 via-white/60 to-white/0"
            style={{ left: `${left}%`, top: `${top}%`, height: `${height}%` }}
          />
        ))}

        <div className="absolute inset-0 grid place-items-center">
          <div className="relative">
            <div className="absolute -inset-16 rounded-full bg-white/50 blur-3xl dark:bg-white/10" />

            <div className="from-primary relative grid size-60 rotate-[-8deg] place-items-center rounded-[3rem] bg-linear-to-br to-[#c2470a] shadow-[0_40px_80px_-20px_rgba(194,71,10,0.55),inset_0_2px_0_rgba(255,255,255,0.35)]">
              <div className="grid size-44 place-items-center rounded-[2.25rem] bg-linear-to-br from-white/25 to-white/0 shadow-[inset_0_-6px_16px_rgba(0,0,0,0.15)]">
                <Feather className="size-24 text-white drop-shadow-lg" strokeWidth={1.5} />
              </div>
            </div>

            <div className="bg-card/85 absolute -top-6 -right-28 rounded-2xl px-4 py-3 shadow-xl backdrop-blur">
              <p className="text-muted-foreground text-[11px]">Throughput</p>
              <p className="font-mono text-lg font-bold">1,240 rps</p>
              <svg viewBox="0 0 100 24" className="text-primary mt-1 h-6 w-28">
                <polyline
                  points="0,18 12,15 24,17 36,10 48,12 60,7 72,9 84,4 100,6"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </div>

            <div className="bg-card/85 absolute -bottom-8 -left-24 flex items-center gap-3 rounded-2xl px-4 py-3 shadow-xl backdrop-blur">
              <span className="bg-success size-2 rounded-full" />
              <div>
                <p className="text-muted-foreground text-[11px]">p99 latency</p>
                <p className="font-mono text-lg font-bold">42 ms</p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
