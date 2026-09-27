"use client";

import { useDeferredValue, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Send } from "lucide-react";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useSelectedEndpoint, useWorkspaceStore } from "@/stores/workspace-store";
import type { HttpMethod } from "@/types/engine/HttpMethod";
import { errorMessage, renderRequest } from "@/lib/client";
import type { Endpoint } from "@/types/engine/Endpoint";
import { Textarea } from "@/components/ui/textarea";
import { KeyValueEditor } from "./key-value-editor";
import { useSendStore } from "@/stores/send-store";
import type { Auth } from "@/types/engine/Auth";
import { Button } from "@/components/ui/button";
import type { Body } from "@/types/engine/Body";
import { Input } from "@/components/ui/input";
import { Label, Tabs } from "./fields";

const METHODS: HttpMethod[] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
type Tab = "query" | "headers" | "body" | "auth";

export const RequestEditor = () => {
  const endpoint = useSelectedEndpoint();
  const update = useWorkspaceStore((s) => s.updateEndpoint);
  const environment = useWorkspaceStore((s) => s.workspace?.activeEnvironment ?? null);
  const { send, pending } = useSendStore();
  const [tab, setTab] = useState<Tab>("query");

  if (!endpoint) {
    return (
      <p className="text-muted-foreground text-sm">Select or add an endpoint in the sidebar.</p>
    );
  }
  const patch = (p: Partial<Endpoint>) => update(endpoint.id, p);

  return (
    <div className="flex flex-col gap-3">
      <Input
        className="border-none px-0 text-base font-medium"
        value={endpoint.name}
        placeholder="Endpoint name"
        onChange={(e) => patch({ name: e.target.value })}
      />
      <div className="flex gap-2">
        <Select value={endpoint.method} onValueChange={(v) => patch({ method: v as HttpMethod })}>
          <SelectTrigger className="w-20 font-mono">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {METHODS.map((m) => (
              <SelectItem key={m} value={m}>
                {m}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Input
          className="flex-1 font-mono"
          value={endpoint.url}
          placeholder="{{base}}/path"
          onChange={(e) => patch({ url: e.target.value })}
          onKeyDown={(e) => e.key === "Enter" && send(endpoint, environment)}
        />
        <Button onClick={() => send(endpoint, environment)} disabled={pending}>
          <Send /> Send
        </Button>
      </div>
      <RenderedPreview endpoint={endpoint} environment={environment} />
      {endpoint.expect && (
        <p className="text-muted-foreground flex flex-wrap items-center gap-1.5 text-xs">
          Spec responses
          {endpoint.expect.responses.map((r) => (
            <span key={r.status} className="bg-muted rounded px-1.5 py-0.5 font-mono">
              {r.status}
            </span>
          ))}
          <span>· checked on Send and during runs</span>
        </p>
      )}

      <Tabs<Tab>
        value={tab}
        onChange={setTab}
        tabs={[
          { id: "query", label: "Query", count: endpoint.query.length },
          { id: "headers", label: "Headers", count: endpoint.headers.length },
          { id: "body", label: "Body" },
          { id: "auth", label: "Auth" },
        ]}
      />
      {tab === "query" && (
        <KeyValueEditor rows={endpoint.query} onChange={(query) => patch({ query })} />
      )}
      {tab === "headers" && (
        <KeyValueEditor rows={endpoint.headers} onChange={(headers) => patch({ headers })} />
      )}
      {tab === "body" && <BodyEditor body={endpoint.body} onChange={(body) => patch({ body })} />}
      {tab === "auth" && <AuthEditor auth={endpoint.auth} onChange={(auth) => patch({ auth })} />}
    </div>
  );
};

/** The fully resolved URL (secrets masked), or why it can't be resolved. */
const RenderedPreview = ({
  endpoint,
  environment,
}: {
  endpoint: Endpoint;
  environment: string | null;
}) => {
  const deferred = useDeferredValue(endpoint);
  const preview = useQuery({
    queryKey: ["render", environment, deferred],
    queryFn: () => renderRequest({ endpoint: deferred, environment, timeoutMs: null }),
    placeholderData: (prev) => prev,
  });
  return (
    <p className="truncate font-mono text-xs">
      {preview.isError ? (
        <span className="text-destructive">{errorMessage(preview.error)}</span>
      ) : (
        <span className="text-muted-foreground">→ {preview.data?.url ?? "…"}</span>
      )}
    </p>
  );
};

const BodyEditor = ({ body, onChange }: { body: Body; onChange: (b: Body) => void }) => (
  <div className="flex flex-col gap-2">
    <Select
      value={body.type}
      onValueChange={(v) => {
        const type = v as Body["type"];
        const content = body.type === "none" ? "" : body.content;
        onChange(
          type === "none"
            ? { type }
            : type === "json"
              ? { type, content }
              : { type, content, contentType: "text/plain" },
        );
      }}
    >
      <SelectTrigger className="w-20 self-start font-mono">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="none">No body</SelectItem>
        <SelectItem value="json">JSON</SelectItem>
        <SelectItem value="raw">Raw</SelectItem>
      </SelectContent>
    </Select>
    {body.type === "raw" && (
      <Input
        placeholder="Content-Type"
        value={body.contentType}
        onChange={(e) => onChange({ ...body, contentType: e.target.value })}
      />
    )}
    {body.type !== "none" && (
      <Textarea
        value={body.content}
        placeholder={body.type === "json" ? '{ "id": {{seq}} }' : ""}
        onChange={(e) => onChange({ ...body, content: e.target.value })}
      />
    )}
  </div>
);

const AuthEditor = ({ auth, onChange }: { auth: Auth; onChange: (a: Auth) => void }) => (
  <div className="flex flex-col gap-2">
    <Select
      value={auth.type}
      onValueChange={(v) => {
        const type = v as Auth["type"];
        onChange(
          type === "bearer"
            ? { type, token: "{{token}}" }
            : type === "basic"
              ? { type, username: "", password: "{{password}}" }
              : type === "apiKey"
                ? { type, location: "header", name: "X-API-Key", value: "{{apiKey}}" }
                : { type },
        );
      }}
    >
      <SelectTrigger className="w-20 self-start font-mono">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="none">No auth</SelectItem>
        <SelectItem value="bearer">Bearer token</SelectItem>
        <SelectItem value="basic">Basic</SelectItem>
        <SelectItem value="apiKey">API key</SelectItem>
      </SelectContent>
    </Select>
    {auth.type === "bearer" && (
      <Field label="Token">
        <Input
          className="font-mono"
          value={auth.token}
          onChange={(e) => onChange({ ...auth, token: e.target.value })}
        />
      </Field>
    )}
    {auth.type === "basic" && (
      <>
        <Field label="Username">
          <Input
            value={auth.username}
            onChange={(e) => onChange({ ...auth, username: e.target.value })}
          />
        </Field>
        <Field label="Password">
          <Input
            className="font-mono"
            value={auth.password}
            onChange={(e) => onChange({ ...auth, password: e.target.value })}
          />
        </Field>
      </>
    )}
    {auth.type === "apiKey" && (
      <>
        <Field label="In">
          <Select
            value={auth.location}
            onValueChange={(v) => onChange({ ...auth, location: v as "header" | "query" })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="header">Header</SelectItem>
              <SelectItem value="query">Query</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field label="Name">
          <Input value={auth.name} onChange={(e) => onChange({ ...auth, name: e.target.value })} />
        </Field>
        <Field label="Value">
          <Input
            className="font-mono"
            value={auth.value}
            onChange={(e) => onChange({ ...auth, value: e.target.value })}
          />
        </Field>
      </>
    )}
    {auth.type !== "none" && (
      <p className="text-muted-foreground text-xs">
        Put credentials in a secret (Environment card) and reference it as {"{{name}}"}. Secrets are
        stored in kestrel.secrets.json and redacted from results.
      </p>
    )}
  </div>
);

const Field = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <label className="grid grid-cols-[6rem_1fr] items-center gap-2">
    <Label>{label}</Label>
    {children}
  </label>
);
