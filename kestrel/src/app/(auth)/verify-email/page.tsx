"use client";

import { useQueryClient } from "@tanstack/react-query";
import { CircleCheck, TriangleAlert } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Suspense, useEffect, useRef, useState } from "react";

import { errorMessage, verifyEmail } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { ME_KEY, useMe } from "@/hooks/use-me";

type State = { status: "verifying" } | { status: "verified" } | { status: "failed"; error: string };

/** Confirms the address from the emailed link as soon as it opens. Works signed in or out. */
const Verify = () => {
  const params = useSearchParams();
  const queryClient = useQueryClient();
  const me = useMe();
  const [token] = useState(() => params.get("token"));
  const [state, setState] = useState<State>(
    token ? { status: "verifying" } : { status: "failed", error: "This link is incomplete." },
  );
  // The token is single-use: an effect that runs twice (React's dev mode does this) must not send
  // it twice, or the second call would report the link as used.
  const sent = useRef(false);

  useEffect(() => {
    if (!token || sent.current) return;
    sent.current = true;
    window.history.replaceState(null, "", window.location.pathname);
    verifyEmail(token)
      .then(() => {
        setState({ status: "verified" });
        void queryClient.invalidateQueries({ queryKey: ME_KEY });
      })
      .catch((err) => setState({ status: "failed", error: errorMessage(err) }));
  }, [token, queryClient]);

  if (state.status === "verifying") {
    return (
      <div className="flex flex-col items-center gap-4 text-center">
        <CircleLoader />
        <p className="text-muted-foreground text-sm">Confirming your email…</p>
      </div>
    );
  }

  const signedIn = !!me.data?.user;
  // An old or used link opened after the address was confirmed some other way: nothing to fix.
  const already = state.status === "failed" && !!me.data?.user?.emailVerified;
  const ok = state.status === "verified" || already;
  return (
    <div className="flex flex-col gap-6">
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
          <h1 className="text-3xl font-bold tracking-tight">
            {already ? "Already confirmed" : ok ? "Email confirmed" : "Couldn't confirm your email"}
          </h1>
          <p className="text-muted-foreground text-sm">
            {already
              ? "This link was already used, but your address is verified, so there's nothing to do."
              : ok
                ? "Thanks. Your address is verified."
                : `${state.error.replace(/^./, (c) => c.toUpperCase()).replace(/\.?$/, ".")} ${
                    signedIn
                      ? "Send a new link from Profile → Account."
                      : "Sign in and send a new link from Profile → Account."
                  }`}
          </p>
        </div>
      </div>
      <Button
        render={<Link href={signedIn ? (ok ? "/workspace" : "/profile") : "/"} />}
        nativeButton={false}
        className="h-12 text-sm"
      >
        {signedIn ? (ok ? "Go to your workspace" : "Open your profile") : "Sign in"}
      </Button>
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
    <Verify />
  </Suspense>
);

export default Page;
