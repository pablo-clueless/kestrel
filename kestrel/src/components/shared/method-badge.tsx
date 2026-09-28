import { cn } from "cn";

const STYLES: Record<string, string> = {
  GET: "bg-green-100 text-green-700 ",
  POST: "bg-orange-100 text-orange-700",
  PUT: "bg-blue-100 text-blue-700",
  PATCH: "bg-purple-100 text-purple-700",
  DELETE: "bg-red-100 text-red-700",
};

const SHORT: Record<string, string> = { DELETE: "DEL", OPTIONS: "OPT", PATCH: "PATCH" };

/** Tinted method pill. `short` abbreviates long methods for tight lists. */
export const MethodBadge = ({
  method,
  short,
  className,
}: {
  method: string;
  short?: boolean;
  className?: string;
}) => (
  <span
    className={cn(
      "inline-flex h-5 shrink-0 items-center justify-center rounded px-1.5 font-mono text-[10px] font-bold",
      STYLES[method] ?? "bg-muted text-muted-foreground",
      className,
    )}
  >
    {short ? (SHORT[method] ?? method) : method}
  </span>
);

/** Status code pill: green 2xx, amber 3xx/4xx, red 5xx. */
export const StatusBadge = ({
  status,
  children,
}: {
  status: number;
  children?: React.ReactNode;
}) => (
  <span
    className={cn(
      "inline-flex h-5 items-center rounded px-1.5 font-mono text-[11px] font-semibold",
      status < 300 && "bg-green-50 text-green-700 dark:bg-green-950 dark:text-green-400",
      status >= 300 &&
        status < 500 &&
        "bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-400",
      status >= 500 && "bg-red-50 text-red-700 dark:bg-red-950 dark:text-red-400",
    )}
  >
    {status}
    {children}
  </span>
);
