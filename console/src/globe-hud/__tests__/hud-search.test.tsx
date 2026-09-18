import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, test, vi } from "vitest";
import { HudTopBar } from "../HudTopBar";
import type {
  LocationSearchHandle,
  SearchState,
} from "../../gev-visual/location-search";

function fakeSearch(initial: SearchState = "idle"): LocationSearchHandle & {
  run: ReturnType<typeof vi.fn>;
  setState(s: SearchState): void;
} {
  let state = initial;
  const listeners = new Set<(s: SearchState) => void>();
  return {
    run: vi.fn(async () => {}),
    getState: () => state,
    subscribe: (fn) => {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
    setState(s: SearchState) {
      state = s;
      listeners.forEach((f) => f(s));
    },
    destroy: vi.fn(),
  };
}

// HudTopBar uses useNavigate/useLocation (Back button) — tests must wrap in
// MemoryRouter. (The 5 pre-existing hud-bars failures are exactly this
// missing-wrapper bug; do not replicate it.)
function renderTopBar(locationSearch: LocationSearchHandle | null) {
  return render(
    <MemoryRouter>
      <HudTopBar overview={null} locationSearch={locationSearch} />
    </MemoryRouter>,
  );
}

afterEach(() => cleanup());

describe("HudTopBar location search", () => {
  test("search input is enabled with location placeholder", () => {
    renderTopBar(fakeSearch());
    const input = screen.getByTestId("hud-search-location");
    expect(input).toBeEnabled();
    expect(input).toHaveAttribute(
      "placeholder",
      expect.stringContaining("地点"),
    );
  });

  test("Enter triggers run with the typed query", () => {
    const search = fakeSearch();
    renderTopBar(search);
    const input = screen.getByTestId("hud-search-location");
    fireEvent.change(input, { target: { value: "Paris" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(search.run).toHaveBeenCalledWith("Paris");
  });

  test("missing state renders zh/en feedback", () => {
    renderTopBar(fakeSearch("missing"));
    expect(screen.getByTestId("hud-search-status")).toHaveTextContent("未找到");
  });

  test("null handle keeps the input disabled (engine not ready)", () => {
    renderTopBar(null);
    expect(screen.getByTestId("hud-search-location")).toBeDisabled();
  });
});
