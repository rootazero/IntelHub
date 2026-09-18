// GEV P8 — HudAnnotationList component tests. Pure presentational; no router,
// no async, no engine — the list routes row-click → onSelect and delete →
// onDelete, and hides itself when visible=false.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { HudAnnotationList } from "../HudAnnotationList";
import type { AnnotationSpec } from "../../gev-visual/annotations";

afterEach(cleanup);

const specs: AnnotationSpec[] = [
  {
    id: "a1",
    shape: "pin",
    vertices: [{ lon: 1, lat: 2 }],
    label: "Pin A",
    color: "primary",
  },
  {
    id: "a2",
    shape: "line",
    vertices: [
      { lon: 1, lat: 2 },
      { lon: 3, lat: 4 },
    ],
    label: "Route B",
    color: "cyan",
  },
];

describe("HudAnnotationList", () => {
  test("not rendered when visible=false", () => {
    const { container } = render(
      <HudAnnotationList
        viewer={{}}
        annotations={specs}
        onSelect={() => {}}
        onDelete={() => {}}
        visible={false}
      />,
    );
    expect(
      container.querySelector('[data-testid="hud-annotation-list"]'),
    ).toBeNull();
  });

  test("renders one row per annotation when visible=true", () => {
    render(
      <HudAnnotationList
        viewer={{}}
        annotations={specs}
        onSelect={() => {}}
        onDelete={() => {}}
        visible
      />,
    );
    expect(screen.getByTestId("hud-annotation-list")).toBeInTheDocument();
    expect(screen.getAllByTestId("hud-annotation-row")).toHaveLength(2);
  });

  test("click row triggers onSelect with the spec", () => {
    const onSelect = vi.fn();
    render(
      <HudAnnotationList
        viewer={{}}
        annotations={specs}
        onSelect={onSelect}
        onDelete={() => {}}
        visible
      />,
    );
    fireEvent.click(screen.getByText(/Pin A/));
    expect(onSelect).toHaveBeenCalledWith(specs[0]);
  });

  test("click delete triggers onDelete with the id", () => {
    const onDelete = vi.fn();
    render(
      <HudAnnotationList
        viewer={{}}
        annotations={specs}
        onSelect={() => {}}
        onDelete={onDelete}
        visible
      />,
    );
    fireEvent.click(screen.getAllByTestId("hud-annotation-delete")[0]);
    expect(onDelete).toHaveBeenCalledWith("a1");
  });
});
