import { useEffect, useRef, useState } from "react";
import { NavLink, Route, Routes } from "react-router-dom";
import { getKey, setKey } from "./api";
import { I18nProvider, LANGS, useT } from "./i18n";
import Overview from "./pages/Overview";
import Monitor from "./pages/Monitor";
import Radar from "./pages/Radar";
import Investigations from "./pages/Investigations";
import InvestigationWorkspace from "./pages/InvestigationWorkspace";
import SearchPage from "./pages/SearchPage";
import Evidence from "./pages/Evidence";
import DocumentDetail from "./pages/DocumentDetail";
import EntityDetail from "./pages/EntityDetail";
import Alerts from "./pages/Alerts";
import Signals from "./pages/Signals";
import Agents from "./pages/Agents";
import Audit from "./pages/Audit";
import System from "./pages/System";

const NAV = [
  ["/", "nav.monitor"],
  ["/overview", "nav.overview"],
  ["/radar", "nav.radar"],
  ["/signals", "nav.signals"],
  ["/investigations", "nav.investigations"],
  ["/search", "nav.search"],
  ["/evidence", "nav.evidence"],
  ["/alerts", "nav.alerts"],
  ["/agents", "nav.agents"],
  ["/audit", "nav.audit"],
  ["/system", "nav.system"],
] as const;

function LangSwitch() {
  const { lang, setLang } = useT();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const current = LANGS.find((l) => l.code === lang) ?? LANGS[0];
  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="rounded border border-edge px-1.5 py-0.5 text-[10px] font-semibold text-dim hover:border-accent hover:text-accent"
        title="Language / 语言"
      >
        {current.short} ▴
      </button>
      {open && (
        <div className="absolute bottom-full right-0 mb-1 min-w-24 overflow-hidden rounded border border-edge bg-panel shadow-lg">
          {LANGS.map((l) => (
            <button
              key={l.code}
              onClick={() => {
                setLang(l.code);
                setOpen(false);
              }}
              className={`block w-full px-2.5 py-1 text-left text-[11px] ${
                l.code === lang ? "bg-accent/15 text-accent font-semibold" : "text-dim hover:bg-edge hover:text-ink"
              }`}
            >
              {l.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function KeyGate({ onDone }: { onDone: () => void }) {
  const [value, setValue] = useState("");
  const { t } = useT();
  return (
    <div className="flex h-full items-center justify-center">
      <form
        className="w-96 rounded border border-edge bg-panel p-6"
        onSubmit={(e) => {
          e.preventDefault();
          if (value.trim()) {
            setKey(value);
            onDone();
          }
        }}
      >
        <h1 className="mb-1 text-sm font-semibold">{t("brand.title")} Console</h1>
        <p className="mb-4 text-xs text-dim">
          {t("gate.intro_pre")} <span className="mono">console</span>{t("gate.intro_post")}
        </p>
        <input
          autoFocus
          type="password"
          className="mb-3 w-full rounded border border-edge bg-base px-2 py-1.5 mono text-xs outline-none focus:border-accent"
          placeholder="ihk_…"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
        <button className="w-full rounded bg-accent/20 px-3 py-1.5 text-xs font-semibold text-accent hover:bg-accent/30">
          {t("gate.connect")}
        </button>
      </form>
    </div>
  );
}

export default function App() {
  return (
    <I18nProvider>
      <Shell />
    </I18nProvider>
  );
}

function Shell() {
  const [authed, setAuthed] = useState(!!getKey());
  const { t } = useT();
  useEffect(() => {
    const on401 = () => setAuthed(false);
    window.addEventListener("intelhub:unauthorized", on401);
    return () => window.removeEventListener("intelhub:unauthorized", on401);
  }, []);

  if (!authed) return <KeyGate onDone={() => setAuthed(true)} />;

  return (
    <div className="flex h-full">
      <nav className="flex w-44 shrink-0 flex-col border-r border-edge bg-panel">
        <div className="border-b border-edge px-3 py-3">
          <div className="text-sm font-bold tracking-wide">{t("brand.title")}</div>
          <div className="text-[10px] uppercase tracking-widest text-dim">{t("brand.subtitle")}</div>
        </div>
        <div className="flex-1 overflow-y-auto py-2">
          {NAV.map(([to, label]) => (
            <NavLink
              key={to}
              to={to}
              end={to === "/"}
              className={({ isActive }) =>
                `block px-3 py-1.5 text-xs ${isActive ? "bg-accent/10 text-accent border-r-2 border-accent" : "text-dim hover:text-ink"}`
              }
            >
              {t(label)}
            </NavLink>
          ))}
        </div>
        <div className="flex items-center justify-between border-t border-edge px-3 py-2 text-[10px] text-dim mono">
          <span>hub 10.10.10.41:8800</span>
          <LangSwitch />
        </div>
      </nav>
      <main className="flex-1 overflow-y-auto">
        <Routes>
          <Route path="/" element={<Monitor />} />
          <Route path="/overview" element={<Overview />} />
          <Route path="/radar" element={<Radar />} />
          <Route path="/signals" element={<Signals />} />
          <Route path="/investigations" element={<Investigations />} />
          <Route path="/investigations/:id" element={<InvestigationWorkspace />} />
          <Route path="/search" element={<SearchPage />} />
          <Route path="/evidence" element={<Evidence />} />
          <Route path="/evidence/:id" element={<DocumentDetail />} />
          <Route path="/entities/:id" element={<EntityDetail />} />
          <Route path="/alerts" element={<Alerts />} />
          <Route path="/agents" element={<Agents />} />
          <Route path="/audit" element={<Audit />} />
          <Route path="/system" element={<System />} />
        </Routes>
      </main>
    </div>
  );
}
