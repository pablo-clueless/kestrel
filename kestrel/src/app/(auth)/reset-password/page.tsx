"use client";

import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { CircleCheck, LockKeyhole, TriangleAlert } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";

import { PASSWORD_MAX, PASSWORD_MESSAGE, PASSWORD_MIN } from "@/config/string";
import { errorMessage, resetPassword } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { AuthField, PasswordToggle } from "@/components/shared/auth-field";
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
  const [error, setError] = useState<string | null>(null);
  const [showPassword, setShowPassword] = useState(false);
  const {
    register,
    handleSubmit,
    formState: { errors, isSubmitting },
  } = useForm<FormValues>({ defaultValues: { password: "" }, resolver: zodResolver(schema) });

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

  const onSubmit = async ({ password }: FormValues) => {
    setError(null);
    try {
      await resetPassword({ token, newPassword: password });
      // Every session ended, this browser's included.
      queryClient.removeQueries({ queryKey: ME_KEY });
      setDone(true);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col items-center gap-1.5 text-center">
        <h1 className="text-3xl font-bold tracking-tight">Choose a new password</h1>
        <p className="text-muted-foreground text-sm">Setting it signs you out on every device.</p>
      </div>

      <form className="flex flex-col gap-3" onSubmit={handleSubmit(onSubmit)} noValidate>
        <div className="flex flex-col gap-1.5">
          <AuthField
            id="new-password"
            label="New password"
            icon={LockKeyhole}
            type={showPassword ? "text" : "password"}
            autoComplete="new-password"
            autoFocus
            placeholder="••••••••"
            invalid={!!errors.password}
            aria-describedby="password-hint"
            trailing={
              <PasswordToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
            }
            {...register("password")}
          />
          {errors.password ? (
            <p className="text-destructive text-xs">{errors.password.message}</p>
          ) : (
            <p id="password-hint" className="text-muted-foreground text-xs">
              {PASSWORD_MESSAGE}
            </p>
          )}
        </div>

        {error && (
          <div role="alert" className="text-destructive flex flex-col gap-1 text-sm">
            <p>{error}</p>
            <Link href="/forgot-password" className="text-primary font-medium hover:underline">
              Get a new link
            </Link>
          </div>
        )}

        <Button type="submit" disabled={isSubmitting} className="mt-2 h-12 text-sm">
          {isSubmitting ? <CircleLoader radius={8} /> : "Set new password"}
        </Button>
      </form>
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
