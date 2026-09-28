import axios from "axios";

import type { WorkspaceResponse } from "@/types/engine/WorkspaceResponse";
import type { SetSecretRequest } from "@/types/engine/SetSecretRequest";
import type { StartRunResponse } from "@/types/engine/StartRunResponse";
import type { SentRequest } from "@/types/engine/SentRequest";
import type { TryRequest } from "@/types/engine/TryRequest";
import type { RunSummary } from "@/types/engine/RunSummary";
import type { RunConfig } from "@/types/engine/RunConfig";
import type { RunReport } from "@/types/engine/RunReport";
import type { Workspace } from "@/types/engine/Workspace";
import type { SendResponse } from "@/types/engine/SendResponse";
import type { FileRef } from "@/types/engine/FileRef";
import type { ImportRequest } from "@/types/engine/ImportRequest";
import type { ImportResult } from "@/types/engine/ImportResult";

// Build-time values, used by `pnpm dev` (the UI on :3000 talks to the engine on :7070).
const BUILD_ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL ?? "http://127.0.0.1:7070";
const BUILD_TOKEN = process.env.NEXT_PUBLIC_KESTREL_TOKEN ?? "";

/** Set when the engine serves the UI itself (single binary / container): it injects the token into the
 * page, and the API is on the same origin. Read per call: this module also loads during prerender. */
const injectedToken = () =>
  typeof document === "undefined"
    ? null
    : (document.querySelector<HTMLMetaElement>('meta[name="kestrel-token"]')?.content ?? null);

const engineUrl = () => (injectedToken() === null ? BUILD_ENGINE_URL : "");
const token = () => injectedToken() ?? BUILD_TOKEN;

/** REST client for the engine. Every request carries the session token (see HANDOFF → Safety rails). */
export const engine = axios.create();
engine.interceptors.request.use((config) => {
  config.baseURL = `${engineUrl()}/api`;
  config.headers.set("X-Kestrel-Token", token());
  return config;
});

export const getHealth = async () =>
  (await engine.get<{ ok: boolean; version: string }>("/health")).data;

export const listRuns = async () => (await engine.get<RunSummary[]>("/runs")).data;

export const startRun = async (config: RunConfig) =>
  (await engine.post<StartRunResponse>("/runs", config)).data;

export const stopRun = async (runId: string) => {
  await engine.delete(`/runs/${runId}`);
};

export const getReport = async (runId: string) =>
  (await engine.get<RunReport>(`/runs/${runId}/report`)).data;

export const getWorkspace = async () => (await engine.get<WorkspaceResponse>("/workspace")).data;

export const putWorkspace = async (workspace: Workspace) => {
  await engine.put("/workspace", workspace);
};

/** Secrets are write-only: `value: null` deletes. */
export const setSecret = async (req: SetSecretRequest) => {
  await engine.put("/secrets", req);
};

/** The request as it would be sent, secrets masked. */
export const renderRequest = async (req: TryRequest) =>
  (await engine.post<SentRequest>("/render", req)).data;

/** The "try it" button: one request, redacted response, plus what the extract rules saved. */
export const sendRequest = async (req: TryRequest) =>
  (await engine.post<SendResponse>("/send", req)).data;

/** Stores a file for a multipart file field; the field keeps the returned reference. */
export const uploadFile = async (file: File) =>
  (
    await engine.post<FileRef>("/files", file, {
      params: { name: file.name },
      headers: { "Content-Type": file.type || "application/octet-stream" },
    })
  ).data;

/** Parses an OpenAPI/Swagger spec into a collection. Nothing is saved until the UI adds it. */
export const importSpec = async (req: ImportRequest) =>
  (await engine.post<ImportResult>("/import", req)).data;

/** Hosts confirmed for load testing this engine session. */
export const listHosts = async () => (await engine.get<string[]>("/hosts")).data;

/** Records "I own or am authorised to test this host". */
export const confirmHost = async (host: string) => {
  await engine.post("/hosts/confirm", { host });
};

/** If the engine refused a load test because the host isn't confirmed yet, the host. */
export const unconfirmedHost = (err: unknown): string | null => {
  if (!axios.isAxiosError(err)) return null;
  const body = err.response?.data as { code?: string; host?: string } | undefined;
  return body?.code === "hostNotConfirmed" && body.host ? body.host : null;
};

/** `EventSource` can't send headers, so the SSE endpoint alone takes the token as a query param. */
export const runEventsUrl = (runId: string) =>
  `${engineUrl()}/api/runs/${runId}/events?token=${encodeURIComponent(token())}`;

export const errorMessage = (err: unknown) => {
  if (axios.isAxiosError(err)) {
    const body = err.response?.data as { error?: string } | undefined;
    if (body?.error) return body.error;
    if (!err.response) return "Can't reach the engine. Is `cargo run -p engine` running?";
  }
  return err instanceof Error ? err.message : String(err);
};
