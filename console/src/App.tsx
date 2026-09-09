import { useEffect, useState } from "react";
import { NavLink, Route, Routes } from "react-router-dom";
import { getKey, setKey } from "./api";
import Overview from "./pages/Overview";
import Radar from "./pages/Radar";
import Investigations from "./pages/Investigations";
import InvestigationWorkspace from "./pages/InvestigationWorkspace";
import SearchPage from "./pages/SearchPage";
import Evidence from "./pages/Evidence";
import DocumentDetail from "./pages/DocumentDetail";
import EntityDetail from "./pages/EntityDetail";
import Alerts from "./pages/Alerts";
import Agents from "./pages/Agents";
import Audit from "./pages/Audit";
import System from "./pages/System";

const NAV = [
  ["/", "Overview"],
  ["/radar", "Radar"],
  ["/investigations", "Investigations"],
  ["/search", "Search"],
  ["/evidence", "Evidence"],
  ["/alerts", "Alerts"],
  ["/agents", "Agents"],
  ["/audit", "Audit"],
  ["/system", "System"],
] as const;

function KeyGate({ onDone }: { onDone: () => void }) {
  const [value, setValue] = useState("");
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
        <h1 className="mb-1 text-sm font-semibold">IntelHub Console</h1>
        <p className="mb-4 text-xs text-dim">
          Paste the console API key (agent identity <span className="mono">console</span>). Stored only in this
          browser's localStorage.
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
          Connect
        </button>
      </form>
    </div>
  );
}

export default function App() {
  const [authed, setAuthed] = useState(!!getKey());
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
          <div className="text-sm font-bold tracking-wide">IntelHub</div>
          <div className="text-[10px] uppercase tracking-widest text-dim">Unified Console</div>
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
              {label}
            </NavLink>
          ))}
        </div>
        <div className="border-t border-edge px-3 py-2 text-[10px] text-dim mono">
          hub 10.10.10.41:8800
        </div>
      </nav>
      <main className="flex-1 overflow-y-auto">
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/radar" element={<Radar />} />
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
