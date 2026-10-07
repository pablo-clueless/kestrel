"use client";

import { Check, Copy, Download, ShieldCheck } from "lucide-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import QRCode from "qrcode";
import { toast } from "sonner";
import { z } from "zod";

import {
  disableTwoFactor,
  enableTwoFactor,
  errorMessage,
  regenerateRecoveryCodes,
  setupTwoFactor,
  twoFactorStatus,
} from "@/lib/client";
import type { TwoFactorSetup } from "@/types/engine/TwoFactorSetup";
import { Button } from "../ui/button";
import { Form } from "../form";

const STATUS_KEY = ["two-factor"] as const;

type Step =
  | { kind: "idle" }
  | { kind: "password" }
  | { kind: "scan"; setup: TwoFactorSetup }
  | { kind: "codes"; codes: string[] }
  | { kind: "confirm"; action: "disable" | "regenerate" };

/** Profile → Security: two-factor sign-in with an authenticator app, and its recovery codes. */
export const TwoFactor = () => {
  const queryClient = useQueryClient();
  const status = useQuery({ queryKey: STATUS_KEY, queryFn: twoFactorStatus });
  const [step, setStep] = useState<Step>({ kind: "idle" });
  const refresh = () => void queryClient.invalidateQueries({ queryKey: STATUS_KEY });

  return (
    <section className="flex flex-col gap-4">
      <div>
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          Two-factor sign-in
          {status.data?.enabled && (
            <span className="bg-success/15 text-success rounded-full px-2 py-0.5 text-[11px] font-medium">
              On
            </span>
          )}
        </h2>
        <p className="text-muted-foreground text-sm">
          After your password, signing in asks for a code from an authenticator app (1Password,
          Google Authenticator, Authy…), so a stolen password alone isn&apos;t enough.
        </p>
      </div>

      {step.kind === "codes" ? (
        <RecoveryCodesView codes={step.codes} onDone={() => setStep({ kind: "idle" })} />
      ) : step.kind === "scan" ? (
        <Scan
          setup={step.setup}
          onCancel={() => setStep({ kind: "idle" })}
          onEnabled={(codes) => {
            refresh();
            setStep({ kind: "codes", codes });
          }}
        />
      ) : step.kind === "password" ? (
        <PasswordForm
          action="Continue"
          onCancel={() => setStep({ kind: "idle" })}
          onSubmit={async (password) =>
            setStep({ kind: "scan", setup: await setupTwoFactor(password) })
          }
        />
      ) : step.kind === "confirm" ? (
        <ConfirmForm
          action={step.action === "disable" ? "Turn off" : "Make new codes"}
          onCancel={() => setStep({ kind: "idle" })}
          onSubmit={async (req) => {
            if (step.action === "disable") {
              await disableTwoFactor(req);
              toast.success("Two-factor sign-in is off.");
              setStep({ kind: "idle" });
            } else {
              setStep({ kind: "codes", codes: await regenerateRecoveryCodes(req) });
            }
            refresh();
          }}
        />
      ) : status.isPending ? (
        <p className="text-muted-foreground text-sm">Loading…</p>
      ) : status.data?.enabled ? (
        <div className="flex flex-col gap-3">
          <p className="text-sm">
            {status.data.recoveryCodesLeft === 0
              ? "You have no recovery codes left. Make new ones in case you lose your phone."
              : `${status.data.recoveryCodesLeft} recovery code${status.data.recoveryCodesLeft === 1 ? "" : "s"} left.`}
          </p>
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => setStep({ kind: "confirm", action: "regenerate" })}
            >
              New recovery codes
            </Button>
            <Button
              variant="outline"
              onClick={() => setStep({ kind: "confirm", action: "disable" })}
            >
              Turn off
            </Button>
          </div>
        </div>
      ) : (
        <Button className="self-start" onClick={() => setStep({ kind: "password" })}>
          <ShieldCheck /> Turn on
        </Button>
      )}
    </section>
  );
};

const passwordSchema = z.object({ password: z.string().min(1, "Enter your password") });

const PasswordForm = ({
  action,
  onSubmit,
  onCancel,
}: {
  action: string;
  onSubmit: (password: string) => Promise<void>;
  onCancel: () => void;
}) => (
  <Form
    className="max-w-sm"
    schema={passwordSchema}
    defaultValues={{ password: "" }}
    toastOnInvalid={false}
    fields={{
      password: {
        type: "password",
        label: "Your password",
        autoComplete: "current-password",
        autoFocus: true,
      },
    }}
    onSubmit={async ({ password }, form) => {
      try {
        await onSubmit(password);
      } catch (err) {
        form.setError("password", { message: errorMessage(err) });
      }
    }}
  >
    {({ field, isSubmitting }) => (
      <div className="flex flex-col gap-3">
        {field("password")}
        <Actions action={action} busy={isSubmitting} onCancel={onCancel} />
      </div>
    )}
  </Form>
);

const confirmSchema = z.object({
  password: z.string().min(1, "Enter your password"),
  code: z.string().trim().min(6, "Enter a code from your app, or a recovery code"),
});

/** Password plus a current code: what turning it off or replacing the recovery codes needs. */
const ConfirmForm = ({
  action,
  onSubmit,
  onCancel,
}: {
  action: string;
  onSubmit: (req: { password: string; code: string }) => Promise<void>;
  onCancel: () => void;
}) => (
  <Form
    className="max-w-sm"
    schema={confirmSchema}
    defaultValues={{ password: "", code: "" }}
    toastOnInvalid={false}
    fields={{
      password: {
        type: "password",
        label: "Your password",
        autoComplete: "current-password",
        autoFocus: true,
      },
      code: {
        type: "text",
        label: "Code from your app, or a recovery code",
        autoComplete: "one-time-code",
      },
    }}
    onSubmit={async (req, form) => {
      try {
        await onSubmit(req);
      } catch (err) {
        const message = errorMessage(err);
        form.setError(/password/i.test(message) ? "password" : "code", { message });
      }
    }}
  >
    {({ field, isSubmitting }) => (
      <div className="flex flex-col gap-3">
        {field("password")}
        {field("code")}
        <Actions action={action} busy={isSubmitting} onCancel={onCancel} />
      </div>
    )}
  </Form>
);

const codeSchema = z.object({
  code: z
    .string()
    .trim()
    .regex(/^\d{6}$/, "Enter the 6-digit code the app shows"),
});

/** Add the secret to the app (QR code or typed), then prove it with a code. */
const Scan = ({
  setup,
  onEnabled,
  onCancel,
}: {
  setup: TwoFactorSetup;
  onEnabled: (codes: string[]) => void;
  onCancel: () => void;
}) => {
  const [qr, setQr] = useState<string | null>(null);
  useEffect(() => {
    QRCode.toDataURL(setup.otpauthUrl, { margin: 1, width: 180 })
      .then(setQr)
      .catch(() => setQr(null));
  }, [setup.otpauthUrl]);

  return (
    <div className="flex max-w-md flex-col gap-4">
      <ol className="text-muted-foreground list-decimal space-y-1 pl-5 text-sm">
        <li>Scan this with your authenticator app, or type the key into it.</li>
        <li>Enter the 6-digit code it shows.</li>
      </ol>
      <div className="flex items-center gap-4">
        {qr ? (
          // A data URL made here: nothing to optimise, and the secret never leaves the page.
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={qr}
            alt="QR code for your authenticator app"
            className="size-44 rounded-xs border bg-white"
          />
        ) : (
          <div className="bg-muted size-44 rounded-xs" aria-hidden />
        )}
        <div className="flex min-w-0 flex-col gap-1">
          <span className="text-muted-foreground text-xs">Key</span>
          <code className="font-mono text-xs break-all">
            {setup.secret.match(/.{1,4}/g)?.join(" ")}
          </code>
        </div>
      </div>
      <Form
        schema={codeSchema}
        defaultValues={{ code: "" }}
        toastOnInvalid={false}
        fields={{
          code: {
            type: "text",
            label: "Code",
            autoComplete: "one-time-code",
            placeholder: "123456",
          },
        }}
        onSubmit={async ({ code }, form) => {
          try {
            onEnabled(await enableTwoFactor(code));
          } catch (err) {
            form.setError("code", { message: errorMessage(err) });
          }
        }}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("code")}
            <Actions action="Turn on" busy={isSubmitting} onCancel={onCancel} />
          </div>
        )}
      </Form>
    </div>
  );
};

/** Recovery codes, shown once: copy or download them before leaving. */
const RecoveryCodesView = ({ codes, onDone }: { codes: string[]; onDone: () => void }) => {
  const [copied, setCopied] = useState(false);
  const text = codes.join("\n");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      toast.error("Couldn't copy. Select the codes and copy them yourself.");
    }
  };
  const download = () => {
    const url = URL.createObjectURL(
      new Blob([`Kestrel recovery codes\n\n${text}\n`], { type: "text/plain" }),
    );
    const a = Object.assign(document.createElement("a"), {
      href: url,
      download: "kestrel-recovery-codes.txt",
    });
    a.click();
    URL.revokeObjectURL(url);
  };
  return (
    <div className="flex max-w-md flex-col gap-3">
      <p className="text-sm">
        Save these recovery codes somewhere safe. Each one signs you in once if you lose your phone.
        <strong> This is the only time they&apos;re shown.</strong>
      </p>
      <ul className="bg-muted/40 grid grid-cols-2 gap-x-6 gap-y-1 rounded-xs border p-3 font-mono text-sm">
        {codes.map((c) => (
          <li key={c}>{c}</li>
        ))}
      </ul>
      <div className="flex gap-2">
        <Button variant="outline" onClick={copy}>
          {copied ? <Check /> : <Copy />} {copied ? "Copied" : "Copy"}
        </Button>
        <Button variant="outline" onClick={download}>
          <Download /> Download
        </Button>
        <Button onClick={onDone}>I&apos;ve saved them</Button>
      </div>
    </div>
  );
};

const Actions = ({
  action,
  busy,
  onCancel,
}: {
  action: string;
  busy: boolean;
  onCancel: () => void;
}) => (
  <div className="flex gap-2">
    <Button type="submit" disabled={busy}>
      {busy ? "Working…" : action}
    </Button>
    <Button type="button" variant="outline" onClick={onCancel}>
      Cancel
    </Button>
  </div>
);
