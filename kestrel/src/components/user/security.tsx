"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Laptop, Smartphone } from "lucide-react";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { changePassword, errorMessage, listSessions, revokeSession } from "@/lib/client";
import type { SessionInfo } from "@/types/engine/SessionInfo";
import { useMe } from "@/hooks/use-me";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { TabPanel } from "../shared";

interface Props {
  selected: string;
}

const SESSIONS_KEY = ["sessions"] as const;

export const Security = ({ selected }: Props) => {
  const me = useMe();
  const accountsOn = me.data?.auth === "on";

  return (
    <TabPanel selected={selected} value="security">
      <div className="bg-background flex flex-col gap-8 p-5">
        {accountsOn ? (
          <>
            <ChangePassword />
            <Sessions />
          </>
        ) : (
          <p className="text-muted-foreground text-sm">
            Accounts are off on this engine, so there&apos;s no password or sign-in to manage.
          </p>
        )}
      </div>
    </TabPanel>
  );
};

const passwordSchema = z
  .object({
    currentPassword: z.string().min(1, "Enter your current password"),
    newPassword: z.string().min(PASSWORD_MIN, PASSWORD_MESSAGE).max(PASSWORD_MAX, PASSWORD_MESSAGE),
    confirm: z.string(),
  })
  .refine((v) => v.newPassword === v.confirm, {
    path: ["confirm"],
    message: "Passwords don't match",
  })
  .refine((v) => v.newPassword !== v.currentPassword, {
    path: ["newPassword"],
    message: "Choose a password you aren't using now",
  });

type PasswordValues = z.infer<typeof passwordSchema>;

const ChangePassword = () => {
  const queryClient = useQueryClient();
  const {
    register,
    handleSubmit,
    reset,
    setError,
    formState: { errors, isSubmitting },
  } = useForm<PasswordValues>({
    defaultValues: { currentPassword: "", newPassword: "", confirm: "" },
    resolver: zodResolver(passwordSchema),
  });

  const onSubmit = async ({ currentPassword, newPassword }: PasswordValues) => {
    try {
      await changePassword({ currentPassword, newPassword });
      reset();
      toast.success("Password changed. Your other devices have been signed out.");
      void queryClient.invalidateQueries({ queryKey: SESSIONS_KEY });
    } catch (err) {
      const message = errorMessage(err);
      if (message === "current password is incorrect") {
        setError("currentPassword", { message: "That's not your current password" });
      } else {
        setError("root", { message });
      }
    }
  };

  const field = (
    name: keyof PasswordValues,
    label: string,
    autoComplete: string,
    hint?: string,
  ) => (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={name}>{label}</Label>
      <Input
        id={name}
        type="password"
        autoComplete={autoComplete}
        aria-invalid={!!errors[name]}
        {...register(name)}
      />
      {errors[name] ? (
        <p className="text-destructive text-xs">{errors[name].message}</p>
      ) : (
        hint && <p className="text-muted-foreground text-xs">{hint}</p>
      )}
    </div>
  );

  return (
    <section className="flex flex-col gap-4">
      <div>
        <h2 className="text-sm font-semibold">Change password</h2>
        <p className="text-muted-foreground text-sm">
          Every other device signed in to this account is signed out.
        </p>
      </div>
      <form className="flex max-w-sm flex-col gap-4" onSubmit={handleSubmit(onSubmit)} noValidate>
        {field("currentPassword", "Current password", "current-password")}
        {field("newPassword", "New password", "new-password", PASSWORD_MESSAGE)}
        {field("confirm", "Confirm new password", "new-password")}
        {errors.root && (
          <p role="alert" className="text-destructive text-sm">
            {errors.root.message}
          </p>
        )}
        <Button type="submit" className="self-start" disabled={isSubmitting}>
          {isSubmitting ? "Changing…" : "Change password"}
        </Button>
      </form>
    </section>
  );
};

const Sessions = () => {
  const queryClient = useQueryClient();
  const sessions = useQuery({ queryKey: SESSIONS_KEY, queryFn: listSessions });
  const revoke = useMutation({
    mutationFn: revokeSession,
    onSuccess: () => {
      toast.success("Signed out that device.");
      void queryClient.invalidateQueries({ queryKey: SESSIONS_KEY });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });

  return (
    <section className="flex flex-col gap-4">
      <div>
        <h2 className="text-sm font-semibold">Where you&apos;re signed in</h2>
        <p className="text-muted-foreground text-sm">
          Sign out anything you don&apos;t recognise, then change your password.
        </p>
      </div>
      {sessions.isPending ? (
        <p className="text-muted-foreground text-sm">Loading…</p>
      ) : sessions.isError ? (
        <p className="text-destructive text-sm">{errorMessage(sessions.error)}</p>
      ) : (
        <ul className="divide-y rounded-xs border">
          {sessions.data.map((s) => (
            <SessionRow
              key={s.id}
              session={s}
              revoking={revoke.isPending && revoke.variables === s.id}
              onRevoke={() => revoke.mutate(s.id)}
            />
          ))}
        </ul>
      )}
    </section>
  );
};

const SessionRow = ({
  session,
  revoking,
  onRevoke,
}: {
  session: SessionInfo;
  revoking: boolean;
  onRevoke: () => void;
}) => {
  const { label, mobile } = describeDevice(session.userAgent);
  const Icon = mobile ? Smartphone : Laptop;
  return (
    <li className="flex items-center gap-3 px-3 py-2.5 text-sm">
      <Icon className="text-muted-foreground size-4 shrink-0" />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex items-center gap-2 font-medium">
          {label}
          {session.current && (
            <span className="bg-primary/15 text-primary rounded-full px-2 py-0.5 text-[11px] font-medium">
              This device
            </span>
          )}
        </span>
        <span className="text-muted-foreground truncate text-xs">
          {[
            session.ip,
            `active ${relative(session.lastSeenAtMs)}`,
            `signed in ${relative(session.createdAtMs)}`,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </div>
      {!session.current && (
        <Button variant="outline" size="sm" onClick={onRevoke} disabled={revoking}>
          {revoking ? "Signing out…" : "Sign out"}
        </Button>
      )}
    </li>
  );
};

/** "Chrome on Windows", from a user agent. Rough on purpose: it's only to recognise your devices. */
const describeDevice = (ua: string | null) => {
  if (!ua) return { label: "Unknown device", mobile: false };
  const browser =
    [
      [/Edg\//, "Edge"],
      [/OPR\/|Opera/, "Opera"],
      [/Firefox\//, "Firefox"],
      [/Chrome\//, "Chrome"],
      [/Safari\//, "Safari"],
      [/curl\//, "curl"],
    ].find(([re]) => (re as RegExp).test(ua))?.[1] ?? "Browser";
  const os =
    [
      [/iPhone|iPad/, "iOS"],
      [/Android/, "Android"],
      [/Windows/, "Windows"],
      [/Mac OS X|Macintosh/, "macOS"],
      [/Linux/, "Linux"],
    ].find(([re]) => (re as RegExp).test(ua))?.[1] ?? null;
  return {
    label: os ? `${browser} on ${os}` : String(browser),
    mobile: /Mobi|iPhone|Android/.test(ua),
  };
};

const relativeFormat = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** "5 minutes ago", "yesterday". */
const relative = (ms: number) => {
  const seconds = Math.round((ms - Date.now()) / 1000);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return relativeFormat.format(Math.round(seconds / size), unit);
  }
  return "just now";
};
