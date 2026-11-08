//! PolygonHead.MathApproximation primitives recovered for gibs and Death.
//! Evidence: research/gibs.md and research/summon-death.md.

/// Native FastSinCos parabola, for the caller's angle in [-pi, pi].
pub fn fast_sin_cos(angle: f32) -> (f32, f32) {
    let f = |a: f32| {
        if a < 0.0 {
            4.0 / std::f32::consts::PI * a + 0.405_284_73 * a * a
        } else {
            4.0 / std::f32::consts::PI * a - 0.405_284_73 * a * a
        }
    };
    let sin = f(angle);
    let mut a = angle + std::f32::consts::PI / 2.0;
    if a > std::f32::consts::PI {
        a -= std::f32::consts::PI * 2.0;
    }
    (sin, f(a))
}
