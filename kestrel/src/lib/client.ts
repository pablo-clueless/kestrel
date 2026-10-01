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
import type { MeResponse } from "@/types/engine/MeResponse";
import type { AuthRequest } from "@/types/engine/AuthRequest";
import type { ChangePasswordRequest } from "@/types/engine/ChangePasswordRequest";
import type { SessionInfo } from "@/types/engine/SessionInfo";
import type { PasswordResetConfirm } from "@/types/engine/PasswordResetConfirm";

// Build-time values, used by `pnpm dev` (the UI on :3000 talks to the engine on :7070).
const BUILD_TOKEN = process.env.NEXT_PUBLIC_KESTREL_TOKEN ?? "";

/** In `pnpm dev`, the engine on the page's own hostname: `localhost:3000` → `localhost:7070`. The
 * session cookie is `SameSite=Lax`, so the UI and engine must be the same *site*: `localhost` and
 * `127.0.0.1` are different sites, and mixing them would silently drop the cookie. */
const devEngineUrl = () =>
  process.env.NEXT_PUBLIC_ENGINE_URL ||
  (typeof location === "undefined"
    ? "http://localhost:7070"
    : `${location.protocol}//${location.hostname}:7070`);

/** Set when the engine serves the UI itself (single binary / container): it injects the token into the
 * page, and the API is on the same origin. Read per call: this module also loads during prerender. */
const injectedToken = () =>
  typeof document === "undefined"
    ? null
    : (document.querySelector<HTMLMetaElement>('meta[name="kestrel-token"]')?.content ?? null);

const engineUrl = () => (injectedToken() === null ? devEngineUrl() : "");
const token = () => injectedToken() ?? BUILD_TOKEN;

const WORKSPACE_KEY = "kestrel-workspace";
let memoryWorkspaceId: string | null = null;

/** The workspace id this browser already has, without making one. */
const storedWorkspaceId = (): string | undefined => {
  try {
    return localStorage.getItem(WORKSPACE_KEY) ?? memoryWorkspaceId ?? undefined;
  } catch {
    return memoryWorkspaceId ?? undefined;
  }
};

/** Remembers the workspace to send. With accounts on, the engine says which one (see `getMe`). */
const setWorkspaceId = (id: string) => {
  memoryWorkspaceId = id;
  try {
    localStorage.setItem(WORKSPACE_KEY, id);
  } catch {
    // Storage blocked (e.g. some private windows): kept for this tab only.
  }
};

/** This browser's workspace. Accounts off: a random id made on first use and kept in localStorage;
 * the engine keeps each id's data apart, and whoever knows the id can open that workspace. Accounts
 * on: the signed-in user's workspace, as `/auth/me` reported it. */
export const workspaceId = () => {
  const saved = storedWorkspaceId();
  if (saved) return saved;
  const id = crypto.randomUUID();
  setWorkspaceId(id);
  return id;
};

/** REST client for the engine. Every request carries the session token (see HANDOFF → Safety rails),
 * and, with accounts on, the session cookie (`withCredentials`, for `pnpm dev`'s second origin). */
export const engine = axios.create({ withCredentials: true });
engine.interceptors.request.use((config) => {
  config.baseURL = `${engineUrl()}/api`;
  config.headers.set("X-Kestrel-Token", token());
  config.headers.set("X-Kestrel-Workspace", workspaceId());
  return config;
});

/** A session that ended (expired, signed out elsewhere) sends the app back to sign-in. A full page
 * load, so nothing from the old session stays in memory. The auth routes report their own errors. */
engine.interceptors.response.use(undefined, (err) => {
  const url = axios.isAxiosError(err) ? (err.config?.url ?? "") : "";
  if (axios.isAxiosError(err) && err.response?.status === 401 && !url.startsWith("/auth/")) {
    // A full load on purpose (and no router outside React): drops the old session's state.
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination
    window.location.assign("/");
  }
  return Promise.reject(err);
});

/** Who's signed in. Also adopts the workspace the engine assigns, so later calls act on it. */
export const getMe = async () => {
  const me = (await engine.get<MeResponse>("/auth/me")).data;
  if (me.workspaceId) setWorkspaceId(me.workspaceId);
  return me;
};

/** Signs in or creates an account. Sends this browser's existing workspace, which a first sign-in
 * adopts if nobody owns it, so work done before signing up isn't lost. */
export const authenticate = async (mode: "login" | "signup", email: string, password: string) => {
  const req: AuthRequest = { email, password, workspace: storedWorkspaceId() };
  const me = (await engine.post<MeResponse>(`/auth/${mode}`, req)).data;
  if (me.workspaceId) setWorkspaceId(me.workspaceId);
  return me;
};

export const signOut = async () => {
  await engine.post("/auth/logout");
};

/** Every other session is signed out; this one stays. */
export const changePassword = async (req: ChangePasswordRequest) => {
  await engine.post("/auth/password", req);
};

/** This account's signed-in devices, the current one marked. */
export const listSessions = async () => (await engine.get<SessionInfo[]>("/auth/sessions")).data;

export const revokeSession = async (id: string) => {
  await engine.delete(`/auth/sessions/${id}`);
};

/** Emails a reset link if the address has an account. Succeeds either way, so it doesn't say. */
export const requestPasswordReset = async (email: string) => {
  await engine.post("/auth/password-reset", { email });
};

/** Sets a new password from a reset link. Every session ends; sign in again afterwards. */
export const resetPassword = async (req: PasswordResetConfirm) => {
  await engine.post("/auth/password-reset/confirm", req);
};

/** Confirms the email address from a verification link. Works signed out too. */
export const verifyEmail = async (token: string) => {
  await engine.post("/auth/verify-email", { token });
};

/** Emails the signed-in user a new verification link. */
export const resendVerification = async () => {
  await engine.post("/auth/verify-email/resend");
};

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

/** The report's chartable table as CSV: the timeline, or Big-O's per-size points. */
export const getReportCsv = async (runId: string) =>
  (
    await engine.get<string>(`/runs/${runId}/report`, {
      params: { format: "csv" },
      responseType: "text",
    })
  ).data;

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

/** `EventSource` can't send headers, so the SSE endpoint takes the token and workspace as query params.
 * The session cookie goes along as a cookie (open it with `withCredentials: true`). */
export const runEventsUrl = (runId: string) =>
  `${engineUrl()}/api/runs/${runId}/events?token=${encodeURIComponent(token())}&workspace=${encodeURIComponent(workspaceId())}`;

export const errorMessage = (err: unknown) => {
  if (axios.isAxiosError(err)) {
    const body = err.response?.data as { error?: string } | undefined;
    if (body?.error) return body.error;
    if (!err.response) return "Can't reach the engine. Is `cargo run -p engine` running?";
  }
  return err instanceof Error ? err.message : String(err);
};
