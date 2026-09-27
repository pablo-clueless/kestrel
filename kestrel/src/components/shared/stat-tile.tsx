import { Info } from "lucide-react";

import { cn } from "cn";

interface Props {
  label: string;
  value: string;
  unit?: string;
  info?: string;
  tone?: "default" | "bad" | "good";
}

/** Small label, large value, small unit. */
export const StatTile = ({ label, value, unit, info, tone = "default" }: Props) => (
  <div className="flex flex-col gap-1.5">
    <p className="text-muted-foreground flex items-center gap-1 text-xs">
      {label}
      {info && (
        <span title={info}>
          <Info className="size-3" />
        </span>
      )}
    </p>
    <p
      className={cn(
        "text-2xl leading-none font-semibold tabular-nums",
        tone === "bad" && "text-destructive",
        tone === "good" && "text-success",
      )}
    >
      {value}
      {unit && value !== "–" && (
        <span className="text-muted-foreground ml-1 text-sm font-normal">{unit}</span>
      )}
    </p>
  </div>
);

/** Tiles in a grid with hairline dividers, like a summary panel. */
export const StatGrid = ({
  children,
  cols = 3,
}: {
  children: React.ReactNode;
  cols?: 2 | 3 | 4;
}) => (
  <div
    className={cn(
      "grid gap-x-6 gap-y-5",
      cols === 2 && "grid-cols-2",
      cols === 3 && "grid-cols-3",
      cols === 4 && "grid-cols-4",
    )}
  >
    {children}
  </div>
);
