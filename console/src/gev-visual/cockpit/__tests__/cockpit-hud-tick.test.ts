import { describe, expect, test, vi } from "vitest";
import { mountCockpitHudTick } from "../cockpit-hud-tick";

describe("cockpit-hud-tick", () => {
  test("start() triggers tick on cadence", () => {
    vi.useFakeTimers();
    const instruments = { update: vi.fn(), destroy: vi.fn() };
    const chaseCam = { getResolvedState: () => null };
    const flights = { getTrackedInfo: () => null };
    const tick = mountCockpitHudTick(instruments, chaseCam as any, flights, { intervalMs: 100 });
    tick.start();
    vi.advanceTimersByTime(100);
    expect(instruments.update).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(100);
    expect(instruments.update).toHaveBeenCalledTimes(2);
    tick.stop();
    vi.useRealTimers();
  });

  test("stop() cancels interval", () => {
    vi.useFakeTimers();
    const instruments = { update: vi.fn() };
    const chaseCam = { getResolvedState: () => null };
    const flights = { getTrackedInfo: () => null };
    const tick = mountCockpitHudTick(instruments, chaseCam as any, flights);
    tick.start();
    vi.advanceTimersByTime(100);
    tick.stop();
    vi.advanceTimersByTime(500);
    expect(instruments.update).toHaveBeenCalledTimes(1); // Only the first tick fired
  });

  test("isRunning() reflects interval state", () => {
    vi.useFakeTimers();
    const instruments = { update: vi.fn() };
    const chaseCam = { getResolvedState: () => null };
    const flights = { getTrackedInfo: () => null };
    const tick = mountCockpitHudTick(instruments, chaseCam as any, flights);
    expect(tick.isRunning()).toBe(false);
    tick.start();
    expect(tick.isRunning()).toBe(true);
    tick.stop();
    expect(tick.isRunning()).toBe(false);
  });

  test("viewer.isDestroyed() stops the tick", () => {
    vi.useFakeTimers();
    const instruments = { update: vi.fn() };
    const chaseCam = { getResolvedState: () => null };
    const flights = { getTrackedInfo: () => null };
    let destroyed = false;
    const viewer = { isDestroyed: () => destroyed };
    const tick = mountCockpitHudTick(instruments, chaseCam as any, flights, { viewer });
    tick.start();
    vi.advanceTimersByTime(100);
    expect(instruments.update).toHaveBeenCalledTimes(1);
    destroyed = true;
    vi.advanceTimersByTime(100);
    expect(instruments.update).toHaveBeenCalledTimes(1); // No more ticks after destroy
    expect(tick.isRunning()).toBe(false);
  });
});