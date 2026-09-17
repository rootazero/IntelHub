// Contract guard suite — the upgrade tripwire that turns vendor syncs from
// blind diffs into governed upgrades (P2 T12).
//
//   c1  contract pinning: parse the vendored engine's SOURCE_METHODS and
//       structural code anchors, then assert IntelHub's adapters still satisfy
//       them. Anchors are identifier-level CODE (property accesses,
//       declarations) — never line numbers or comment text — so they survive
//       comment drift and reformatting but fire the moment an upstream sync
//       changes a load-bearing contract.
//   c2  engine behavior contracts: the degraded semantics the engine's
//       ingestion relies on (503 envelopes, empty groups, contract-misuse
//       TypeError), driven entirely by a mocked apiFetch — zero network.
//   c3  vendor boundary: host code must not reference the engine's
//       local_data internals outside the exceptions declared in
//       console/gev-engine/UPSTREAM.json (the vite.config.ts externalize
//       plugin is a T8-reviewed intentional exception), and the engine alias
//       channel must stay in sync across vite / vitest / tsconfig (Ruling 6).
//
// Explicit vitest imports (repo convention, cf. gev-adapters/__tests__/stubs.test.ts):
// vitest.config.ts sets globals:true for the runtime, but `tsc -b` does not
// load vitest/globals types, so bare `test`/`expect` break `npm run build`.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test, vi } from "vitest";
import type { ApiFetch } from "../../gev-adapters/http";
import { createIntelHubLayerSources } from "../../gev-adapters";

const here = dirname(fileURLToPath(import.meta.url));
const consoleRoot = join(here, "..", "..", "..");
const vendor = (rel: string) => join(consoleRoot, "gev-engine", rel);
const readVendor = (rel: string) => readFileSync(vendor(rel), "utf8");
const readConsole = (rel: string) => readFileSync(join(consoleRoot, rel), "utf8");

/** Parse SOURCE_METHODS out of the vendored constructCatalog.js (code structure). */
function parseSourceMethods(src: string): Array<[string, string[]]> {
  const block =
    src.match(/SOURCE_METHODS\s*=\s*Object\.freeze\(\{([\s\S]*?)\}\)/)?.[1] ?? "";
  return [...block.matchAll(/(\w+):\s*\[([^\]]*)\]/g)].map(
    (m) =>
      [
        m[1],
        m[2].match(/'(\w+)'/g)?.map((s) => s.slice(1, -1)) ?? [],
      ] as [string, string[]],
  );
}

// ── c1: contract pinning ───────────────────────────────────────────────────

describe("c1: contract pinning (upstream churn fuse)", () => {
  test("adapters cover upstream SOURCE_METHODS verbatim, in both directions", () => {
    const entries = parseSourceMethods(
      readVendor("src/app/constructCatalog.js"),
    );
    expect(entries.length).toBeGreaterThanOrEqual(14);
    const sources = createIntelHubLayerSources({
      apiFetch: async () => new Response("{}"),
    });
    for (const [layer, methods] of entries)
      for (const m of methods)
        expect(
          typeof (sources as Record<string, any>)[layer]?.[m],
          `${layer}.${m}`,
        ).toBe("function");

    // Reverse direction: no factory entry outside the pinned layer set.
    // `transit` is the sole documented extra — it is absent from
    // SOURCE_METHODS (the engine layer validates it itself,
    // layers/transit/index.js:34-41) but the catalog consumes
    // sources.transit, so the factory must register it (T3 I-2).
    const upstreamLayers = new Set(entries.map(([layer]) => layer));
    for (const key of Object.keys(sources)) {
      if (key === "transit") continue;
      expect(upstreamLayers.has(key), `factory entry outside SOURCE_METHODS: ${key}`).toBe(
        true,
      );
    }
  });

  test("flights/military ingestion still reads the pinned snapshot keys", () => {
    // Keys the engine's ingestion destructures off our envelopes
    // (flights/ingestion.js:35-53, military/ingestion.js:34-52, verified T6).
    // Anchored on `snapshot.<key>` property accesses in vendor code. The two
    // layers deliberately differ — military keys freshness off observedAtMs +
    // stale and never reads ageMs — so the pin is per-file, not a shared set.
    const pins: Array<[string, string[]]> = [
      [
        "src/layers/flights/ingestion.js",
        [
          "status",
          "observedAtMs",
          "ageMs",
          "stale",
          "freshness",
          "source",
          "coverage",
        ],
      ],
      [
        "src/layers/military/ingestion.js",
        ["status", "observedAtMs", "stale", "freshness", "source", "reason"],
      ],
    ];
    for (const [rel, keys] of pins) {
      const src = readVendor(rel);
      for (const k of keys)
        expect(src, `${rel} reads snapshot.${k}`).toMatch(
          new RegExp(`snapshot\\.${k}\\b`),
        );
    }
  });

  test("catalog still validates sources and throws Invalid catalog source", () => {
    const src = readVendor("src/app/constructCatalog.js");
    // The exact typeof guard the catalog runs per SOURCE_METHODS entry.
    expect(src).toMatch(
      /typeof sources\?\.\[name\]\?\.\[method\] !== 'function'/,
    );
    expect(src).toMatch(/Invalid catalog source/);
  });

  test("satellites ingestion still resolves readGroup and maps non-ok to an empty group", () => {
    const src = readVendor("src/layers/satellites/ingestion.js");
    expect(src).toMatch(/source\.readGroup\(/);
    // Non-ok must degrade to `{ entries: [], ok: false }` — the adapter's
    // contract is "transport failures resolve, never throw".
    expect(src).toMatch(/!res\.ok\)\s*return\s*\{[^}]*entries:\s*\[\][^}]*ok:\s*false/);
  });

  test("earthquakes update still destructures the pinned row fields", () => {
    const src = readVendor("src/layers/earthquakes/index.js");
    const destructure = src.match(/for \(const \{([\s\S]*?)\} of rows\)/)?.[1] ?? "";
    expect(destructure).not.toBe("");
    for (const f of [
      "stableId",
      "usgsId",
      "lon",
      "lat",
      "depthKm",
      "mag",
      "place",
      "time",
    ])
      expect(destructure, `row field ${f}`).toMatch(new RegExp(`\\b${f}\\b`));
  });

  test("vessels ingestion still reads the pinned envelope keys", () => {
    const src = readVendor("src/layers/vessels/ingestion.js");
    for (const k of ["records", "observedAtMs", "freshness", "complete"])
      expect(src, `vessels reads snapshot.${k}`).toMatch(
        new RegExp(`snapshot\\.${k}\\b`),
      );
  });

  test("wave-1 snapshot status is a number across fresh/stale/degraded — never pinned to 200 (T6 M-3)", async () => {
    // Ruling T6 M-3: a stale upstream snapshot is INTENTIONALLY framed as
    // status 503 (aircraft-map.ts toEnvelope — "stale-200 responses get 503"),
    // so the contract under test is "status is an HTTP-ish number", never
    // "status === 200". This test deliberately exercises a fresh body and the
    // hub's degraded {stale:true} body and only asserts the numeric contract.
    const fresh = {
      ts: new Date(Date.now() - 30_000).toISOString(),
      aircraft: [],
    };
    for (const body of [fresh, { stale: true, aircraft: [] }]) {
      const apiFetch: ApiFetch = async () =>
        new Response(JSON.stringify(body), { status: 200 });
      const s = createIntelHubLayerSources({ apiFetch });
      const env = await (s as Record<string, any>).flights.getSnapshot();
      expect(typeof env.status, JSON.stringify(Object.keys(body))).toBe("number");
      expect(Number.isInteger(env.status)).toBe(true);
    }
    // The degraded hub body must produce 503 (the intentional deviation a
    // 200-only assertion would forbid) while staying a number.
    const degraded = createIntelHubLayerSources({
      apiFetch: async () =>
        new Response(JSON.stringify({ stale: true, aircraft: [] }), {
          status: 200,
        }),
    });
    const staleEnv = await (degraded as Record<string, any>).flights.getSnapshot();
    expect(staleEnv.status).toBe(503);
    expect(typeof staleEnv.status).toBe("number");
  });
});

// ── c2: engine behavior contracts (mock fetch, no network) ─────────────────

describe("c2: engine behavior contracts (mock fetch, zero network)", () => {
  const jsonFetch =
    (body: unknown, status = 200): ApiFetch =>
    async () =>
      new Response(JSON.stringify(body), { status });

  test("envelope stubs resolve degraded-by-contract: 503, empty records, unknown freshness", async () => {
    // What the engine reads as "degraded" (flights/ingestion.js:41: stale ||
    // freshness === "unknown") must be exactly what an idle stub produces.
    const s = createIntelHubLayerSources({ apiFetch: jsonFetch({}) });
    const env = await (s as Record<string, any>).vessels.getSnapshot();
    expect(env.records).toEqual([]);
    expect(env.status).toBe(503);
    expect(env.freshness).toBe("unknown");
    expect(env.observedAtMs).toBeNull();
    expect(env.ageMs).toBeNull();
    // FIRMS has its own payload shape: empty + stale, never fabricated fires.
    const firms = await (s as Record<string, any>).firms.getSnapshot();
    expect(firms).toMatchObject({ fires: [], stale: true });
  });

  test("satellites: transport failure resolves {ok:false,status} and never throws; unknown group is a contract-misuse TypeError", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: async () => new Response("upstream boom", { status: 500 }),
    });
    // The engine maps non-ok to an empty group (satellites/ingestion.js:34),
    // so the adapter resolving — not rejecting — on HTTP 500 is the contract.
    await expect(
      (s as Record<string, any>).satellites.readGroup("visual"),
    ).resolves.toEqual({ ok: false, status: 500, text: "" });
    // A group outside the served set is a programming error, not a transport
    // state: TypeError, per T5's source contract.
    await expect(
      (s as Record<string, any>).satellites.readGroup("not-a-real-group"),
    ).rejects.toBeInstanceOf(TypeError);
  });

  test("earthquakes: non-array body degrades to [] (never reaches `for...of` throw); non-ok rejects with the status", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "proxy html page" }),
    });
    await expect(
      (s as Record<string, any>).earthquakes.getSnapshot(),
    ).resolves.toEqual([]);
    const s502 = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "x" }, 502),
    });
    await expect(
      (s502 as Record<string, any>).earthquakes.getSnapshot(),
    ).rejects.toThrow("IntelHub earthquakes HTTP 502");
  });

  test("flights/military: hub degraded body resolves stale/503 with null epoch; non-ok rejects with the status", async () => {
    const s = createIntelHubLayerSources({
      apiFetch: jsonFetch({ stale: true, aircraft: [] }),
    });
    for (const layer of ["flights", "military"]) {
      const env = await (s as Record<string, any>)[layer].getSnapshot();
      expect(env.records).toEqual([]);
      expect(env.stale).toBe(true);
      // 503 on the hub's degraded body is intentional (T6 M-3) — the assertion
      // is the degraded semantics, not a universal 200.
      expect(env.status).toBe(503);
      expect(env.observedAtMs).toBeNull();
      expect(env.ageMs).toBeNull();
    }
    const s401 = createIntelHubLayerSources({
      apiFetch: jsonFetch({ error: "unauthorized" }, 401),
    });
    await expect(
      (s401 as Record<string, any>).flights.getSnapshot(),
    ).rejects.toThrow("HTTP 401");
    await expect(
      (s401 as Record<string, any>).military.getSnapshot(),
    ).rejects.toThrow("HTTP 401");
  });

  test("transit stub keeps the engine out of its unauthenticated fallback: 503 + {vehicles:[]} + empty history", async () => {
    // T3 I-2: without a sources.transit entry the engine falls back to its own
    // same-origin UNauthenticated /api/transit polling. The stub's 503 +
    // empty vehicles lands the layer in its ordinary "temporarily
    // unavailable" state instead.
    const s = createIntelHubLayerSources({ apiFetch: jsonFetch({}) });
    const snap = await (s as Record<string, any>).transit.requestSnapshot();
    expect(snap).toMatchObject({ ok: false, status: 503 });
    await expect(snap.json()).resolves.toEqual({ vehicles: [] });
    await expect(
      (s as Record<string, any>).transit.getHistory(),
    ).resolves.toEqual({ epochs: [] });
  });
});

// ── c3: vendor boundary (UPSTREAM.json exceptions are the single source) ───

describe("c3: vendor boundary (UPSTREAM.json exceptions)", () => {
  const upstreamMeta = JSON.parse(readVendor("UPSTREAM.json")) as {
    exceptions?: Array<string | { file: string; reason?: string }>;
  };
  const exceptionFiles = new Set(
    (upstreamMeta.exceptions ?? []).map((e) => (typeof e === "string" ? e : e.file)),
  );

  test("the vite.config.ts local_data externalize is a DECLARED exception, not an implicit one", () => {
    expect(
      exceptionFiles.has("console/vite.config.ts"),
      "vite.config.ts must be listed in UPSTREAM.json exceptions " +
        "(T8-reviewed intentional local_data externalize)",
    ).toBe(true);
    expect(existsSync(join(consoleRoot, "vite.config.ts"))).toBe(true);
  });

  test("no host file references the vendor's local_data outside declared exceptions", () => {
    const hits: string[] = [];
    const scan = (dir: string) => {
      for (const entry of readdirSync(dir)) {
        if (entry === "node_modules" || entry === "dist" || entry.startsWith("."))
          continue;
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
          scan(full);
          continue;
        }
        if (!/\.(ts|tsx|js|mts|cts|json)$/.test(entry)) continue;
        // The guard file itself is the scanner — its own literal patterns are
        // not host references to vendor internals.
        if (full === fileURLToPath(import.meta.url)) continue;
        if (/local_data/.test(readFileSync(full, "utf8")))
          hits.push(relative(consoleRoot, full));
      }
    };
    scan(join(consoleRoot, "src"));
    for (const cfg of ["vite.config.ts", "vitest.config.ts", "tsconfig.json"]) {
      const full = join(consoleRoot, cfg);
      if (existsSync(full) && /local_data/.test(readFileSync(full, "utf8")))
        hits.push(cfg);
    }
    expect(hits.length).toBeGreaterThan(0); // sanity: the exemption is exercised
    for (const hit of hits)
      expect(
        exceptionFiles.has(`console/${hit}`),
        `${hit} references local_data — declare it in UPSTREAM.json ` +
          "exceptions or remove the reference",
      ).toBe(true);
  });

  test("the externalize plugin itself is still wired in vite.config.ts", () => {
    const vite = readConsole("vite.config.ts");
    expect(vite).toContain("gev-local-data-external");
    // The resolveId pattern — externalizing the un-vendored local_data packs
    // is what keeps `npm run build` green without touching the vendor tree.
    expect(vite).toContain("local_data\\/[^/]+\\/[^/]+\\.json$");
    expect(vite).toContain("external: true");
  });

  test("engine alias channel stays in sync across vite / vitest / tsconfig (Ruling 6)", () => {
    // Ruling 6's stated cost — "三处配置需保持同步" — is enforced here:
    // vitest ignores vite.config.ts entirely when vitest.config.ts exists, so
    // a config that drops the alias breaks tests keyless/broken while the
    // build keeps working.
    const vite = readConsole("vite.config.ts");
    const vitest = readConsole("vitest.config.ts");
    const tsconfig = readConsole("tsconfig.json");
    expect(vite).toMatch(/alias:\s*\{[\s\S]*?"gev-engine"/);
    expect(vitest).toMatch(/alias:\s*\{[\s\S]*?"gev-engine"/);
    expect(tsconfig).toMatch(/"gev-engine\/\*"/);
  });

  test("every declared exception file exists on disk", () => {
    for (const file of exceptionFiles)
      expect(existsSync(join(consoleRoot, "..", file)), file).toBe(true);
  });
});
