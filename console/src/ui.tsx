// Shared UI primitives — information-dense, signal-first (directive §75).
// Type distinction is a hard requirement (§29): FACT / OBSERVATION /
// AGENT CLAIM / HYPOTHESIS / ALERT / SYSTEM STATE must be visually distinct.

import type { ReactNode } from "react";
import { useEnum, useT } from "./i18n";

export function Panel(props: { title: ReactNode; right?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`rounded border border-edge bg-panel ${props.className ?? ""}`}>
      <header className="flex items-center justify-between border-b border-edge px-3 py-1.5">
        <h2 className="text-[11px] font-semibold uppercase tracking-wider text-dim">{props.title}</h2>
        {props.right}
      </header>
      <div className="p-3">{props.children}</div>
    </section>
  );
}

export function Stat(props: { label: string; value: ReactNode; tone?: "ok" | "warn" | "crit" | "dim" }) {
  const tone = props.tone ?? "dim";
  const color =
    tone === "ok" ? "text-ok" : tone === "warn" ? "text-warn" : tone === "crit" ? "text-crit" : "text-ink";
  return (
    <div className="rounded border border-edge bg-panel px-3 py-2">
      <div className="text-[10px] uppercase tracking-wider text-dim">{props.label}</div>
      <div className={`mt-0.5 text-lg font-semibold mono ${color}`}>{props.value}</div>
    </div>
  );
}

const SEV_TONE: Record<string, string> = {
  critical: "text-crit border-crit/40",
  warning: "text-warn border-warn/40",
  info: "text-accent border-accent/40",
};
export function SeverityBadge({ severity }: { severity: string }) {
  const en = useEnum();
  return (
    <span className={`inline-block rounded border px-1.5 py-0 text-[10px] font-semibold uppercase ${SEV_TONE[severity] ?? "text-dim border-edge"}`}>
      {en("severity", severity)}
    </span>
  );
}

const BUDGET_TONE: Record<string, string> = {
  GREEN: "text-ok border-ok/40",
  YELLOW: "text-warn border-warn/40",
  RED: "text-crit border-crit/40",
  KILL: "text-crit border-crit",
};
export function BudgetBadge({ state }: { state: string }) {
  const en = useEnum();
  return (
    <span className={`inline-block rounded border px-1.5 py-0 text-[10px] font-bold mono ${BUDGET_TONE[state] ?? "text-dim border-edge"}`}>
      {en("budget", state)}
    </span>
  );
}

// §29 epistemic type badge — never blur agent inference with fact.
const EPI_TONE: Record<string, string> = {
  FACT: "text-ok border-ok/50",
  OBSERVATION: "text-accent border-accent/50",
  "AGENT CLAIM": "text-warn border-warn/50",
  HYPOTHESIS: "text-purple-400 border-purple-400/50",
  ALERT: "text-crit border-crit/50",
  "SYSTEM STATE": "text-dim border-edge",
};
export function EpistemicBadge({ kind }: { kind: keyof typeof EPI_TONE | string }) {
  const en = useEnum();
  return (
    <span className={`inline-block rounded border px-1.5 py-0 text-[10px] font-bold tracking-wide ${EPI_TONE[kind] ?? "text-dim border-edge"}`}>
      {en("epistemic", kind)}
    </span>
  );
}

export function StatusDot({ up }: { up: boolean }) {
  return <span className={`inline-block h-2 w-2 rounded-full ${up ? "bg-ok" : "bg-crit"}`} />;
}

export function TimeAgo({ ts }: { ts: string }) {
  const d = new Date(ts);
  const s = Math.max(0, (Date.now() - d.getTime()) / 1000);
  const label =
    s < 60 ? `${Math.floor(s)}s` : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`;
  return (
    <span className="mono text-dim" title={d.toISOString()}>
      {label}
    </span>
  );
}

export function Loading() {
  const { t } = useT();
  return <div className="p-4 text-dim mono text-xs">{t("common.loading")}</div>;
}

export function ErrorBox({ error }: { error: unknown }) {
  const msg = error instanceof Error ? error.message : String(error);
  return <div className="rounded border border-crit/40 bg-crit/10 p-3 text-crit text-xs mono">{msg}</div>;
}

export function Empty({ label }: { label: string }) {
  return <div className="p-3 text-dim text-xs">{label}</div>;
}
