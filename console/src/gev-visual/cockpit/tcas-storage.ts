// GEV P20 TCAS — localStorage round-trip for the tcasEnabled toggle.
//
// Mirrors P17 (element-visibility) and P19 (svs-storage) conventions:
// namespace `intelhub.cockpit.*`, write AFTER the store reducer runs
// so the post-reduce value is what gets persisted.

export const TCAS_ENABLED_STORAGE_KEY = "intelhub.cockpit.tcasEnabled";

export function readPersistedTcasEnabled(): boolean | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(TCAS_ENABLED_STORAGE_KEY);
    if (raw === null) return null;
    return raw === "1";
  } catch {
    return null;
  }
}

export function persistTcasEnabled(enabled: boolean): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(TCAS_ENABLED_STORAGE_KEY, enabled ? "1" : "0");
  } catch {
    // quota or disabled storage — best-effort
  }
}