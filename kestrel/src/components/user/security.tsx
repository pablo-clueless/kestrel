"use client";

import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Laptop, Smartphone } from "lucide-react";
import { toast } from "sonner";
import { z } from "zod";

import {
  changePassword,
  errorMessage,
  listSessions,
  revokeSession,
  signOutOtherSessions,
} from "@/lib/client";
import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import type { SessionInfo } from "@/types/engine/SessionInfo";
import { useMe } from "@/hooks/use-me";
import { relative } from "@/lib/format";
import { Form } from "../form";
import { Button } from "../ui/button";
import { ConfirmButton, TabPanel } from "../shared";
import { TwoFactor } from "./two-factor";

interface Props {
  selected: string;
}

const SESSIONS_KEY = ["sessions"] as const;

export const Security = ({ selected }: Props) => {
  const me = useMe();
  const accountsOn = me.data?.auth === "on";

  return (
    <TabPanel selected={selected} value="security">
      <div className="bg-background flex h-[calc(100%-36px)] flex-col gap-4 overflow-y-auto p-5">
        {accountsOn ? (
          <>
            <ChangePassword />
            <TwoFactor />
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

const ChangePassword = () => {
  const queryClient = useQueryClient();

  return (
    <section className="flex flex-col gap-4">
      <div>
        <h2 className="text-sm font-semibold">Change password</h2>
        <p className="text-muted-foreground text-sm">
          Every other device signed in to this account is signed out.
        </p>
      </div>
      <Form
        className="max-w-sm"
        schema={passwordSchema}
        defaultValues={{ currentPassword: "", newPassword: "", confirm: "" }}
        toastOnInvalid={false}
        fields={{
          currentPassword: {
            type: "password",
            label: "Current password",
            autoComplete: "current-password",
          },
          newPassword: {
            type: "password",
            label: "New password",
            autoComplete: "new-password",
            description: PASSWORD_MESSAGE,
          },
          confirm: {
            type: "password",
            label: "Confirm new password",
            autoComplete: "new-password",
          },
        }}
        onSubmit={async ({ currentPassword, newPassword }, form) => {
          try {
            await changePassword({ currentPassword, newPassword });
            form.reset();
            toast.success("Password changed. Your other devices have been signed out.");
            void queryClient.invalidateQueries({ queryKey: SESSIONS_KEY });
          } catch (err) {
            const message = errorMessage(err);
            if (message.toLowerCase() === "current password is incorrect") {
              form.setError("currentPassword", { message: "That's not your current password" });
            } else {
              toast.error(message);
            }
          }
        }}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-4">
            {field("currentPassword")}
            {field("newPassword")}
            {field("confirm")}
            <Button type="submit" className="self-start" disabled={isSubmitting}>
              {isSubmitting ? "Changing…" : "Change password"}
            </Button>
          </div>
        )}
      </Form>
    </section>
  );
};

const Sessions = () => {
  const queryClient = useQueryClient();
  const sessions = useQuery({ queryKey: SESSIONS_KEY, queryFn: listSessions });
  useEffect(() => {
    if (sessions.isError) toast.error(errorMessage(sessions.error));
  }, [sessions.isError, sessions.error]);
  const revoke = useMutation({
    mutationFn: revokeSession,
    onSuccess: () => {
      toast.success("Signed out that device.");
      void queryClient.invalidateQueries({ queryKey: SESSIONS_KEY });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  const revokeOthers = useMutation({
    mutationFn: signOutOtherSessions,
    onSuccess: (ended) => {
      toast.success(`Signed out ${ended} other ${ended === 1 ? "device" : "devices"}.`);
      void queryClient.invalidateQueries({ queryKey: SESSIONS_KEY });
    },
    onError: (err) => toast.error(errorMessage(err)),
  });
  const others = sessions.data?.filter((s) => !s.current).length ?? 0;

  return (
    <section className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-semibold">Where you&apos;re signed in</h2>
          <p className="text-muted-foreground text-sm">
            Sign out anything you don&apos;t recognise, then change your password.
          </p>
        </div>
        {others > 0 && (
          <ConfirmButton
            variant="outline"
            size="sm"
            className="shrink-0"
            pending={revokeOthers.isPending}
            pendingLabel="Signing out…"
            confirmLabel={`Sign out ${others} ${others === 1 ? "device" : "devices"}?`}
            onConfirm={() => revokeOthers.mutate()}
          >
            Sign out everywhere else
          </ConfirmButton>
        )}
      </div>
      {sessions.isPending ? (
        <p className="text-muted-foreground text-sm">Loading…</p>
      ) : sessions.isError ? (
        <p className="text-muted-foreground text-sm">Couldn&apos;t load your devices.</p>
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
