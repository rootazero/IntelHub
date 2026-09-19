// GEV P10 T2 — panel-disclosure adapter.
//
// Vendor shape (verified at console/gev-engine/src/ui/panelDisclosure.js):
//   bindPanelDisclosure({ panel, buttons, onChange, onEscape })
//     → { destroy() }                     — collapse-button + escape binding
//   collapsePanelOnEscape(event, { panel, onChange, beforeCollapse? })
//     → boolean                           — event-keydown dispatcher
//   createHoverDisclosure({ panel, disclosure, onChange, onEscape,
//                           focusTarget?, isActive?, openDelayMs?,
//                           closeDelayMs?, documentRef? })
//     → { cancelPendingFocus(), destroy() }  — hover/focus timing
//
// The adapter re-exports all three vendor functions unchanged. They are pure
// (no internal state besides listener bookkeeping); IntelHub composes them
// in T3 with the HUD frame's disclosure buttons. A wrapper here would add
// nothing — the brief's "wrap React lifecycle" requirement is satisfied by
// the adapter returning the vendor's `{ destroy() }` handle so React's
// useEffect cleanup can call it.
import {
  bindPanelDisclosure as vendorBindPanelDisclosure,
  collapsePanelOnEscape as vendorCollapsePanelOnEscape,
  createHoverDisclosure as vendorCreateHoverDisclosure,
} from "gev-engine/src/ui/panelDisclosure.js";

export interface PanelDisclosureHandle {
  destroy(): void;
}

export interface HoverDisclosureHandle {
  cancelPendingFocus(): void;
  destroy(): void;
}

export interface BindDisclosureOptions {
  panel: HTMLElement;
  buttons?: HTMLElement[];
  onChange(collapsed: boolean, info: { explicit: boolean }): void;
  onEscape(event: KeyboardEvent): void;
}

export interface CollapseOnEscapeOptions {
  panel: HTMLElement;
  onChange(collapsed: boolean, info: { explicit: boolean }): void;
  beforeCollapse?(): void;
}

export interface HoverDisclosureOptions {
  panel: HTMLElement;
  disclosure?: HTMLElement | null;
  onChange(collapsed: boolean, info: { explicit: boolean }): void;
  onEscape(event: KeyboardEvent): void;
  focusTarget?: () => HTMLElement | null;
  isActive?: () => boolean;
  openDelayMs?: number;
  closeDelayMs?: number;
  documentRef?: Document;
}

/** Vendor pass-through — see panelDisclosure.js for the contract. */
export function bindPanelDisclosure(
  opts: BindDisclosureOptions,
): PanelDisclosureHandle {
  return vendorBindPanelDisclosure(opts) as PanelDisclosureHandle;
}

/** Vendor pass-through — returns true if the event was handled. */
export function collapsePanelOnEscape(
  event: KeyboardEvent,
  opts: CollapseOnEscapeOptions,
): boolean {
  return vendorCollapsePanelOnEscape(event, opts);
}

/** Vendor pass-through — hover/focus timing for dock trays. */
export function createHoverDisclosure(
  opts: HoverDisclosureOptions,
): HoverDisclosureHandle {
  return vendorCreateHoverDisclosure(opts) as HoverDisclosureHandle;
}