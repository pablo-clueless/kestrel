import * as React from "react";
import { cn } from "cn";

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "border-primary placeholder:text-muted-foreground focus-visible:border-primary disabled:bg-primary/10 aria-invalid:border-destructive dark:bg-primary/10 dark:disabled:bg-primary/20 dark:aria-invalid:border-destructive/50 flex field-sizing-content min-h-16 w-full rounded-xs border bg-transparent px-2.5 py-2 text-base transition-colors outline-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
