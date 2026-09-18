import { beforeEach, describe, expect, test, vi } from "vitest";

// Shared harness state. vi.hoisted keeps it available to the hoisted
// vi.mock factories without TDZ surprises.
const hoisted = vi.hoisted(() => ({
  instances: [] as any[],
  flyCalls: [] as any[],
}));

// Fake vendor LocationSearch replicating the REAL contract
// (gev-engine/src/ui/locationSearch.js):
//   * constructor requires begin/isCurrent/beforeFly + search (run() calls all
//     three unconditionally) and an input with classList + blur()
//   * getState() returns a state RECORD with a `status` string
//   * subscribe(listener, {emitCurrent}) delivers {state, change, revision,
//     initial} notifications
// The constructor assertions are the lenient-mock guard: a fake that accepts a
// bare {input, search} would let an adapter that throws on the real engine pass.
vi.mock("gev-engine/src/ui/locationSearch.js", () => ({
  LocationSearch: class {
    state = {
      status: "idle",
      searching: false,
      query: "",
      generation: null as unknown,
      requestId: 0,
      destination: null as unknown,
      error: null as unknown,
    };
    listeners = new Set<{ listener: Function }>();
    revision = 0;
    controller: AbortController | null = null;
    generation = 0;
    destroyed = false;
    opts: any;
    // Assigned from opts by Object.assign (the real class does the same).
    input!: any;
    begin!: () => unknown;
    isCurrent!: (generation: unknown) => boolean;
    beforeFly!: (generation: unknown) => boolean;
    search!: (query: string, options: any) => Promise<any>;

    constructor(opts: any) {
      for (const key of ["begin", "isCurrent", "beforeFly", "search"]) {
        if (typeof opts?.[key] !== "function") {
          throw new TypeError(`LocationSearch: ${key} must be a function`);
        }
      }
      if (!opts.input?.classList || typeof opts.input.blur !== "function") {
        throw new TypeError(
          "LocationSearch: input must have classList and blur()",
        );
      }
      Object.assign(this, opts);
      this.opts = opts;
      hoisted.instances.push(this);
    }

    getState() {
      return this.state;
    }

    subscribe(
      listener: Function,
      { emitCurrent = true }: { emitCurrent?: boolean } = {},
    ) {
      if (typeof listener !== "function") {
        throw new TypeError("Expected a state listener");
      }
      if (this.destroyed) return () => {};
      const entry = { listener };
      this.listeners.add(entry);
      if (emitCurrent) {
        listener({
          state: this.state,
          change: null,
          revision: this.revision,
          initial: true,
        });
      }
      return () => this.listeners.delete(entry);
    }

    publish(change: unknown) {
      if (this.destroyed) return false;
      const notification = {
        state: this.state,
        change,
        revision: ++this.revision,
        initial: false,
      };
      for (const entry of [...this.listeners]) entry.listener(notification);
      return true;
    }

    async run(query: string) {
      query = String(query || "").trim();
      if (!query || this.destroyed) return;
      const authority = this.begin();
      if (authority === false) {
        this.input.classList.remove("searching");
        this.input.blur();
        return;
      }
      this.controller?.abort();
      const controller = new AbortController();
      this.controller = controller;
      const generation = ++this.generation;
      const current = () =>
        !this.destroyed &&
        generation === this.generation &&
        this.isCurrent(authority);
      const change = (type: string) => ({
        type,
        generation: authority,
        requestId: generation,
        query,
      });
      this.state = {
        ...this.state,
        status: "searching",
        searching: true,
        query,
        generation: authority,
        requestId: generation,
        destination: null,
        error: null,
      };
      try {
        if (!current() || controller.signal.aborted) return;
        this.input.classList.add("searching");
        this.publish(change("started"));
        if (!current() || controller.signal.aborted) return;
        const destination = await this.search(query, {
          signal: controller.signal,
          beforeFly: () => current() && this.beforeFly(authority),
        });
        if (!current() || controller.signal.aborted) return;
        if (destination?.cancelled) return;
        if (destination) {
          this.state = {
            ...this.state,
            status: "found",
            destination: { ...destination },
          };
          this.publish(change("found"));
        } else {
          this.state = { ...this.state, status: "missing" };
          this.publish(change("missing"));
        }
      } catch (error: any) {
        if (controller.signal.aborted || !current()) return;
        this.state = {
          ...this.state,
          status: "failed",
          error: { message: String(error?.message || error) },
        };
        this.publish(change("failed"));
      } finally {
        if (this.controller === controller) this.controller = null;
        if (!this.destroyed) {
          if (generation === this.generation) {
            this.state = {
              ...this.state,
              searching: false,
              status:
                this.state.status === "searching"
                  ? "cancelled"
                  : this.state.status,
            };
          }
          this.publish(change("settled"));
        }
      }
    }

    destroy() {
      if (this.destroyed) return;
      this.destroyed = true;
      this.generation++;
      this.controller?.abort();
      this.controller = null;
      this.state = { ...this.state, status: "disposed", searching: false };
      this.listeners.clear();
    }
  },
}));

// searchAndFlyTo fake: records the options so the test can assert the
// applicationServices bypass (features + recoverNearView overrides) and
// forwards to the adapter's placeSearch, like the real function does.
vi.mock("gev-engine/src/locations.js", () => ({
  searchAndFlyTo: vi.fn(async (_viewer: any, query: string, options: any) => {
    hoisted.flyCalls.push({ query, options });
    const outcome = await options.placeSearch.geocode(query, {
      signal: options.signal,
    });
    if (!outcome.place) return null;
    return {
      label: outcome.place.label,
      navigationMode: "city-overview",
      rangeM: 50000,
    };
  }),
}));

import { mountLocationSearch } from "../location-search";
import type { ApiFetch } from "../../gev-adapters/http";

function fakeInput() {
  return {
    classList: { add: vi.fn(), remove: vi.fn() },
    blur: vi.fn(),
  } as unknown as HTMLInputElement;
}

function apiFetchReturning(body: unknown, ok = true): ApiFetch {
  return vi.fn(async () => ({
    ok,
    json: async () => body,
  })) as unknown as ApiFetch;
}

beforeEach(() => {
  hoisted.instances.length = 0;
  hoisted.flyCalls.length = 0;
});

describe("mountLocationSearch", () => {
  test("found: hub result flies and state ends at found", async () => {
    const fetch = apiFetchReturning({
      results: [
        {
          lat: 48.85,
          lng: 2.35,
          name: "Paris",
          label: "Paris, France",
          types: ["locality"],
          viewport: null,
        },
      ],
    });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    const states: string[] = [];
    h.subscribe((s) => states.push(s));
    await h.run("Paris");
    expect(h.getState()).toBe("found");
    expect(states).toEqual(["searching", "found"]);
    expect((fetch as any).mock.calls[0][0]).toBe("/api/v1/gev/geocode?q=Paris");
    // applicationServices bypass: explicit overrides present
    const opts = hoisted.flyCalls[0].options;
    expect(typeof opts.features).toBeDefined();
    // ...and they are OUR disabled feature source, not the engine singleton.
    expect(typeof opts.features.getFocusFootprints).toBe("function");
    expect(await opts.features.getFocusFootprints({ lat: 1, lng: 1 }, {})).toBeNull();
    expect(typeof opts.recoverNearView).toBe("function");
    expect(await opts.recoverNearView()).toBeNull();
    h.destroy();
  });

  test("missing: empty results → state missing, no fly", async () => {
    const fetch = apiFetchReturning({ results: [] });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("zzz-no-such-place");
    expect(h.getState()).toBe("missing");
    h.destroy();
  });

  test("failed: hub 5xx → state failed", async () => {
    const fetch = apiFetchReturning({ error: "geocode upstream failed" }, false);
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("paris");
    expect(h.getState()).toBe("failed");
    h.destroy();
  });

  test("query is URL-encoded", async () => {
    const fetch = apiFetchReturning({ results: [] });
    const h = mountLocationSearch({} as any, fakeInput(), fetch);
    await h.run("São Paulo & beyond");
    expect((fetch as any).mock.calls[0][0]).toBe(
      "/api/v1/gev/geocode?q=S%C3%A3o%20Paulo%20%26%20beyond",
    );
    h.destroy();
  });

  test("engine constructor contract: begin/isCurrent/beforeFly supplied and honoured", () => {
    const h = mountLocationSearch(
      {} as any,
      fakeInput(),
      apiFetchReturning({ results: [] }),
    );
    const engineOpts = hoisted.instances[0].opts;
    for (const key of ["begin", "isCurrent", "beforeFly", "search"]) {
      expect(typeof engineOpts[key], key).toBe("function");
    }
    // A new search takes a fresh generation and supersedes the previous one.
    const first = engineOpts.begin();
    expect(engineOpts.isCurrent(first)).toBe(true);
    expect(engineOpts.beforeFly(first)).toBe(true);
    const second = engineOpts.begin();
    expect(engineOpts.isCurrent(first)).toBe(false);
    expect(engineOpts.isCurrent(second)).toBe(true);
    h.destroy();
  });

  test("destroy is idempotent and stops notifications", async () => {
    const h = mountLocationSearch(
      {} as any,
      fakeInput(),
      apiFetchReturning({ results: [] }),
    );
    const states: string[] = [];
    h.subscribe((s) => states.push(s));
    h.destroy();
    h.destroy();
    await h.run("paris");
    expect(states).toEqual([]);
  });
});
