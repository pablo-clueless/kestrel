import type { NextConfig } from "next";

// One `.env` at the repo root configures both the engine and the UI.
try {
  process.loadEnvFile("../.env");
} catch {
  // No root .env: fall back to whatever is already in the environment.
}

const enginePort = process.env.KESTREL_PORT ?? "7070";

const allowedDevOrigins = (process.env.ALLOWED_DEV_ORIGINS || "").split(",");

const nextConfig: NextConfig = {
  // Static export so the engine can serve the UI itself later (single-binary mode). This rules out
  // server actions and route handlers: all data comes from the engine.
  output: "export",
  images: { unoptimized: true },
  env: {
    NEXT_PUBLIC_KESTREL_TOKEN: process.env.KESTREL_TOKEN ?? "",
    // Empty unless set: the UI then calls the engine on the page's own hostname (see
    // `devEngineUrl`), which the session cookie needs. A fixed 127.0.0.1 broke sign-in from
    // localhost or a LAN address, because the browser dropped the cookie as cross-site.
    NEXT_PUBLIC_ENGINE_URL: process.env.KESTREL_ENGINE_URL ?? "",
    NEXT_PUBLIC_ENGINE_PORT: enginePort,
  },
  allowedDevOrigins,
};

export default nextConfig;
