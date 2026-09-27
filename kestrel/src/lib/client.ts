import axios from "axios";

import type { RunConfig } from "@/types/engine/RunConfig";
import type { RunReport } from "@/types/engine/RunReport";
import type { RunSummary } from "@/types/engine/RunSummary";
import type { Sample } from "@/types/engine/Sample";
import type { SentRequest } from "@/types/engine/SentRequest";
import type { SetSecretRequest } from "@/types/engine/SetSecretRequest";
import type { StartRunResponse } from "@/types/engine/StartRunResponse";
import type { TryRequest } from "@/types/engine/TryRequest";
import type { Workspace } from "@/types/engine/Workspace";
import type { WorkspaceResponse } from "@/types/engine/WorkspaceResponse";

export const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL ?? "http://127.0.0.1:7070";
const TOKEN = process.env.NEXT_PUBLIC_KESTREL_TOKEN ?? "";

/** REST client for the engine. Every request carries the session token (see HANDOFF → Safety rails). */
export const engine = axios.create({
  baseURL: `${ENGINE_URL}/api`,
  headers: { "X-Kestrel-Token": TOKEN },
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

/** The "try it" button: one request, redacted response. */
export const sendRequest = async (req: TryRequest) => (await engine.post<Sample>("/send", req)).data;

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
  `${ENGINE_URL}/api/runs/${runId}/events?token=${encodeURIComponent(TOKEN)}`;

export const errorMessage = (err: unknown) => {
  if (axios.isAxiosError(err)) {
    const body = err.response?.data as { error?: string } | undefined;
    if (body?.error) return body.error;
    if (!err.response) return "Can't reach the engine. Is `cargo run -p engine` running?";
  }
  return err instanceof Error ? err.message : String(err);
};
