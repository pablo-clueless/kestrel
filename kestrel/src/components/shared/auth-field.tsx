import type { ControllerRenderProps, FieldValues, Path } from "react-hook-form";
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
      "bg-card focus-within:border-primary flex h-14 items-center border transition-[border-color,box-shadow]",
      invalid && "border-destructive focus-within:border-destructive",
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

type ControlProps<T extends FieldValues> = Omit<
  Props,
  "name" | "value" | "onChange" | "onBlur" | "invalid" | "ref"
> & {
  /** From a `Form` custom field's render props. */
  field: ControllerRenderProps<T, Path<T>>;
  error?: string;
  /** Shown under the field while there's no error, e.g. the password rule. */
  hint?: React.ReactNode;
};

/** An `AuthField` wired to a `Form` field, with its error (or hint) underneath. */
export const AuthControl = <T extends FieldValues>({
  // Destructured: reading `field.ref` and then `field.value` makes the React Compiler treat the
  // whole object as a ref.
  field: { ref, name, value, onChange, onBlur },
  error,
  hint,
  id,
  ...props
}: ControlProps<T>) => (
  <div className="flex flex-col gap-1.5">
    <AuthField
      {...props}
      id={id}
      ref={ref}
      name={name}
      value={(value as string | undefined) ?? ""}
      onChange={(e) => onChange(e.target.value)}
      onBlur={onBlur}
      invalid={!!error}
      aria-describedby={error || hint ? `${id}-note` : undefined}
    />
    {error ? (
      <p id={`${id}-note`} className="text-destructive text-xs">
        {error}
      </p>
    ) : (
      hint && (
        <p id={`${id}-note`} className="text-muted-foreground text-xs">
          {hint}
        </p>
      )
    )}
  </div>
);
