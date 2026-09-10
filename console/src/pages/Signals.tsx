// Signals (SP6B): monitor finance plane — watchlist management + latest
// time-series observations (macro FRED/EIA/Treasury, quotes, sentiment).

import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import { useT } from "../i18n";
import { Empty, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface WatchItem {
  symbol: string;
  asset_class: string;
  label: string;
  enabled: boolean;
}

interface SeriesPoint {
  series: string;
  observed_at: string;
  value: number;
  payload: Record<string, unknown>;
}

export default function Signals() {
  const { t } = useT();
  const [watch, setWatch] = useState<WatchItem[]>([]);
  const [series, setSeries] = useState<SeriesPoint[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [symbol, setSymbol] = useState("");
  const [assetClass, setAssetClass] = useState("us_stock");
  const [label, setLabel] = useState("");

  const load = useCallback(() => {
    Promise.all([
      api<{ watchlist: WatchItem[] }>("/api/v1/signals/watchlist"),
      api<{ series: SeriesPoint[] }>("/api/v1/signals/latest"),
    ])
      .then(([w, s]) => {
        setWatch(w.watchlist ?? []);
        setSeries((s.series ?? []).sort((a, b) => a.series.localeCompare(b.series)));
      })
      .catch(setError)
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);

  const add = async () => {
    if (!symbol.trim()) return;
    try {
      await api("/api/v1/signals/watchlist", {
        method: "POST",
        body: JSON.stringify({ symbol: symbol.trim(), asset_class: assetClass, label: label.trim() }),
      });
      setSymbol("");
      setLabel("");
      load();
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  const setEnabled = async (sym: string, enabled: boolean) => {
    try {
      await api("/api/v1/signals/watchlist", {
        method: "POST",
        body: JSON.stringify({ symbol: sym, enabled }),
      });
      load();
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  const remove = async (sym: string) => {
    try {
      await api(`/api/v1/signals/watchlist/${encodeURIComponent(sym)}`, { method: "DELETE" });
      load();
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  const fmt = (v: number) =>
    Math.abs(v) >= 1e12
      ? `${(v / 1e12).toFixed(2)}T`
      : Math.abs(v) >= 1e9
        ? `${(v / 1e9).toFixed(2)}B`
        : v.toLocaleString(undefined, { maximumFractionDigits: 4 });

  return (
    <div className="p-4">
      {error && <ErrorBox error={error} />}
      <Panel title={t("signals.watchlist")}>
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <input
            className="w-28 rounded border border-edge bg-panel2 px-2 py-1 text-sm uppercase"
            placeholder={t("signals.symbolPh")}
            value={symbol}
            onChange={(e) => setSymbol(e.target.value.toUpperCase())}
          />
          <select
            className="rounded border border-edge bg-panel2 px-2 py-1 text-sm"
            value={assetClass}
            onChange={(e) => setAssetClass(e.target.value)}
          >
            <option value="us_stock">us_stock</option>
            <option value="etf">etf</option>
            <option value="index">index</option>
            <option value="crypto">crypto</option>
          </select>
          <input
            className="w-44 rounded border border-edge bg-panel2 px-2 py-1 text-sm"
            placeholder={t("signals.labelPh")}
            value={label}
            onChange={(e) => setLabel(e.target.value)}
          />
          <button
            className="rounded bg-accent px-3 py-1 text-sm font-medium text-black disabled:opacity-40"
            onClick={add}
            disabled={!symbol.trim()}
          >
            {t("signals.add")}
          </button>
        </div>
        {loading ? (
          <Loading />
        ) : watch.length === 0 ? (
          <Empty label={t("signals.emptyWatch")} />
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-edge text-left text-dim">
                <th className="py-1">{t("signals.colSymbol")}</th>
                <th>{t("signals.colClass")}</th>
                <th>{t("signals.colLabel")}</th>
                <th>{t("signals.colEnabled")}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {watch.map((w) => (
                <tr key={w.symbol} className="border-b border-edge/50">
                  <td className="py-1 font-mono font-medium">{w.symbol}</td>
                  <td className="text-dim">{w.asset_class}</td>
                  <td>{w.label}</td>
                  <td>
                    <button
                      className={`rounded px-2 py-0.5 text-xs ${w.enabled ? "bg-ok/20 text-ok" : "bg-edge text-dim"}`}
                      onClick={() => setEnabled(w.symbol, !w.enabled)}
                    >
                      {w.enabled ? t("signals.on") : t("signals.off")}
                    </button>
                  </td>
                  <td className="text-right">
                    <button className="text-xs text-danger hover:underline" onClick={() => remove(w.symbol)}>
                      {t("signals.remove")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>

      <Panel title={t("signals.latest")}>
        {loading ? (
          <Loading />
        ) : series.length === 0 ? (
          <Empty label={t("signals.emptySeries")} />
        ) : (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-edge text-left text-dim">
                <th className="py-1">{t("signals.colSeries")}</th>
                <th className="text-right">{t("signals.colValue")}</th>
                <th className="text-right">{t("signals.colTime")}</th>
              </tr>
            </thead>
            <tbody>
              {series.map((s) => (
                <tr key={s.series} className="border-b border-edge/50">
                  <td className="py-1 font-mono">{s.series}</td>
                  <td className="text-right font-medium">{fmt(s.value)}</td>
                  <td className="text-right text-dim">
                    <TimeAgo ts={s.observed_at} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>
    </div>
  );
}
