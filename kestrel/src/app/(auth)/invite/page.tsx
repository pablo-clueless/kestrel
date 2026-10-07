"use client";

import { Suspense, useEffect, useRef, useState } from "react";
import { CircleCheck, TriangleAlert, Users } from "lucide-react";
import { useSearchParams } from "next/navigation";
import Link from "next/link";

import { acceptInvite, errorMessage, signOut, switchWorkspace } from "@/lib/client";
import { CircleLoader } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { useMe } from "@/hooks/use-me";

/** Where the token waits while the invitee signs in (or switches accounts): the link's address bar
 * is cleared straight away, so it doesn't linger in history. */
const PENDING_KEY = "kestrel-invite";

type State =
  | { status: "waiting" }
  | { status: "joining" }
  | { status: "joined"; workspaceId: string; alreadyMember: boolean }
  | { status: "failed"; error: string; wrongAccount: boolean };

const readPending = () => {
  try {
    return sessionStorage.getItem(PENDING_KEY);
  } catch {
    return null;
  }
};

const writePending = (token: string | null) => {
  try {
    if (token) sessionStorage.setItem(PENDING_KEY, token);
    else sessionStorage.removeItem(PENDING_KEY);
  } catch {
    // Storage blocked: the invite then needs the link opened again after signing in.
  }
};

/** Accepts an invite to a shared workspace. Signed out, it sends the invitee to sign in (or sign up)
 * and back here; signed in, it joins straight away. */
const AcceptInvite = () => {
  const params = useSearchParams();
  const me = useMe();
  const [token] = useState(() => params.get("token") ?? readPending());
  const [state, setState] = useState<State>({ status: "waiting" });
  // The token is single-use: an effect that runs twice (React's dev mode does this) must not send
  // it twice, or the second call would report the invite as used.
  const sent = useRef(false);

  useEffect(() => {
    if (!token) return;
    writePending(token);
    if (params.get("token")) window.history.replaceState(null, "", window.location.pathname);
  }, [token, params]);

  const user = me.data?.user;
  useEffect(() => {
    if (!token || !user || sent.current) return;
    sent.current = true;
    setState({ status: "joining" });
    acceptInvite(token)
      .then((res) => {
        writePending(null);
        setState({
          status: "joined",
          workspaceId: res.workspaceId,
          alreadyMember: res.alreadyMember,
        });
      })
      .catch((err) => {
        const error = errorMessage(err);
        const wrongAccount = /different email/i.test(error);
        // A used or expired link won't start working; one for another address might, once signed
        // in as that address.
        if (!wrongAccount) writePending(null);
        setState({ status: "failed", error, wrongAccount });
      });
  }, [token, user]);

  if (me.isPending || state.status === "joining") {
    return (
      <div className="flex flex-col items-center gap-4 text-center">
        <CircleLoader />
        <p className="text-muted-foreground text-sm">
          {state.status === "joining" ? "Joining the workspace…" : "Checking your invite…"}
        </p>
      </div>
    );
  }

  if (!token) {
    return (
      <Outcome
        ok={false}
        title="This link is incomplete"
        detail="Open the invite link again, exactly as you received it."
        action={{ label: "Go to Kestrel", href: "/" }}
      />
    );
  }

  if (me.data?.auth === "off") {
    return (
      <Outcome
        ok={false}
        title="Accounts are off"
        detail="This engine runs without accounts, so there are no shared workspaces to join."
        action={{ label: "Go to your workspace", href: "/workspace" }}
      />
    );
  }

  if (!user) {
    return (
      <div className="flex flex-col gap-6">
        <div className="flex flex-col items-center gap-6 text-center">
          <span className="bg-primary/10 text-primary grid size-14 place-items-center rounded-2xl">
            <Users className="size-7" />
          </span>
          <div className="flex flex-col gap-1.5">
            <h1 className="text-3xl font-bold tracking-tight">You&apos;re invited</h1>
            <p className="text-muted-foreground text-sm">
              Sign in with the email address the invite was sent to, or create an account with it,
              to join the workspace.
            </p>
          </div>
        </div>
        <Button
          render={<Link href="/?next=/invite" />}
          nativeButton={false}
          className="h-12 text-sm"
        >
          Sign in to accept
        </Button>
      </div>
    );
  }

  if (state.status === "joined") {
    return (
      <Outcome
        ok
        title={state.alreadyMember ? "You're already in" : "You've joined"}
        detail={
          state.alreadyMember
            ? "You were already a member of this workspace, so nothing changed."
            : "The workspace is now in your list, next to your own."
        }
        action={{ label: "Open the workspace", onClick: () => switchWorkspace(state.workspaceId) }}
      />
    );
  }

  if (state.status === "failed") {
    const switchAccount = async () => {
      await signOut().catch(() => undefined);
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination
      window.location.assign("/?next=/invite");
    };
    return (
      <Outcome
        ok={false}
        title="Couldn't join"
        detail={
          state.wrongAccount
            ? `You're signed in as ${user.email}, but this invite is for another address. Sign in with that one to accept it.`
            : `${state.error}. Ask one of the workspace's admins to invite you again.`
        }
        action={
          state.wrongAccount
            ? { label: "Sign in with another account", onClick: switchAccount }
            : { label: "Go to your workspace", href: "/workspace" }
        }
      />
    );
  }

  return null;
};

const Outcome = ({
  ok,
  title,
  detail,
  action,
}: {
  ok: boolean;
  title: string;
  detail: string;
  action: { label: string; href: string } | { label: string; onClick: () => void };
}) => (
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
        <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
        <p className="text-muted-foreground text-sm">{detail}</p>
      </div>
    </div>
    {"href" in action ? (
      <Button render={<Link href={action.href} />} nativeButton={false} className="h-12 text-sm">
        {action.label}
      </Button>
    ) : (
      <Button onClick={action.onClick} className="h-12 text-sm">
        {action.label}
      </Button>
    )}
  </div>
);

const Page = () => (
  <Suspense
    fallback={
      <div className="grid h-40 place-items-center">
        <CircleLoader />
      </div>
    }
  >
    <AcceptInvite />
  </Suspense>
);

export default Page;
