// Investigations list + creation (§27: investigation is a first-class object).

import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { api } from "../api";
import { Empty, ErrorBox, Loading, Panel, TimeAgo } from "../ui";

interface Investigation {
  investigation_id: string;
  title: string;
  target?: string | null;
  question?: string | null;
  status: string;
  created_by: string;
  created_at: string;
}

export default function Investigations() {
  const [items, setItems] = useState<Investigation[]>([]);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState("");
  const [params] = useSearchParams();
  const [title, setTitle] = useState(params.get("prefill") ?? "");
  const [target, setTarget] = useState("");
  const [question, setQuestion] = useState("");
  const nav = useNavigate();

  const load = useCallback(() => {
    const qs = status ? `?status=${status}&limit=100` : "?limit=100";
    api<{ items: Investigation[] }>(`/api/v1/investigations${qs}`)
      .then((r) => setItems(r.items ?? []))
      .catch(setError)
      .finally(() => setLoading(false));
  }, [status]);
  useEffect(load, [load]);

  const create = async () => {
    if (!title.trim()) return;
    try {
      const r = await api<{ investigation_id: string }>("/api/v1/investigations", {
        method: "POST",
        body: JSON.stringify({ title: title.trim(), target: target || null, question: question || null }),
      });
      nav(`/investigations/${r.investigation_id}`);
    } catch (e) {
      setError(e instanceof Error ? e : new Error(String(e)));
    }
  };

  return (
    <div className="space-y-3 p-4">
      <Panel title="New Investigation">
        <div className="flex flex-wrap gap-1.5 text-xs">
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="title (required)"
            className="min-w-64 flex-1 rounded border border-edge bg-base px-2 py-1" />
          <input value={target} onChange={(e) => setTarget(e.target.value)} placeholder="target (person/org/domain/…)"
            className="w-56 rounded border border-edge bg-base px-2 py-1" />
          <input value={question} onChange={(e) => setQuestion(e.target.value)} placeholder="question"
            className="w-64 rounded border border-edge bg-base px-2 py-1" />
          <button onClick={create} className="rounded bg-accent/20 px-3 py-1 font-semibold text-accent hover:bg-accent/30">
            Create
          </button>
        </div>
      </Panel>

      <Panel
        title="Investigations"
        right={
          <select value={status} onChange={(e) => setStatus(e.target.value)} className="rounded border border-edge bg-base px-1.5 py-0.5 text-[11px]">
            <option value="">all</option><option value="open">open</option><option value="paused">paused</option><option value="closed">closed</option>
          </select>
        }
      >
        {error && <ErrorBox error={error} />}
        {loading ? <Loading /> : items.length === 0 ? <Empty label="no investigations" /> : (
          <table className="w-full text-xs">
            <thead><tr className="text-left text-[10px] uppercase text-dim">
              <th className="pb-1">Title</th><th className="pb-1">Target</th><th className="pb-1">Status</th>
              <th className="pb-1">Created by</th><th className="pb-1">Age</th>
            </tr></thead>
            <tbody>
              {items.map((i) => (
                <tr key={i.investigation_id} className="border-t border-edge/50">
                  <td className="py-1.5 pr-2">
                    <Link className="text-accent hover:underline" to={`/investigations/${i.investigation_id}`}>{i.title}</Link>
                  </td>
                  <td className="py-1.5 pr-2 mono text-dim">{i.target ?? "—"}</td>
                  <td className="py-1.5 pr-2">
                    <span className={`mono ${i.status === "open" ? "text-ok" : "text-dim"}`}>{i.status}</span>
                  </td>
                  <td className="py-1.5 pr-2 mono">{i.created_by}</td>
                  <td className="py-1.5"><TimeAgo ts={i.created_at} /></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>
    </div>
  );
}
