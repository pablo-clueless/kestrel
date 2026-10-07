"use client";

import { useCanEdit } from "@/hooks/use-me";
import { cn } from "cn";

/**
 * Wraps controls that change the workspace or send requests. For a member with read access every
 * input, select and button inside is disabled (a disabled `<fieldset>` does that natively), so they
 * can look but not make edits that would go nowhere. The engine refuses those anyway; this is what
 * makes it visible.
 */
export const Editable = ({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) => {
  const canEdit = useCanEdit();
  return (
    // A fieldset's default `min-width: min-content` would stop flex children from shrinking.
    <fieldset disabled={!canEdit} className={cn("m-0 min-w-0 border-0 p-0", className)}>
      {children}
    </fieldset>
  );
};
