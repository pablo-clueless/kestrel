import { Eye, EyeOff, type LucideIcon } from "lucide-react";

import { cn } from "cn";

interface Props extends React.ComponentProps<"input"> {
  id: string;
  label: string;
  icon: LucideIcon;
  invalid?: boolean;
  trailing?: React.ReactNode;
}

/** The auth pages' input: a leading icon and a small label above the value, in one rounded box. */
export const AuthField = ({
  id,
  label,
  icon: Icon,
  invalid,
  trailing,
  className,
  ...props
}: Props) => (
  <div
    className={cn(
      "bg-card focus-within:border-primary focus-within:ring-primary/15 flex h-14 items-center border transition-[border-color,box-shadow] focus-within:ring-4",
      invalid &&
        "border-destructive focus-within:border-destructive focus-within:ring-destructive/15",
      className,
    )}
  >
    <span className="text-muted-foreground grid h-full w-12 shrink-0 place-items-center">
      <Icon className="size-4.5" />
    </span>
    <span className="bg-border h-7 w-px shrink-0" />
    <div className="flex min-w-0 flex-1 flex-col justify-center px-3">
      <label htmlFor={id} className="text-muted-foreground text-[11px] leading-none">
        {label}
      </label>
      <input
        id={id}
        aria-invalid={invalid}
        className="placeholder:text-muted-foreground/60 mt-1 w-full bg-transparent text-sm font-medium outline-none"
        {...props}
      />
    </div>
    {trailing && <span className="grid shrink-0 place-items-center pr-3">{trailing}</span>}
  </div>
);

/** The show/hide button for an `AuthField` holding a password. */
export const PasswordToggle = ({ shown, onToggle }: { shown: boolean; onToggle: () => void }) => (
  <button
    type="button"
    onClick={onToggle}
    aria-label={shown ? "Hide password" : "Show password"}
    className="text-muted-foreground hover:text-foreground"
  >
    {shown ? <EyeOff className="size-4.5" /> : <Eye className="size-4.5" />}
  </button>
);
