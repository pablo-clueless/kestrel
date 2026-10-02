import { Input as InputPrimitive } from "@base-ui/react/input";
import { Eye, EyeOff, Search, X } from "lucide-react";
import * as React from "react";
import { cn } from "cn";

import { getPasswordStrength, type PasswordStrength } from "@/lib/security";

const config: Record<PasswordStrength, { label: string; className: string }> = {
  weak: { label: "Weak", className: "bg-red-100 text-red-600" },
  fair: { label: "Fair", className: "bg-yellow-100 text-yellow-600" },
  strong: { label: "Strong", className: "bg-green-100 text-green-600" },
};

function Input({
  className,
  containerClassName,
  ref,
  showStrength = false,
  type,
  ...props
}: React.ComponentProps<"input"> & {
  containerClassName?: string;
  showStrength?: boolean;
}) {
  const [showPassword, setShowPassword] = React.useState(false);
  const [passwordValue, setPasswordValue] = React.useState("");
  const [uncontrolledSearchValue, setUncontrolledSearchValue] = React.useState(() =>
    String(props.defaultValue ?? ""),
  );

  const input = React.useRef<HTMLInputElement>(null);

  const isControlled = props.value !== undefined;
  const isPassword = type === "password";
  const isSearch = type === "search";

  const password = isControlled ? String(props.value ?? "") : passwordValue;
  const strength = isPassword && showStrength ? getPasswordStrength(password) : null;
  const searchValue = isControlled ? String(props.value ?? "") : uncontrolledSearchValue;
  const showClear = isSearch && searchValue.length > 0 && !props.disabled && !props.readOnly;

  const setInputRef = React.useCallback(
    (node: HTMLInputElement | null) => {
      input.current = node;
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
    },
    [ref],
  );

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (isPassword) setPasswordValue(e.target.value);
    if (isSearch && !isControlled) setUncontrolledSearchValue(e.target.value);
    props.onChange?.(e);
  };

  const clearSearch = () => {
    if (!isControlled && input.current) {
      input.current.value = "";
      setUncontrolledSearchValue("");
    }
    props.onChange?.({
      target: { value: "" },
      currentTarget: { value: "" },
    } as React.ChangeEvent<HTMLInputElement>);
    input.current?.focus();
  };

  return (
    <div className={cn("relative w-full", containerClassName)}>
      {isSearch && (
        <Search className="text-muted-foreground absolute top-1/2 left-3 size-4 -translate-y-1/2" />
      )}
      <InputPrimitive
        className={cn(
          "border-primary file:text-foreground placeholder:text-muted-foreground focus-visible:border-primary disabled:bg-primary/10 aria-invalid:border-destructive dark:bg-primary/10 dark:disabled:bg-primary/20 dark:aria-invalid:border-destructive/50 h-8 w-full min-w-0 rounded-xs border bg-transparent px-2.5 py-1 text-base transition-colors outline-none file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
          className,
        )}
        data-slot="input"
        onChange={handleChange}
        ref={setInputRef}
        type={showPassword ? "text" : type}
        {...props}
      />
      {showClear && (
        <button
          type="button"
          onClick={clearSearch}
          className="text-muted-foreground hover:text-foreground absolute top-1/2 right-3 -translate-y-1/2 cursor-pointer"
          tabIndex={-1}
          aria-label={"Clear search"}
        >
          <X className="size-4" />
        </button>
      )}
      {isPassword && (
        <div className="absolute top-1/2 right-3 flex -translate-y-1/2 items-center gap-1.5">
          {strength && (
            <span
              className={cn(
                "rounded-full px-2 py-0.5 text-[10px] font-semibold",
                config[strength].className,
              )}
            >
              {config[strength].label}
            </span>
          )}
          <button
            type="button"
            onClick={() => setShowPassword((prev) => !prev)}
            className="text-muted-foreground cursor-pointer"
            tabIndex={-1}
            aria-label={showPassword ? "Hide password" : "Show password"}
          >
            {showPassword ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
          </button>
        </div>
      )}
    </div>
  );
}

export { Input };
