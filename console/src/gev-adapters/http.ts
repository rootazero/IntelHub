// HTTP transport for IntelHub's REST API, injected into the adapter factory.
// Kept separate from the stubs so wave-1 real implementations (T4-T6) share
// one auth-header path (REST auth: `Authorization: Bearer ihk_<hex>`).

export type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

/** Build an authenticated fetcher bound to the hub base URL + agent key. */
export const makeApiFetch =
  (base: string, token: string): ApiFetch =>
  (path, init) =>
    fetch(`${base}${path}`, {
      ...init,
      headers: {
        Authorization: `Bearer ${token}`,
        ...(init?.headers ?? {}),
      },
    });
