"use client";

import { useDeferredValue, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Save, Send } from "lucide-react";
import { toast } from "sonner";

import { errorMessage, looksLikeCurl, parseCurl, renderRequest } from "@/lib/client";
import { useSendEntry, useSendStore } from "@/stores/send-store";
import type { HttpMethod } from "@/types/engine/HttpMethod";
import type { FormField } from "@/types/engine/FormField";
import type { Endpoint } from "@/types/engine/Endpoint";
import { FormFieldEditor } from "./form-field-editor";
import { Textarea } from "@/components/ui/textarea";
import { KeyValueEditor } from "./key-value-editor";
import { ExtractEditor } from "./extract-editor";
import type { Auth } from "@/types/engine/Auth";
import { Button } from "@/components/ui/button";
import type { Body } from "@/types/engine/Body";
import { Input } from "@/components/ui/input";
import { GroupPicker } from "./group-picker";
import { JsonEditor } from "./json-editor";
import { Label, Tabs } from "./fields";
import { cn } from "@/lib/utils";
import { Editable } from "../shared/editable";
import {
  useActiveCollection,
  useSelectedEndpoint,
  useSelectedIsDraft,
  useWorkspaceStore,
  groupsOf,
} from "@/stores/workspace-store";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/** Every text in the request editor is text-xs. `md:` too, since the ui inputs set `md:text-sm`. */
export const XS = "text-xs md:text-xs";

const METHODS: HttpMethod[] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
type Tab = "query" | "headers" | "body" | "auth" | "after";

const CONTENT_TYPES = [
  "application/json",
  "application/x-www-form-urlencoded",
  "application/form-data",
  "application/octet-stream",
  "text/html",
  "text/plain",
];

/** Names new endpoints start with, which a pasted curl may replace. */
const PLACEHOLDER_NAMES = new Set(["", "Untitled request", "New endpoint"]);

/** `https://api.x.io/users` → `{{base}}/users` when the collection's `base` is `https://api.x.io`. */
const relativeToBase = (url: string, base: string | undefined) => {
  const root = base?.replace(/\/+$/, "");
  if (!root || !/^https?:\/\//.test(root) || !url.startsWith(root)) return url;
  const rest = url.slice(root.length);
  return rest === "" || rest.startsWith("/") ? `{{base}}${rest}` : url;
};

export const RequestEditor = () => {
  const endpoint = useSelectedEndpoint();
  const isDraft = useSelectedIsDraft();
  const collection = useActiveCollection();
  /** Names of the collection's enabled headers, sent with this endpoint too. */
  const sharedHeaders = (collection?.headers ?? [])
    .filter((h) => h.enabled && h.key.trim() !== "")
    .map((h) => h.key.trim());
  const saveDraft = useWorkspaceStore((s) => s.saveDraft);
  const update = useWorkspaceStore((s) => s.updateEndpoint);
  const addGroup = useWorkspaceStore((s) => s.addGroup);
  const environment = useWorkspaceStore((s) => s.workspace?.activeEnvironment ?? null);
  const send = useSendStore((s) => s.send);
  const { pending } = useSendEntry(endpoint?.id);
  const [tab, setTab] = useState<Tab>("query");

  if (!endpoint) {
    return (
      <p className="text-muted-foreground text-xs">
        Open a new tab, or select an endpoint in the sidebar.
      </p>
    );
  }
  const patch = (p: Partial<Endpoint>) => update(endpoint.id, p);
  const groups = collection ? groupsOf(collection) : [];

  /** A curl pasted into the URL fills in the whole request (undoably). */
  const pasteCurl = async (command: string) => {
    const before = endpoint;
    try {
      const { endpoint: parsed, warnings } = await parseCurl(command);
      patch({
        method: parsed.method,
        url: relativeToBase(parsed.url, collection?.vars.base),
        query: parsed.query,
        headers: parsed.headers,
        body: parsed.body,
        auth: parsed.auth,
        // A name you chose stays; a placeholder one gives way to the URL's path.
        ...(PLACEHOLDER_NAMES.has(before.name.trim()) ? { name: parsed.name } : {}),
      });
      toast.success("Filled in from curl", {
        description: warnings.length ? warnings.join(" ") : undefined,
        duration: warnings.length ? 10_000 : 4_000,
        action: { label: "Undo", onClick: () => update(before.id, before) },
      });
    } catch (err) {
      toast.error(`Couldn't read the curl command: ${errorMessage(err)}`);
    }
  };

  return (
    <div className="flex flex-col gap-3 text-xs">
      <Editable className="flex flex-col gap-3">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <GroupPicker
              value={endpoint.group}
              groups={groups}
              onChange={(group) => {
                // A group made here is kept like one made in the sidebar, so it survives emptying.
                if (group && collection) addGroup(collection.id, group);
                patch({ group });
              }}
            />
            <Input
              className={cn("max-w-100 border-none px-2 font-medium", XS)}
              value={endpoint.name}
              placeholder="Endpoint name"
              onChange={(e) => patch({ name: e.target.value })}
            />
          </div>
          {isDraft && (
            <Button variant="outline" className={XS} onClick={() => saveDraft(endpoint.id)}>
              <Save /> Save to {collection?.name ?? "a new collection"}
            </Button>
          )}
        </div>
        <div className="flex gap-2">
          <Select value={endpoint.method} onValueChange={(v) => patch({ method: v as HttpMethod })}>
            <SelectTrigger className={cn("w-25 font-mono", XS)}>
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
            className={cn("flex-1 font-mono", XS)}
            value={endpoint.url}
            placeholder="{{base}}/path, or paste a curl command"
            onChange={(e) => patch({ url: e.target.value })}
            onPaste={(e) => {
              const text = e.clipboardData.getData("text");
              if (!looksLikeCurl(text)) return;
              e.preventDefault();
              void pasteCurl(text);
            }}
            onKeyDown={(e) => e.key === "Enter" && send(endpoint, environment)}
          />
          <Button className={XS} onClick={() => send(endpoint, environment)} disabled={pending}>
            <Send /> Send
          </Button>
        </div>
      </Editable>
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
          { id: "after", label: "On Response", count: endpoint.extract?.length ?? 0 },
        ]}
      />
      {/* The tabs above stay usable, so read access can look at every part of the request. */}
      <Editable>
        {tab === "query" && (
          <KeyValueEditor rows={endpoint.query} onChange={(query) => patch({ query })} />
        )}
        {tab === "headers" && (
          <div className="flex flex-col gap-2">
            <KeyValueEditor rows={endpoint.headers} onChange={(headers) => patch({ headers })} />
            {sharedHeaders.length > 0 && (
              <p className="text-muted-foreground text-xs">
                Also sends {sharedHeaders.join(", ")} from the collection settings, unless set here.
              </p>
            )}
          </div>
        )}
        {tab === "body" && <BodyEditor body={endpoint.body} onChange={(body) => patch({ body })} />}
        {tab === "auth" && <AuthEditor auth={endpoint.auth} onChange={(auth) => patch({ auth })} />}
        {tab === "after" && (
          <ExtractEditor rules={endpoint.extract} onChange={(extract) => patch({ extract })} />
        )}
      </Editable>
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
  const empty = !deferred.url.trim();
  const preview = useQuery({
    queryKey: ["render", environment, deferred],
    queryFn: () => renderRequest({ endpoint: deferred, environment, timeoutMs: null }),
    placeholderData: (prev) => prev,
    enabled: !empty,
  });
  // Nothing to resolve yet (a fresh request); an error here would only be noise.
  if (empty) return null;
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

/** Switches body type, carrying text between JSON/raw and fields between the two form types
 * (file fields are dropped going to URL-encoded, which can't carry them). */
const withType = (body: Body, type: Body["type"]): Body => {
  const content = body.type === "json" || body.type === "raw" ? body.content : "";
  const fields: FormField[] =
    body.type === "multipart"
      ? body.fields
      : body.type === "form"
        ? body.fields.map((f) => ({ ...f, kind: "text", file: null }))
        : [];
  switch (type) {
    case "none":
      return { type };
    case "json":
      return { type, content };
    case "raw":
      return { type, content, contentType: "text/plain" };
    case "form":
      return {
        type,
        fields: fields
          .filter((f) => f.kind === "text")
          .map(({ key, value, enabled }) => ({ key, value, enabled })),
      };
    case "multipart":
      return { type, fields };
  }
};

const BodyEditor = ({ body, onChange }: { body: Body; onChange: (b: Body) => void }) => (
  <div className="flex flex-col gap-2">
    <div className="flex items-center gap-x-4">
      <Select value={body.type} onValueChange={(v) => onChange(withType(body, v as Body["type"]))}>
        <SelectTrigger className={cn("w-50 self-start font-mono capitalize", XS)}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="none">None</SelectItem>
          <SelectItem value="json">JSON</SelectItem>
          <SelectItem value="form">Form URL-encoded</SelectItem>
          <SelectItem value="multipart">Multipart form data</SelectItem>
          <SelectItem value="raw">Raw</SelectItem>
        </SelectContent>
      </Select>
      {body.type === "raw" && (
        <Select
          value={body.contentType}
          onValueChange={(value) => onChange({ ...body, contentType: value || "" })}
        >
          <SelectTrigger className={cn("w-75 self-start font-mono", XS)}>
            <SelectValue placeholder="Content-Type" />
          </SelectTrigger>
          <SelectContent>
            {CONTENT_TYPES.map((type) => (
              <SelectItem key={type} value={type}>
                {type}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}
    </div>
    {body.type === "json" && (
      <JsonEditor
        className={XS}
        value={body.content}
        placeholder='{ "id": {{seq}} }'
        onChange={(content) => onChange({ ...body, content })}
      />
    )}
    {body.type === "raw" && (
      <Textarea
        className={cn("font-mono", XS)}
        value={body.content}
        onChange={(e) => onChange({ ...body, content: e.target.value })}
      />
    )}
    {body.type === "form" && (
      <>
        <KeyValueEditor
          rows={body.fields}
          onChange={(fields) => onChange({ ...body, fields })}
          keyPlaceholder="field"
        />
        <p className="text-muted-foreground text-xs">Sent as application/x-www-form-urlencoded.</p>
      </>
    )}
    {body.type === "multipart" && (
      <>
        <FormFieldEditor rows={body.fields} onChange={(fields) => onChange({ ...body, fields })} />
        <p className="text-muted-foreground text-xs">
          Sent as multipart/form-data. Files are uploaded to the engine and kept in its database (up
          to 50 MB each).
        </p>
      </>
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
      <SelectTrigger className={cn("w-50 self-start font-mono capitalize", XS)}>
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
          className={cn("font-mono", XS)}
          value={auth.token}
          onChange={(e) => onChange({ ...auth, token: e.target.value })}
        />
      </Field>
    )}
    {auth.type === "basic" && (
      <>
        <Field label="Username">
          <Input
            className={XS}
            value={auth.username}
            onChange={(e) => onChange({ ...auth, username: e.target.value })}
          />
        </Field>
        <Field label="Password">
          <Input
            className={cn("font-mono", XS)}
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
            <SelectTrigger className={XS}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="header">Header</SelectItem>
              <SelectItem value="query">Query</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field label="Name">
          <Input
            className={XS}
            value={auth.name}
            onChange={(e) => onChange({ ...auth, name: e.target.value })}
          />
        </Field>
        <Field label="Value">
          <Input
            className={cn("font-mono", XS)}
            value={auth.value}
            onChange={(e) => onChange({ ...auth, value: e.target.value })}
          />
        </Field>
      </>
    )}
    {auth.type !== "none" && (
      <p className="text-muted-foreground text-xs">
        Put credentials in a secret (Environment card) and reference it as {"{{name}}"}. Secrets are
        stored in the engine&apos;s database and redacted from results.
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
