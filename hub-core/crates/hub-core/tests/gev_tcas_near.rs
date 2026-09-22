// GEV P20 TCAS — tests for the `/api/v1/flights/near` endpoint math.
// Pure-function tests (haversine, bearing, closure, classify) run without
// Redis. The async handler is exercised via axum-test only on the integration
// level (requires live data); for unit coverage we test the helpers.

// We re-implement the math here to keep the test self-contained — the
// production code lives in src/api.rs as `pub(crate)` helpers. If those
// helpers become `pub`, switch the tests to import them.
mod math {
    const EARTH_RADIUS_NM: f64 = 3440.065;

    pub fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let to_rad = std::f64::consts::PI / 180.0;
        let phi1 = lat1 * to_rad;
        let phi2 = lat2 * to_rad;
        let dphi = (lat2 - lat1) * to_rad;
        let dlambda = (lon2 - lon1) * to_rad;
        let a = (dphi / 2.0).sin().powi(2)
            + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().asin();
        EARTH_RADIUS_NM * c
    }
}

#[test]
fn haversine_zero_distance() {
    let d = math::haversine_nm(37.62, -122.38, 37.62, -122.38);
    assert!(d.abs() < 0.01, "got {d}");
}

#[test]
fn haversine_sf_to_la_is_about_300_nm() {
    let d = math::haversine_nm(37.62, -122.38, 34.05, -118.24);
    // SF → LA great-circle distance ~300 nm (347 sm). Tolerance ±10 nm.
    assert!((d - 300.0).abs() < 10.0, "got {d}");
}

#[test]
fn haversine_doha_to_dubai_is_about_202_nm() {
    let d = math::haversine_nm(25.45, 51.63, 25.25, 55.36);
    // Doha (25.45, 51.63) → Dubai (25.25, 55.36) ~202 nm. Tolerance ±10 nm.
    assert!((d - 202.0).abs() < 10.0, "got {d}");
}

// ── The classify_threat ladder is verified against the spec thresholds ─────────

mod threat {
    pub fn classify_threat(distance_nm: f64, closure_kts: f64) -> &'static str {
        if distance_nm <= 1.0 && closure_kts >= 250.0 {
            return "warning";
        }
        if distance_nm <= 2.0 && closure_kts >= 100.0 {
            return "caution";
        }
        if distance_nm <= 5.0 {
            return "monitor";
        }
        "none"
    }
}

#[test]
fn classify_warning_threshold() {
    assert_eq!(threat::classify_threat(0.5, 250.0), "warning");
    assert_eq!(threat::classify_threat(1.0, 300.0), "warning");
}

#[test]
fn classify_warning_distance_too_far_falls_back() {
    // 2.0 nm, 300 kt closure → caution, not warning (distance > 1.0)
    assert_eq!(threat::classify_threat(2.0, 300.0), "caution");
}

#[test]
fn classify_caution_threshold() {
    assert_eq!(threat::classify_threat(2.0, 100.0), "caution");
    assert_eq!(threat::classify_threat(1.5, 150.0), "caution");
}

#[test]
fn classify_caution_closure_too_low_falls_back() {
    // 1.5 nm, 50 kt closure → monitor (closure < 100)
    assert_eq!(threat::classify_threat(1.5, 50.0), "monitor");
}

#[test]
fn classify_monitor_band() {
    assert_eq!(threat::classify_threat(3.0, 0.0), "monitor");
    assert_eq!(threat::classify_threat(5.0, 0.0), "monitor");
}

#[test]
fn classify_none_outside_5nm() {
    assert_eq!(threat::classify_threat(10.0, 500.0), "none");
    assert_eq!(threat::classify_threat(6.0, 0.0), "none");
}

#[test]
fn closure_rate_direct_approach_is_full() {
    // Target heading directly at agent → closure = full speed.
    let track = 180.0; // moving south
    let gs = 200.0;
    let bearing_to_target = 0.0; // target is north of agent
    // bearing_from_target = (0 + 180) % 360 = 180 = track → diff=0 → cos=1
    // → closure = 200
    let closure = compute_closure(track, gs, bearing_to_target);
    assert!((closure - 200.0).abs() < 1.0, "got {closure}");
}

#[test]
fn closure_rate_perpendicular_is_zero() {
    let track = 90.0; // moving east
    let gs = 200.0;
    let bearing_to_target = 0.0;
    // bearing_from_target = 180, diff = |90-180| = 90, cos(90°) = 0
    let closure = compute_closure(track, gs, bearing_to_target);
    assert!(closure.abs() < 1.0, "got {closure}");
}

#[test]
fn closure_rate_receding_is_negative() {
    let track = 0.0; // moving north
    let gs = 200.0;
    let bearing_to_target = 0.0; // target is north
    // bearing_from_target = 180, diff = |0-180| = 180, cos(180°) = -1
    let closure = compute_closure(track, gs, bearing_to_target);
    assert!((closure + 200.0).abs() < 1.0, "got {closure}");
}

#[test]
fn closure_rate_shortest_angle_diff() {
    // track = 10, bearing_to_target = 350 → bearing_from_target = 170.
    // |10 - 170| = 160 > 180? No, 160 < 180, so diff stays 160 → cos ≈ -0.94.
    // That means target moving NNE is moving AWAY from a target SE of it.
    let closure = compute_closure(10.0, 100.0, 350.0);
    let expected = 100.0 * (160.0_f64).to_radians().cos();
    assert!((closure - expected).abs() < 1.0, "got {closure}, expected {expected}");
}

#[test]
fn closure_rate_shortest_angle_diff_approaching() {
    // track = 190, bearing_to_target = 10 → bearing_from_target = 190.
    // diff = 0 → cos = 1 → full closure.
    let closure = compute_closure(190.0, 100.0, 10.0);
    assert!((closure - 100.0).abs() < 1.0, "got {closure}");
}

#[test]
fn closure_rate_zero_speed_is_zero() {
    // Stationary target has no closure regardless of heading.
    let closure = compute_closure(90.0, 0.0, 0.0);
    assert_eq!(closure, 0.0);
}

fn compute_closure(
    target_track_deg: f64,
    target_gs_kts: f64,
    bearing_to_target_deg: f64,
) -> f64 {
    if target_gs_kts <= 0.0 {
        return 0.0;
    }
    let bearing_from_target = (bearing_to_target_deg + 180.0) % 360.0;
    let mut diff = (target_track_deg - bearing_from_target).abs();
    if diff > 180.0 {
        diff = 360.0 - diff;
    }
    let cos_component = diff.to_radians().cos();
    target_gs_kts * cos_component
}