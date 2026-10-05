"use client";

import { CircleCheck, LockKeyhole, TriangleAlert } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { Suspense, useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { toast } from "sonner";
import Link from "next/link";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { AuthControl, PasswordToggle } from "@/components/shared/auth-field";
import { Form, type FormField } from "@/components/form";
import { errorMessage, resetPassword } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { ME_KEY } from "@/hooks/use-me";

const schema = z.object({
  password: z.string().min(PASSWORD_MIN, PASSWORD_MESSAGE).max(PASSWORD_MAX, PASSWORD_MESSAGE),
});

type FormValues = z.infer<typeof schema>;

const Outcome = ({
  ok,
  title,
  children,
}: {
  ok: boolean;
  title: string;
  children: React.ReactNode;
}) => (
  <div className="flex flex-col items-center gap-6 text-center">
    <span
      className={
        ok
          ? "bg-success/10 text-success grid size-14 place-items-center rounded-2xl"
          : "bg-destructive/10 text-destructive grid size-14 place-items-center rounded-2xl"
      }
    >
      {ok ? <CircleCheck className="size-7" /> : <TriangleAlert className="size-7" />}
    </span>
    <div className="flex flex-col gap-1.5">
      <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
      <p className="text-muted-foreground text-sm">{children}</p>
    </div>
  </div>
);

/** Chooses a new password with the token from the emailed link. */
const ResetForm = () => {
  const queryClient = useQueryClient();
  const params = useSearchParams();
  // Read once, then dropped from the address bar so it doesn't sit in history or get shared.
  const [token] = useState(() => params.get("token"));
  const [done, setDone] = useState(false);
  const router = useRouter();
  const [showPassword, setShowPassword] = useState(false);

  useEffect(() => {
    if (token) window.history.replaceState(null, "", window.location.pathname);
  }, [token]);

  if (!token) {
    return (
      <div className="flex flex-col gap-6">
        <Outcome ok={false} title="This link is incomplete">
          Open the link from the email again, or ask for a new one.
        </Outcome>
        <Button
          render={<Link href="/forgot-password" />}
          nativeButton={false}
          className="h-12 text-sm"
        >
          Get a new link
        </Button>
      </div>
    );
  }

  if (done) {
    return (
      <div className="flex flex-col gap-6">
        <Outcome ok title="Password changed">
          You&apos;ve been signed out everywhere. Sign in with your new password.
        </Outcome>
        <Button render={<Link href="/" />} nativeButton={false} className="h-12 text-sm">
          Sign in
        </Button>
      </div>
    );
  }

  const fields: Record<keyof FormValues, FormField<FormValues>> = {
    password: {
      label: "New password",
      trim: false,
      custom: true,
      customMode: "controlled",
      render: ({ field, error }) => (
        <AuthControl
          field={field}
          error={error}
          hint={PASSWORD_MESSAGE}
          id="new-password"
          label="New password"
          icon={LockKeyhole}
          type={showPassword ? "text" : "password"}
          autoComplete="new-password"
          autoFocus
          placeholder="••••••••"
          trailing={
            <PasswordToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
          }
        />
      ),
    },
  };

  const onSubmit = async ({ password }: FormValues) => {
    try {
      await resetPassword({ token, newPassword: password });
      // Every session ended, this browser's included.
      queryClient.removeQueries({ queryKey: ME_KEY });
      setDone(true);
    } catch (err) {
      // Usually an expired or used link, so offer a new one right there.
      toast.error(errorMessage(err), {
        action: { label: "Get a new link", onClick: () => router.push("/forgot-password") },
      });
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center gap-1.5 text-center">
        <h1 className="text-3xl font-bold tracking-tight">Choose a new password</h1>
        <p className="text-muted-foreground text-sm">Setting it signs you out on every device.</p>
      </div>

      <Form
        schema={schema}
        defaultValues={{ password: "" }}
        fields={fields}
        onSubmit={onSubmit}
        toastOnInvalid={false}
      >
        {({ field, isSubmitting }) => (
          <div className="flex flex-col gap-3">
            {field("password")}
            <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
              {isSubmitting ? <CircleLoader radius={8} /> : "Set new password"}
            </Button>
          </div>
        )}
      </Form>
    </div>
  );
};

const Page = () => (
  <Suspense
    fallback={
      <div className="grid h-40 place-items-center">
        <CircleLoader />
      </div>
    }
  >
    <ResetForm />
  </Suspense>
);

export default Page;
