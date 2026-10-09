"use client";

import { useQuery } from "@tanstack/react-query";

import { AdminPage, QueryState } from "@/components/admin";
import { Card } from "@/components/shared";
import { adminSettings } from "@/lib/client";
import { int } from "@/lib/format";

/** One setting: what it is, its value, and the variable that sets it. */
const Row = ({ label, value, env }: { label: string; value: React.ReactNode; env: string }) => (
  <div className="grid grid-cols-[minmax(0,14rem)_1fr] items-baseline gap-4 py-2.5 text-sm">
    <dt className="flex flex-col">
      <span>{label}</span>
      <code className="text-muted-foreground text-[11px]">{env}</code>
    </dt>
    <dd className="min-w-0 break-words">{value}</dd>
  </div>
);

const Off = ({ children = "Not set" }: { children?: React.ReactNode }) => (
  <span className="text-muted-foreground">{children}</span>
);

const Page = () => {
  const settings = useQuery({ queryKey: ["admin", "settings"], queryFn: adminSettings });

  return (
    <AdminPage
      title="Settings"
      description="How this engine is set up. It's all environment variables: change them where the engine runs, then restart it. Secrets (the token, database URL, SMTP password) aren't shown."
    >
      <QueryState query={settings}>
        {() => {
          const s = settings.data!;
          return (
            <>
              <Card title="Accounts">
                <dl className="divide-y">
                  <Row
                    label="Sign-up"
                    env="KESTREL_SIGNUP"
                    value={
                      s.signupOpen
                        ? "Open: anyone can create an account"
                        : "Closed: accounts are made with `engine user add`"
                    }
                  />
                  <Row label="Admins" env="KESTREL_ADMIN_EMAILS" value={s.adminEmails.join(", ")} />
                  <Row
                    label="Public URL"
                    env="KESTREL_PUBLIC_URL"
                    value={s.publicUrl ?? <Off>Not set: email links use the page that asked</Off>}
                  />
                </dl>
              </Card>
              <Card title="Email">
                {s.mail ? (
                  <dl className="divide-y">
                    <Row
                      label="Server"
                      env="KESTREL_SMTP_HOST, _PORT"
                      value={`${s.mail.host}:${s.mail.port}`}
                    />
                    <Row label="Encryption" env="KESTREL_SMTP_TLS" value={s.mail.tls} />
                    <Row label="Sender" env="KESTREL_SMTP_FROM" value={s.mail.from} />
                    <Row
                      label="Signs in"
                      env="KESTREL_SMTP_USERNAME"
                      value={s.mail.authenticated ? "Yes" : "No"}
                    />
                  </dl>
                ) : (
                  <p className="text-muted-foreground text-sm">
                    Off (<code>KESTREL_SMTP_HOST</code> isn&apos;t set): no email verification,
                    password reset links or emailed invites.
                  </p>
                )}
              </Card>
              <Card title="Server">
                <dl className="divide-y">
                  <Row label="Version" env="(build)" value={s.version} />
                  <Row
                    label="Listening on"
                    env="KESTREL_BIND, KESTREL_PORT / PORT"
                    value={s.listen}
                  />
                  <Row
                    label="Extra allowed hosts"
                    env="KESTREL_ALLOWED_HOSTS"
                    value={
                      s.allowedHosts.length ? s.allowedHosts.join(", ") : <Off>Only localhost</Off>
                    }
                  />
                  <Row
                    label="Behind a trusted proxy"
                    env="KESTREL_TRUSTED_PROXY"
                    value={s.trustedProxy ? "Yes: forwarded client IPs are believed" : "No"}
                  />
                  <Row
                    label="Database connections"
                    env="KESTREL_DB_MAX_CONNECTIONS"
                    value={`Up to ${s.dbMaxConnections}`}
                  />
                </dl>
              </Card>
              <Card title="Test limits">
                <dl className="divide-y">
                  <Row
                    label="Requests per second"
                    env="KESTREL_MAX_RPS"
                    value={int(s.caps.maxRps)}
                  />
                  <Row
                    label="In flight / virtual users"
                    env="KESTREL_MAX_IN_FLIGHT"
                    value={int(s.caps.maxInFlight)}
                  />
                  <Row
                    label="Test duration"
                    env="KESTREL_MAX_DURATION_S"
                    value={`${int(s.caps.maxDurationS)} s`}
                  />
                  <Row
                    label="Request timeout"
                    env="KESTREL_MAX_TIMEOUT_S"
                    value={`${int(s.caps.maxTimeoutMs)} ms`}
                  />
                  <Row label="Samples" env="KESTREL_MAX_SAMPLES" value={int(s.caps.maxSamples)} />
                  <Row
                    label="Warm-up requests"
                    env="KESTREL_MAX_WARMUP"
                    value={int(s.caps.maxWarmup)}
                  />
                  <Row
                    label="Big-O sweep duration"
                    env="KESTREL_MAX_SWEEP_S"
                    value={`${int(s.caps.maxSweepS)} s`}
                  />
                  <Row label="Big-O largest n" env="KESTREL_MAX_N" value={int(s.caps.maxN)} />
                  <Row
                    label="Big-O points"
                    env="KESTREL_MAX_POINTS"
                    value={int(s.caps.maxPoints)}
                  />
                </dl>
              </Card>
            </>
          );
        }}
      </QueryState>
    </AdminPage>
  );
};

export default Page;
