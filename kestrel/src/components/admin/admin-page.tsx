"use client";

import { useEffect } from "react";
import { toast } from "sonner";

import { errorMessage } from "@/lib/client";
import { CircleLoader } from "../shared";
import { Button } from "../ui/button";

interface Props {
  title: string;
  description: string;
  /** Right of the title: search, refresh. */
  actions?: React.ReactNode;
  children: React.ReactNode;
}

/** An admin page: title, description and actions on top, the rest scrolling under them. */
export const AdminPage = ({ title, description, actions, children }: Props) => (
  <div className="h-full overflow-y-auto">
    <div className="flex w-full flex-col gap-5 p-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold">{title}</h1>
          <p className="text-muted-foreground text-sm">{description}</p>
        </div>
        {actions}
      </div>
      {children}
    </div>
  </div>
);

/** What a page shows while its query loads or after it failed (with a retry), else `children`. */
export const QueryState = ({
  query,
  children,
}: {
  query: { isPending: boolean; isError: boolean; error: unknown; refetch: () => unknown };
  children: () => React.ReactNode;
}) => {
  useEffect(() => {
    if (query.isError) toast.error(errorMessage(query.error));
  }, [query.isError, query.error]);

  if (query.isPending) {
    return (
      <div className="grid place-items-center py-16" aria-busy="true" aria-label="Loading">
        <CircleLoader />
      </div>
    );
  }
  if (query.isError) {
    return (
      <div className="flex flex-col items-center gap-3 py-16 text-center text-sm">
        <p className="text-destructive">{errorMessage(query.error)}</p>
        <Button variant="outline" onClick={() => void query.refetch()}>
          Try again
        </Button>
      </div>
    );
  }
  return children();
};

/** A small rounded label next to a name. */
export const Badge = ({
  children,
  tone = "muted",
}: {
  children: React.ReactNode;
  tone?: "muted" | "primary" | "warning" | "destructive";
}) => (
  <span
    className={
      tone === "primary"
        ? "bg-primary/15 text-primary rounded-full px-2 py-0.5 text-[11px] font-medium"
        : tone === "warning"
          ? "bg-warning/15 text-warning rounded-full px-2 py-0.5 text-[11px] font-medium"
          : tone === "destructive"
            ? "bg-destructive/15 text-destructive rounded-full px-2 py-0.5 text-[11px] font-medium"
            : "bg-muted text-muted-foreground rounded-full px-2 py-0.5 text-[11px] font-medium"
    }
  >
    {children}
  </span>
);
