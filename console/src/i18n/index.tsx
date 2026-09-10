// Zero-dependency i18n (SP-i18n, approved design 2026-09-09).
// Detection: localStorage('intelhub-lang') → navigator.language (zh* → zh) → en.
// Missing key → falls back to English string, warns in devtools.

import { createContext, useContext, useState, type ReactNode } from "react";
import { en, type Dict } from "./en";
import { zh } from "./zh";

export type Lang = "zh" | "en";

/** Registered languages — add an entry here to support a new one. */
export const LANGS: { code: Lang; label: string; short: string }[] = [
  { code: "zh", label: "中文", short: "中" },
  { code: "en", label: "English", short: "EN" },
];

const DICTS: Record<Lang, Dict> = { zh, en };
const LS_KEY = "intelhub-lang";

function detect(): Lang {
  const saved = localStorage.getItem(LS_KEY);
  if (saved === "zh" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

interface I18n {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
}

const Ctx = createContext<I18n>({
  lang: "en",
  setLang: () => {},
  t: (k) => k,
});

function lookup(dict: Dict, path: string): string | undefined {
  let node: unknown = dict;
  for (const part of path.split(".")) {
    if (node == null || typeof node !== "object") return undefined;
    node = (node as Record<string, unknown>)[part];
  }
  return typeof node === "string" ? node : undefined;
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detect);
  const setLang = (l: Lang) => {
    localStorage.setItem(LS_KEY, l);
    setLangState(l);
  };
  const t = (key: string, vars?: Record<string, string | number>) => {
    let s = lookup(DICTS[lang], key);
    if (s === undefined) {
      s = lookup(en, key);
      if (s === undefined) {
        console.warn(`[i18n] missing key: ${key}`);
        return key;
      }
    }
    if (vars) for (const [k, v] of Object.entries(vars)) s = s!.replaceAll(`{${k}}`, String(v));
    return s!;
  };
  return <Ctx.Provider value={{ lang, setLang, t }}>{children}</Ctx.Provider>;
}

export function useT() {
  return useContext(Ctx);
}

/** Map a server-provided enum value through the dict; unknown values pass through. */
export function useEnum() {
  const { t } = useT();
  return (ns: string, value: string) => {
    const key = `enum.${ns}.${value}`;
    const s = t(key);
    return s === key ? value : s;
  };
}
