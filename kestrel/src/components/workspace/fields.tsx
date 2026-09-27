"use client";

import { cn } from "@/lib/utils";

/** Plain form controls. The shadcn theme variables (--border, --ring, …) aren't defined yet, so
 * these use the project's own tokens instead of shadcn inputs. */

export const inputClass =
  "h-8 min-w-0 border border-secondary-3/30 bg-transparent px-2 text-sm outline-none focus:border-primary";

export const Input = ({ className, ...props }: React.ComponentProps<"input">) => (
  <input className={cn(inputClass, className)} {...props} />
);

export const Select = ({ className, ...props }: React.ComponentProps<"select">) => (
  <select className={cn(inputClass, "pr-1", className)} {...props} />
);

export const TextArea = ({ className, ...props }: React.ComponentProps<"textarea">) => (
  <textarea
    className={cn(inputClass, "h-40 w-full resize-y py-1 font-mono text-xs", className)}
    spellCheck={false}
    {...props}
  />
);

export const Label = ({ children, className }: { children: React.ReactNode; className?: string }) => (
  <span className={cn("text-muted-foreground text-xs", className)}>{children}</span>
);

export function Tabs<T extends string>({
  tabs,
  value,
  onChange,
}: {
  tabs: readonly { id: T; label: string; count?: number }[];
  value: T;
  onChange: (id: T) => void;
}) {
  // Segmented control: muted track, the active segment raised as a card.
  return (
    <div className="bg-muted inline-flex self-start rounded-lg p-0.5 text-xs">
      {tabs.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onChange(t.id)}
          className={cn(
            "rounded-md px-2.5 py-1 font-medium transition-colors",
            value === t.id ? "bg-card text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
          )}
        >
          {t.label}
          {t.count ? <span className="text-muted-foreground ml-1">{t.count}</span> : null}
        </button>
      ))}
    </div>
  );
}
