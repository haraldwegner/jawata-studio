import { getVersion } from "@tauri-apps/api/app";

let cached: string | null = null;

/** Tauri app version from the bundle (see tauri.conf.json), or a dev placeholder. */
export async function getManagerAppVersion(): Promise<string> {
  if (cached) return cached;
  try {
    cached = await getVersion();
  } catch {
    cached = "dev";
  }
  return cached;
}

/** "v" prefix for dashboard copy when version is SemVer-like. */
export function formatManagerVersionForUi(ver: string): string {
  if (ver === "dev") return "dev";
  return ver.startsWith("v") ? ver : `v${ver}`;
}
