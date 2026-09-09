// IntelHub Console API client — bearer key from localStorage (console agent
// identity, §66 attribution). 401 clears the key and bounces to the gate.

const KEY_STORAGE = "intelhub.console.key";

export function getKey(): string | null {
  return localStorage.getItem(KEY_STORAGE);
}
export function setKey(key: string) {
  localStorage.setItem(KEY_STORAGE, key.trim());
}
export function clearKey() {
  localStorage.removeItem(KEY_STORAGE);
}

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export async function api<T = unknown>(path: string, init: RequestInit = {}): Promise<T> {
  const key = getKey();
  const headers: Record<string, string> = {
    ...(init.headers as Record<string, string>),
  };
  if (key) headers["Authorization"] = `Bearer ${key}`;
  if (init.body) headers["Content-Type"] = "application/json";
  const resp = await fetch(path, { ...init, headers });
  if (resp.status === 401) {
    clearKey();
    window.dispatchEvent(new Event("intelhub:unauthorized"));
    throw new ApiError(401, "unauthorized");
  }
  const text = await resp.text();
  let body: unknown = {};
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { raw: text };
  }
  if (!resp.ok) {
    const msg =
      (body as { error?: string }).error ?? `HTTP ${resp.status}`;
    throw new ApiError(resp.status, msg);
  }
  return body as T;
}

// ---- SSE over fetch (EventSource cannot set Authorization headers) ----

export interface BusEventPayload {
  event_id: string;
  event_type: string;
  ts: string;
  actor: string;
  investigation_id?: string | null;
  trace_id?: string | null;
  payload: Record<string, unknown>;
}

export function streamEvents(
  onEvent: (ev: BusEventPayload) => void,
  onError?: (e: unknown) => void,
): () => void {
  const ctrl = new AbortController();
  const key = getKey();
  (async () => {
    try {
      const resp = await fetch("/api/v1/events", {
        headers: key ? { Authorization: `Bearer ${key}` } : {},
        signal: ctrl.signal,
      });
      if (!resp.ok || !resp.body) throw new ApiError(resp.status, "sse failed");
      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        const frames = buf.split("\n\n");
        buf = frames.pop() ?? "";
        for (const frame of frames) {
          const dataLine = frame.split("\n").find((l) => l.startsWith("data:"));
          if (!dataLine) continue;
          const data = dataLine.slice(5).trim();
          if (!data || data === "keepalive" || !data.startsWith("{")) continue;
          try {
            onEvent(JSON.parse(data) as BusEventPayload);
          } catch {
            /* ignore malformed frame */
          }
        }
      }
    } catch (e) {
      if (!ctrl.signal.aborted && onError) onError(e);
    }
  })();
  return () => ctrl.abort();
}
