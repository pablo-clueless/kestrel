import React from "react";
import { Info } from "lucide-react";

import { cn } from "cn";

interface Props {
  title: React.ReactNode;
  /** Shown as a tooltip on an info icon next to the title. */
  info?: string;
  /** Right side of the header: tabs, buttons, badges. */
  actions?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
  bodyClassName?: string;
}

export const Card = ({ title, info, actions, children, className, bodyClassName }: Props) => {
  return (
    <section className={cn("bg-card text-card-foreground min-h-0 rounded-xs border", className)}>
      <header className="flex min-h-14 items-center justify-between gap-3 px-5 pt-4 pb-2">
        <h2 className="flex items-center gap-1.5 text-[15px] font-semibold">
          {title}
          {info && (
            <span title={info} className="text-muted-foreground">
              <Info className="size-3.5" />
            </span>
          )}
        </h2>
        {actions}
      </header>
      <div className={cn("h-fit min-h-0 px-5 pb-5", bodyClassName)}>{children}</div>
    </section>
  );
};
