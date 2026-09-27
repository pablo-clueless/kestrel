import type { NextConfig } from "next";

// One `.env` at the repo root configures both the engine and the UI.
try {
  process.loadEnvFile("../.env");
} catch {
  // No root .env: fall back to whatever is already in the environment.
}

const enginePort = process.env.KESTREL_PORT ?? "7070";

const nextConfig: NextConfig = {
  // Static export so the engine can serve the UI itself later (single-binary mode). This rules out
  // server actions and route handlers: all data comes from the engine.
  output: "export",
  env: {
    NEXT_PUBLIC_KESTREL_TOKEN: process.env.KESTREL_TOKEN ?? "",
    NEXT_PUBLIC_ENGINE_URL: process.env.KESTREL_ENGINE_URL ?? `http://127.0.0.1:${enginePort}`,
  },
};

export default nextConfig;
