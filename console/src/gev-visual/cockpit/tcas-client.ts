// GEV P20 TCAS — REST client adapter for `/api/v1/flights/near`.
//
// Per §6.3 spec (D-TCAS-1=A): pulls nearby aircraft from hub-core
// every 1 second when TCAS is enabled. Returns the structured JSON
// envelope from the server (which already does the haversine filter
// and threat classification) and exposes it to the HUD via a
// subscribe-style callback.
//
// The mount/destroy pattern matches the existing P15-P19 cockpit
// adapters (chase-cam, instruments-mount, replay-recorder). All
// HTTP fetches use the same bearer token as the rest of the console.

const DEFAULT_POLL_MS = 1000;
const DEFAULT_RADIUS_NM = 5;

export interface TcasTarget {
  hex: string;
  flight: string | null;
  lat: number;
  lon: number;
  alt_m: number;
  gs: number | null;
  track: number | null;
  squawk: string | null;
  mil: boolean | null;
  distance_nm: number;
  bearing_deg: number;
  closure_kts: number;
  threat: "warning" | "caution" | "monitor" | "none";
  age_s: number;
}

export interface TcasSnapshot {
  ts: string;
  query: { lat: number; lng: number; radius_nm: number };
  count: number;
  aircraft: TcasTarget[];
}

export interface MountCockpitTcasDeps {
  /** Bearer token (same as the rest of the console). */
  authToken: string;
  /** Hub base URL (e.g. http://10.10.10.41:8800). */
  baseUrl: string;
  /** Polling interval in ms (default 1000). */
  pollMs?: number;
  /** Search radius in nm (default 5; matches §6.3 D-TCAS-2=A). */
  radiusNm?: number;
}

export interface TcasClientHandle {
  start(
    onSnapshot: (snap: TcasSnapshot) => void,
    getAgentLatLng: () => { lat: number; lng: number } | null,
  ): void;
  stop(): void;
  destroy(): void;
}

async function fetchNear(
  baseUrl: string,
  authToken: string,
  lat: number,
  lng: number,
  radiusNm: number,
): Promise<TcasSnapshot | null> {
  try {
    const url =
      `${baseUrl}/api/v1/flights/near?lat=${lat}&lng=${lng}&radius_nm=${radiusNm}`;
    const res = await fetch(url, {
      headers: { Authorization: `Bearer ${authToken}` },
    });
    if (!res.ok) return null;
    const blob = (await res.json()) as TcasSnapshot;
    return blob;
  } catch {
    return null;
  }
}

export function mountCockpitTcas(
  deps: MountCockpitTcasDeps,
): TcasClientHandle {
  const pollMs = deps.pollMs ?? DEFAULT_POLL_MS;
  const radiusNm = deps.radiusNm ?? DEFAULT_RADIUS_NM;
  let timer: ReturnType<typeof setInterval> | null = null;
  let onSnap: ((s: TcasSnapshot) => void) | null = null;
  let getPos: (() => { lat: number; lng: number } | null) | null = null;
  let destroyed = false;

  async function tick() {
    if (destroyed || !onSnap || !getPos) return;
    const pos = getPos();
    if (!pos) return;
    const snap = await fetchNear(
      deps.baseUrl,
      deps.authToken,
      pos.lat,
      pos.lng,
      radiusNm,
    );
    if (destroyed || !onSnap) return;
    if (snap) onSnap(snap);
  }

  return {
    start(onSnapshot, getAgentLatLng) {
      if (destroyed) return;
      onSnap = onSnapshot;
      getPos = getAgentLatLng;
      if (timer !== null) return;
      // First fetch fires immediately so toggle-on has no 1s blank flash.
      tick();
      timer = setInterval(tick, pollMs);
    },
    stop() {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      onSnap = null;
      getPos = null;
    },
    destroy() {
      destroyed = true;
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      onSnap = null;
      getPos = null;
    },
  };
}