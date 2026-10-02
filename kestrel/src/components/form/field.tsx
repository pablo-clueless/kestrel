import type { FieldValues, Path, RefCallBack, UseFormReturn } from "react-hook-form";
import { Controller } from "react-hook-form";
import { useId } from "react";
import { cn } from "cn";

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../ui/select";
import type { FormField, Option, StandardFormField } from "./type";
import { Checkbox } from "../ui/checkbox";
import { Textarea } from "../ui/textarea";
import { Switch } from "../ui/switch";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import {
  isCustomField,
  isStandardField,
  resolveDisabled,
  resolveHidden,
  resolveRequired,
  resolveValue,
  visibleFieldError,
} from "./utils";

interface FieldProps<T extends FieldValues> {
  field: FormField<T>;
  form: UseFormReturn<T>;
  name: string;
  className?: string;
  disabled?: boolean;
}

interface ControlProps {
  config: StandardFormField;
  name: string;
  value: unknown;
  onChange: (value: unknown) => void;
  onBlur: () => void;
  inputRef: RefCallBack;
  id: string;
  describedBy?: string;
  disabled: boolean;
  invalid: boolean;
  readOnly: boolean;
  required: boolean;
  options: Option[];
  minDate?: Date;
  maxDate?: Date;
}

const INPUT_TYPES: Partial<Record<StandardFormField["type"], string>> = {
  email: "email",
  password: "password",
  search: "search",
  text: "text",
  time: "time",
};

const AUTOCOMPLETE: Partial<Record<StandardFormField["type"], string>> = {
  email: "email",
};

const toDateInput = (value: unknown): string => {
  if (!(value instanceof Date) || Number.isNaN(value.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${value.getFullYear()}-${pad(value.getMonth() + 1)}-${pad(value.getDate())}`;
};

const fromDateInput = (value: string): Date | undefined =>
  value ? new Date(`${value}T00:00:00`) : undefined;

const sameDay = (a: Date, b: Date) => toDateInput(a) === toDateInput(b);

const StandardControl = ({
  config,
  name,
  value,
  onChange,
  onBlur,
  inputRef,
  id,
  describedBy,
  disabled,
  invalid,
  readOnly,
  required,
  options,
  minDate,
  maxDate,
}: ControlProps) => {
  const a11y = {
    id,
    "aria-describedby": describedBy,
    "aria-invalid": invalid || undefined,
    "aria-required": required || undefined,
  };
  const text = {
    ...a11y,
    name: name,
    ref: inputRef,
    onBlur: onBlur,
    disabled,
    readOnly,
    placeholder: config.placeholder,
    minLength: config.minLength,
    maxLength: config.maxLength,
  };

  switch (config.type) {
    case "email":
    case "password":
    case "search":
    case "text":
    case "time":
      return (
        <Input
          {...text}
          type={INPUT_TYPES[config.type]}
          autoComplete={AUTOCOMPLETE[config.type]}
          value={(value as string | undefined) ?? ""}
          onChange={(e) => onChange(e.target.value)}
        />
      );

    case "number":
      return (
        <Input
          {...text}
          type="number"
          min={config.min}
          max={config.max}
          value={typeof value === "number" && !Number.isNaN(value) ? value : ""}
          onChange={(e) => onChange(e.target.value === "" ? undefined : e.target.valueAsNumber)}
        />
      );

    case "date":
      return (
        <Input
          {...text}
          type="date"
          min={toDateInput(minDate) || undefined}
          max={toDateInput(maxDate) || undefined}
          value={toDateInput(value)}
          onChange={(e) => {
            const date = fromDateInput(e.target.value);
            if (date && config.disabledDates?.some((d) => sameDay(d, date))) return;
            onChange(date);
          }}
        />
      );

    case "textarea":
      return (
        <Textarea
          {...text}
          value={(value as string | undefined) ?? ""}
          onChange={(e) => onChange(e.target.value)}
        />
      );

    case "file":
      return (
        <Input
          {...a11y}
          name={name}
          ref={inputRef}
          onBlur={onBlur}
          type="file"
          accept={config.accept}
          multiple={config.multiple}
          disabled={disabled || readOnly}
          onChange={(e) => {
            const files = Array.from(e.target.files ?? []);
            onChange(config.multiple ? files : files[0]);
          }}
        />
      );

    case "checkbox": {
      if (options.length === 0) {
        return (
          <Checkbox
            {...a11y}
            name={name}
            inputRef={inputRef}
            checked={!!value}
            disabled={disabled}
            readOnly={readOnly}
            onCheckedChange={(checked) => {
              onChange(checked);
              onBlur();
            }}
          />
        );
      }

      const selected = Array.isArray(value) ? (value as string[]) : [];
      return (
        <div {...a11y} role="group" className="flex flex-col gap-2">
          {options.map((option) => (
            <Label key={option.value} className="font-normal">
              <Checkbox
                checked={selected.includes(option.value)}
                disabled={disabled || option.disabled}
                readOnly={readOnly}
                onCheckedChange={(checked) => {
                  onChange(
                    checked
                      ? [...selected, option.value]
                      : selected.filter((v) => v !== option.value),
                  );
                  onBlur();
                }}
              />
              {option.label}
            </Label>
          ))}
        </div>
      );
    }

    case "toggle":
      return (
        <Switch
          {...a11y}
          name={name}
          inputRef={inputRef}
          checked={!!value}
          disabled={disabled}
          readOnly={readOnly}
          onCheckedChange={(checked) => {
            onChange(checked);
            onBlur();
          }}
        />
      );

    case "radio":
      return (
        <div {...a11y} role="radiogroup" className="flex flex-col gap-2">
          {options.map((option) => (
            <Label key={option.value} className="font-normal">
              <input
                type="radio"
                name={name}
                value={option.value}
                checked={value === option.value}
                disabled={disabled || readOnly || option.disabled}
                onChange={() => onChange(option.value)}
                onBlur={onBlur}
                className="accent-primary size-4"
              />
              {option.label}
            </Label>
          ))}
        </div>
      );

    case "select":
      return (
        <Select
          name={name}
          inputRef={inputRef}
          items={options}
          multiple={config.multiple}
          value={value ?? (config.multiple ? [] : null)}
          disabled={disabled}
          readOnly={readOnly}
          onValueChange={(value) => {
            onChange(value ?? undefined);
            onBlur();
          }}
        >
          <SelectTrigger {...a11y} className="w-full">
            <SelectValue placeholder={config.placeholder} />
          </SelectTrigger>
          <SelectContent>
            {options.map((option) => (
              <SelectItem key={option.value} value={option.value} disabled={option.disabled}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
  }
};

export const Field = <T extends FieldValues>({
  field,
  form,
  name,
  className,
  disabled: formDisabled,
}: FieldProps<T>) => {
  const id = useId();
  const values = form.watch();
  if (resolveHidden(field.hidden, values)) return null;
  if (field.showIf && !field.showIf(values)) return null;

  if (isStandardField(field)) {
    const disabled = resolveDisabled(field.disabled, formDisabled, values);
    const required = resolveRequired(field.required, values);
    const readOnly = !!resolveValue(field.readOnly, values);
    const error = visibleFieldError(form, name);
    const descriptionId = field.description ? `${id}-description` : undefined;
    const errorId = error ? `${id}-error` : undefined;
    const inline = field.type === "toggle" || (field.type === "checkbox" && !field.options);

    const label = (
      <Label htmlFor={id} className={cn(inline && "font-normal")}>
        {field.label}
        {required && <span className="text-destructive">*</span>}
      </Label>
    );

    return (
      <div className={cn("flex flex-col gap-1.5", className)}>
        {!inline && label}
        <Controller
          control={form.control}
          name={name as Path<T>}
          render={({ field: rhf }) => {
            const control = (
              <StandardControl
                config={field}
                name={rhf.name}
                value={rhf.value}
                onChange={rhf.onChange}
                onBlur={rhf.onBlur}
                inputRef={rhf.ref}
                id={id}
                describedBy={[descriptionId, errorId].filter(Boolean).join(" ") || undefined}
                disabled={disabled}
                invalid={!!error}
                readOnly={readOnly}
                required={required}
                options={resolveValue(field.options, values) ?? []}
                minDate={resolveValue(field.minDate, values)}
                maxDate={resolveValue(field.maxDate, values)}
              />
            );
            return inline ? (
              <div className="flex items-center gap-2">
                {control}
                {label}
              </div>
            ) : (
              control
            );
          }}
        />
        {field.description && (
          <p id={descriptionId} className="text-muted-foreground text-xs">
            {field.description}
          </p>
        )}
        {error && (
          <p id={errorId} className="text-destructive text-xs">
            {error}
          </p>
        )}
      </div>
    );
  }

  if (isCustomField(field)) {
    const disabled = resolveDisabled(field.disabled, formDisabled, values);
    const error = visibleFieldError(form, name);

    if (field.customMode === "uncontrolled") {
      return (
        <div className={className}>
          {typeof field.render === "function"
            ? (field.render as () => React.ReactNode)()
            : field.render}
        </div>
      );
    }

    return (
      <div className={className}>
        <Controller
          control={form.control}
          name={name as Path<T>}
          render={({ field: rhfField }) => (
            <>
              {(
                field.render as (props: {
                  field: typeof rhfField;
                  error?: string;
                  disabled?: boolean;
                  values: T;
                  form: typeof form;
                }) => React.ReactNode
              )({ field: rhfField, error, disabled, values: values as T, form })}
            </>
          )}
        />
      </div>
    );
  }

  return null;
};
