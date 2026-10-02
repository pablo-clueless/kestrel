import type { FieldValues, UseFormReturn } from "react-hook-form";
import type {
  ArrayFormField,
  CustomFormField,
  FormField,
  FormSection,
  ObjectArrayFormField,
  ObjectFormField,
  PrimitiveArrayFormField,
  StandardFormField,
} from "./type";

export const isCustomField = <T extends FieldValues>(
  field: FormField<T>,
): field is CustomFormField<T> => "custom" in field && field.custom === true;

export const isStandardField = <T extends FieldValues>(
  field: FormField<T>,
): field is StandardFormField =>
  !("custom" in field) && !("kind" in field && field.kind !== "standard") && "type" in field;

export const isObjectField = <T extends FieldValues>(
  field: FormField<T>,
): field is ObjectFormField => "kind" in field && field.kind === "object";

export const isArrayField = <T extends FieldValues>(field: FormField<T>): field is ArrayFormField =>
  "kind" in field && field.kind === "array";

export const isPrimitiveArrayField = <T extends FieldValues>(
  field: FormField<T>,
): field is PrimitiveArrayFormField =>
  isArrayField(field) && (field as ArrayFormField).itemKind === "primitive";

export const isObjectArrayField = <T extends FieldValues>(
  field: FormField<T>,
): field is ObjectArrayFormField =>
  isArrayField(field) && (field as ArrayFormField).itemKind === "object";

export const normaliseSections = <T extends FieldValues>(
  fields: Record<string, FormField<T>>,
  sections?: FormSection[],
): FormSection[] => {
  if (sections && sections.length > 0) return sections;
  return [{ fields: Object.keys(fields) }];
};

const readPath = (source: unknown, path: string): unknown =>
  path
    .split(".")
    .reduce<unknown>((value, key) => (value as Record<string, unknown> | undefined)?.[key], source);

export const visibleFieldError = <T extends FieldValues>(
  form: UseFormReturn<T>,
  name: string,
): string | undefined => {
  const isTouched = Boolean(readPath(form.formState.touchedFields, name));
  const wasSubmitted = form.formState.isSubmitted;
  const message = (readPath(form.formState.errors, name) as { message?: unknown } | undefined)
    ?.message;

  if (typeof message !== "string") return undefined;
  return isTouched || wasSubmitted ? message : undefined;
};

export const resolveRequired = (
  fieldRequired?: boolean | ((values: FieldValues) => boolean),
  values?: FieldValues,
): boolean => (typeof fieldRequired === "function" ? fieldRequired(values ?? {}) : !!fieldRequired);

export const resolveDisabled = (
  fieldDisabled?: boolean | ((values: FieldValues) => boolean),
  formDisabled?: boolean,
  values?: FieldValues,
): boolean => {
  const field = typeof fieldDisabled === "function" ? fieldDisabled(values ?? {}) : fieldDisabled;
  return !!(field || formDisabled);
};

export const resolveHidden = (
  fieldHidden?: boolean | ((values: FieldValues) => boolean),
  values?: FieldValues,
): boolean => (typeof fieldHidden === "function" ? fieldHidden(values ?? {}) : !!fieldHidden);

export const resolveValue = <V>(
  value: V | ((values: FieldValues) => V) | undefined,
  values?: FieldValues,
): V | undefined =>
  typeof value === "function" ? (value as (values: FieldValues) => V)(values ?? {}) : value;

export const firstFieldErrorMessage =(errors: unknown): string | undefined => {
  if (!errors || typeof errors !== "object") return undefined;
  const node = errors as { message?: unknown; root?: unknown } & Record<string, unknown>;
  if (typeof node.message === "string" && node.message.trim()) return node.message;
  if (node.root) {
    const fromRoot = firstFieldErrorMessage(node.root);
    if (fromRoot) return fromRoot;
  }
  for (const [key, value] of Object.entries(node)) {
    if (key === "ref" || key === "type") continue;
    const nested = firstFieldErrorMessage(value);
    if (nested) return nested;
  }
  return undefined;
};

export function trimDeep<T>(value: T, skipKeys?: Set<string>): T {
  if (typeof value === "string") return value.trim() as unknown as T;
  if (Array.isArray(value)) return value.map((v) => trimDeep(v)) as unknown as T;
  if (value && typeof value === "object") {
    if (value instanceof Date) return value;
    if (typeof File !== "undefined" && value instanceof File) return value;
    if (typeof Blob !== "undefined" && value instanceof Blob) return value;
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) {
      out[k] = skipKeys?.has(k) ? v : trimDeep(v);
    }
    return out as T;
  }
  return value;
}
