#!/usr/bin/env python3
"""SP2A MCP acceptance driver — stdlib only. Exercises the full Streamable
HTTP handshake plus the key tool flows against intelhub-core."""
import json, sys, urllib.request, urllib.error

BASE = "http://10.10.10.41:8800"
KEY = sys.argv[1]
PASS, FAIL = [], []

def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    print(("PASS " if cond else "FAIL ") + name + (f"  | {detail}" if detail else ""))

def rpc(body, session=None, key=None, timeout=180):
    req = urllib.request.Request(BASE + "/mcp", data=json.dumps(body).encode(), method="POST")
    req.add_header("Authorization", "Bearer " + (key or KEY))
    req.add_header("Content-Type", "application/json")
    req.add_header("Accept", "application/json, text/event-stream")
    if session:
        req.add_header("Mcp-Session-Id", session)
    try:
        resp = urllib.request.urlopen(req, timeout=timeout)
        status = resp.status
        headers = resp.headers
        raw = resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, {"_http_error": e.read().decode()[:400]}
    ct = headers.get("Content-Type", "")
    if "text/event-stream" in ct:
        for line in raw.splitlines():
            if line.startswith("data:") and line[5:].strip():
                raw = line[5:].strip()
                break
    try:
        parsed = json.loads(raw)
    except Exception:
        parsed = {"_raw": raw[:300]}
    return status, headers, parsed

def tool(session, name, arguments, rid):
    return rpc({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                "params": {"name": name, "arguments": arguments}}, session)

def tool_json(resp):
    """Extract the JSON payload text from a tools/call result."""
    content = resp.get("result", {}).get("content", [])
    if content and content[0].get("type") == "text":
        try:
            return json.loads(content[0]["text"])
        except Exception:
            return {"_text": content[0]["text"][:400]}
    return resp

# --- 1. bad key rejected ---
status, _, _ = rpc({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}, key="ihk_wrong")
check("bad key → 401", status == 401, f"got {status}")

# --- 2. initialize ---
status, headers, init = rpc({
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "acceptance", "version": "0.1"}}})
sid = headers.get("Mcp-Session-Id") or headers.get("mcp-session-id")
check("initialize ok", status == 200 and "result" in init, f"status={status} server={init.get('result',{}).get('serverInfo',{}).get('name')}")
check("session id issued", bool(sid))

# --- notifications/initialized ---
status, _, _ = rpc({"jsonrpc": "2.0", "method": "notifications/initialized"}, sid)
check("initialized notification accepted", status in (200, 202), f"got {status}")

# --- 3. tools/list ---
status, _, tools = rpc({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}, sid)
names = [t["name"] for t in tools.get("result", {}).get("tools", [])]
expected = {"search_web","crawl_url","fetch_document","get_evidence","get_document","keyword_search",
            "hybrid_search","semantic_search","query_entity","query_relationship","find_path",
            "create_investigation","update_investigation","create_finding","list_investigations",
            "get_task_status","get_system_health"}
sp6b = {"signal_query","financials_fetch","watchlist_manage"}
check("tools/list = 28 tools (SP2B+SP6B)", expected.issubset(set(names)) and sp6b.issubset(set(names)) and len(names) == 28, f"missing: {(expected | sp6b) - set(names)} count={len(names)}")

# --- 4. search_web ---
_, _, r = tool(sid, "search_web", {"query": "open source intelligence", "limit": 3}, 3)
d = tool_json(r)
check("search_web returns results", d.get("count", 0) > 0, f"count={d.get('count')}")

# --- 5. crawl_url + dedupe ---
_, _, r1 = tool(sid, "crawl_url", {"url": "https://example.com/"}, 4)
c1 = tool_json(r1)
doc_id = c1.get("document_id")
check("crawl_url ingests document", bool(doc_id), f"doc={doc_id} dup={c1.get('duplicate')}")
_, _, r2 = tool(sid, "crawl_url", {"url": "https://example.com/"}, 5)
c2 = tool_json(r2)
check("re-crawl dedupe hit", c2.get("duplicate") is True, f"dup={c2.get('duplicate')}")

# --- 6. investigation + evidence-bound finding ---
_, _, ri = tool(sid, "create_investigation", {"title": "Acceptance Probe", "question": "does SP2A work?", "target": "intelhub"}, 6)
inv = tool_json(ri)
inv_id = inv.get("investigation_id")
check("create_investigation", bool(inv_id), f"inv={inv_id}")

_, _, rf = tool(sid, "create_finding", {
    "investigation_id": inv_id, "title": "Hub can crawl",
    "claim_text": "The hub successfully crawled and stored example.com",
    "evidence": [{"document_id": doc_id, "relation": "supports"}]}, 7)
f = tool_json(rf)
check("create_finding with evidence", bool(f.get("finding_id")), f"finding={f.get('finding_id')}")

# --- 6b. finding without evidence must be rejected ---
_, _, rf2 = tool(sid, "create_finding", {
    "investigation_id": inv_id, "title": "no evidence", "claim_text": "x", "evidence": []}, 8)
check("finding w/o evidence rejected", "error" in rf2 or rf2.get("result", {}).get("isError") is True)

# --- 7. keyword search finds the crawled doc ---
_, _, rk = tool(sid, "keyword_search", {"query": "example", "limit": 5}, 9)
k = tool_json(rk)
check("keyword_search hits stored doc", k.get("count", 0) >= 1, f"count={k.get('count')}")

# --- 8. semantic_search is labeled fallback ---
_, _, rs = tool(sid, "semantic_search", {"query": "example", "limit": 3}, 10)
s = tool_json(rs)
check("semantic_search labeled (vector or honest fallback)", s.get("mode") in ("vector", "keyword-fallback"), f"mode={s.get('mode')}")

# --- 9. graph: seed via cypher-shell happens externally; here query + injection probe ---
_, _, rg = tool(sid, "query_entity", {"name": "IntelHub", "limit": 5}, 11)
g = tool_json(rg)
check("query_entity executes", "rows" in g or "count" in g, f"count={g.get('count')}")
_, _, rinj = tool(sid, "query_entity", {"name": "x' DETACH DELETE e //", "limit": 5}, 12)
inj = tool_json(rinj)
check("injection probe handled safely", "rows" in inj or "error" in inj, "no crash")

# --- 10. system health ---
_, _, rh = tool(sid, "get_system_health", {}, 13)
h = tool_json(rh)
comp = h.get("components", {})
check("get_system_health all up", all(v.get("status") == "up" for v in comp.values()),
      json.dumps({k: v.get("status") for k, v in comp.items()}))

print(f"\n== {len(PASS)} passed, {len(FAIL)} failed ==")
sys.exit(1 if FAIL else 0)
