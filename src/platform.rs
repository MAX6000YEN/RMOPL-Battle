//! Platform geometry: rounded rectangles at arbitrary position and rotation.
//!
//! Every query works by projecting into the platform's own basis, which is what
//! makes a rotated platform cost the same as an axis-aligned one. The basis is
//! two vectors and a dot product; there is no matrix and no trigonometry
//! anywhere except in building `right()`.
//!
//! Only the flat top face is *used* this phase. [`Platform::normal_at`] is the
//! deliberate exception: it answers all eight zones, because half of it is more
//! work than all of it and the surface walking that arrives later leans on it
//! being right.

use crate::math::{FVec2, Fix, cos, sin};

/// What a surface is made of. Only the friction rules differ, and nothing
/// reads this yet — ice is surface walking, which is a later phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformKind {
    Normal,
    Ice,
}

impl PlatformKind {
    /// A stable number for hashing and ordering. Not a cast: the discriminant
    /// is an implementation detail and this is the wire-visible value.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Ice => 1,
        }
    }
}

/// A platform's own dimensions, without saying where it is.
///
/// Split from [`Platform`] so that an entity's `position` and `rotation` are
/// the single source of truth for where a platform sits. Storing a centre in
/// two places is how a moving platform ends up drawn in one spot and collided
/// with in another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlatformShape {
    /// Half width and half height of the **inner** rectangle, with the corner
    /// radius **excluded**.
    ///
    /// So the flat top face is exactly `2 * extents.x` long and the full
    /// bounding box is `2 * extents + 2 * radius` in each direction. This
    /// convention decides every perimeter length in the surface-walking phase;
    /// changing it later means redoing the surface parameterisation rather than
    /// editing a constant.
    pub extents: FVec2,
    /// Corner radius. Zero is a sharp-cornered rectangle.
    pub radius: Fix,
    pub kind: PlatformKind,
}

/// A platform in the world: a shape, a centre and a rotation.
///
/// Built on demand from an entity rather than stored, so it is always
/// consistent with the entity it came from. Self-contained on purpose — every
/// geometric query here can be tested without a simulation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Platform {
    pub center: FVec2,
    /// See [`PlatformShape::extents`] — the corner radius is excluded.
    pub extents: FVec2,
    pub radius: Fix,
    pub rotation: Fix,
    pub kind: PlatformKind,
}

/// Trig output can land up to two raw units outside -1..=1, and a basis vector
/// is exactly the kind of thing that must not.
fn clamp_unit(v: Fix) -> Fix {
    v.clamp(-Fix::ONE, Fix::ONE)
}

impl Platform {
    pub const fn new(center: FVec2, rotation: Fix, shape: PlatformShape) -> Self {
        Self {
            center,
            extents: shape.extents,
            radius: shape.radius,
            rotation,
            kind: shape.kind,
        }
    }

    /// The platform's local +x axis in world space.
    ///
    /// An unrotated platform answers exactly `(1, 0)` rather than going through
    /// CORDIC, which returns a value a few raw units off and would put every
    /// landing on every axis-aligned platform in the game slightly askew for no
    /// reason. Most platforms are unrotated, so this is the common path as well
    /// as the exact one.
    pub fn right(&self) -> FVec2 {
        if self.rotation == Fix::ZERO {
            return FVec2::RIGHT;
        }
        FVec2::new(
            clamp_unit(cos(self.rotation)),
            clamp_unit(sin(self.rotation)),
        )
    }

    /// The platform's local +y axis in world space: the direction its top face
    /// points.
    pub fn up(&self) -> FVec2 {
        self.right().perp()
    }

    /// A world point expressed in the platform's basis.
    pub fn to_local(&self, point: FVec2) -> FVec2 {
        let offset = point - self.center;
        FVec2::new(offset.dot(self.right()), offset.dot(self.up()))
    }

    /// A point in the platform's basis expressed in world space.
    pub fn to_world(&self, local: FVec2) -> FVec2 {
        self.center + self.right() * local.x + self.up() * local.y
    }

    /// The outward surface normal nearest `point`, for all eight zones: four
    /// flat faces and four corner arcs.
    ///
    /// Only the up-facing zone is exercised this phase. The rest is written now
    /// because it is a single expression per zone and because the eight-case
    /// test is the safety net the surface-walking phase needs to already exist.
    ///
    /// Boundaries resolve in favour of the flat faces: a point directly above a
    /// corner's inner edge is on the top face, not on the arc. That is the rule
    /// that keeps `normal_at` and the flat-top landing query from disagreeing
    /// about a body that lands exactly on the end of the top face.
    ///
    /// A point strictly inside the inner rectangle is not on any surface at
    /// all; it answers `up()` so the function is total. Pushing an overlapping
    /// body back out is collision work, and that is a later phase.
    pub fn normal_at(&self, point: FVec2) -> FVec2 {
        let l = self.to_local(point);
        let (ex, ey) = (self.extents.x, self.extents.y);
        let within_x = l.x >= -ex && l.x <= ex;
        let within_y = l.y >= -ey && l.y <= ey;

        let local_normal = if within_x && l.y >= ey {
            FVec2::UP
        } else if within_x && l.y <= -ey {
            FVec2::DOWN
        } else if within_y && l.x >= ex {
            FVec2::RIGHT
        } else if within_y && l.x <= -ex {
            FVec2::LEFT
        } else if within_x && within_y {
            // Strictly inside the solid.
            FVec2::UP
        } else {
            // A corner arc: the normal points away from the corner's centre of
            // curvature, which is the inner rectangle's corner.
            let corner = FVec2::new(
                if l.x > Fix::ZERO { ex } else { -ex },
                if l.y > Fix::ZERO { ey } else { -ey },
            );
            let away = (l - corner).normalized_safe();
            if away == FVec2::ZERO {
                // Exactly on the centre of curvature. Undefined by geometry, so
                // pick the diagonal and stay deterministic about it.
                corner.normalized_safe()
            } else {
                away
            }
        };

        self.right() * local_normal.x + self.up() * local_normal.y
    }

    /// Total length once around the outside.
    ///
    /// Four straight runs of `2 * extents` between the corners, plus four
    /// quarter-arcs that together make one full circle.
    pub fn perimeter(&self) -> Fix {
        (self.extents.x + self.extents.y) * Fix::lit("4") + self.radius * Fix::TAU
    }

    /// Where the flat top face ends, as a fraction of the perimeter.
    ///
    /// Surface positions run **clockwise from the left end of the top face**,
    /// so the top face is segment zero and spans `[0, top_face_end()]`. Chosen
    /// over the counter-clockwise-from-+x convention that would match
    /// [`FVec2::perp`] because the only face in scope this phase then starts at
    /// exactly zero, and because moving right along the ground increases the
    /// coordinate, which is what movement code will want. Values stored now are
    /// already in their final coordinate system; the remaining seven segments
    /// extend this without invalidating any of them.
    pub fn top_face_end(&self) -> Fix {
        (self.extents.x + self.extents.x) / self.perimeter()
    }

    /// The world position of a point on the surface.
    ///
    /// `None` outside the flat top face: the other seven segments belong to the
    /// surface-walking phase. Callers must handle that rather than unwrap it —
    /// it shares a code path with a body grounded on a platform that no longer
    /// exists.
    pub fn surface_point(&self, local_pos: Fix) -> Option<FVec2> {
        if local_pos < Fix::ZERO || local_pos > self.top_face_end() {
            return None;
        }
        let along = local_pos * self.perimeter();
        Some(self.top_face_point(along - self.extents.x))
    }

    /// The surface position of a world point directly above the top face, or
    /// `None` if it is not above the flat part.
    pub fn top_face_local_pos(&self, point: FVec2) -> Option<Fix> {
        let l = self.to_local(point);
        if l.x < -self.extents.x || l.x > self.extents.x {
            return None;
        }
        Some(self.local_pos_of_top_x(l.x))
    }

    /// Where a segment from `from` to `from + delta` first crosses the flat top
    /// face, as a fraction of the segment in `0..=1`.
    ///
    /// Only counts a crossing that arrives **from above and moving downwards**
    /// in the platform's own frame, so a body travelling up through a platform
    /// does not attach to it.
    ///
    /// The comparisons below avoid forming the quotient until it is known to be
    /// at most one. A near-horizontal segment far above the surface would
    /// otherwise produce a value that does not fit in the fixed-point range,
    /// and a fixed-point overflow is a panic in debug and a wrapped, silently
    /// divergent number in release.
    pub fn top_face_crossing(&self, from: FVec2, delta: FVec2) -> Option<Fix> {
        let (right, up) = (self.right(), self.up());
        let origin = {
            let offset = from - self.center;
            FVec2::new(offset.dot(right), offset.dot(up))
        };
        let direction = FVec2::new(delta.dot(right), delta.dot(up));

        if direction.y >= Fix::ZERO {
            return None;
        }

        // Negative when the segment starts above the surface, which is the only
        // case that can be a landing.
        let gap = self.extents.y + self.radius - origin.y;
        if gap > Fix::ZERO || gap < direction.y {
            return None;
        }

        let t = gap / direction.y;
        let x = origin.x + direction.x * t;
        if x < -self.extents.x || x > self.extents.x {
            return None;
        }
        Some(t)
    }

    /// A point on the flat top face at local x `x`, in world space.
    pub fn top_face_point(&self, x: Fix) -> FVec2 {
        self.to_world(FVec2::new(x, self.extents.y + self.radius))
    }

    /// The surface position of local x `x` on the top face.
    pub fn local_pos_of_top_x(&self, x: Fix) -> Fix {
        (x + self.extents.x) / self.perimeter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::TRIG_ERROR_BOUND;

    fn shape(ex: &str, ey: &str, radius: &str) -> PlatformShape {
        PlatformShape {
            extents: FVec2::new(Fix::lit(ex), Fix::lit(ey)),
            radius: Fix::lit(radius),
            kind: PlatformKind::Normal,
        }
    }

    fn flat() -> Platform {
        Platform::new(FVec2::ZERO, Fix::ZERO, shape("4", "1", "0.5"))
    }

    fn at(x: &str, y: &str) -> FVec2 {
        FVec2::new(Fix::lit(x), Fix::lit(y))
    }

    /// Within the documented trig error, componentwise.
    fn near(a: FVec2, b: FVec2) -> bool {
        (a.x.to_bits() - b.x.to_bits()).abs() <= TRIG_ERROR_BOUND
            && (a.y.to_bits() - b.y.to_bits()).abs() <= TRIG_ERROR_BOUND
    }

    /// An unrotated platform answers with exact axis vectors rather than with
    /// CORDIC's approximation, which would otherwise put every landing on every
    /// axis-aligned platform in the game a few raw units askew.
    #[test]
    fn an_unrotated_basis_is_exact() {
        assert_eq!(flat().right(), FVec2::RIGHT);
        assert_eq!(flat().up(), FVec2::UP);
    }

    /// And the shortcut has to agree with the general path it is skipping.
    #[test]
    fn the_unrotated_shortcut_matches_the_trig_it_replaces() {
        let barely = Platform::new(FVec2::ZERO, Fix::from_bits(1), shape("4", "1", "0.5"));
        assert!(near(barely.right(), FVec2::RIGHT));
        assert!(near(barely.up(), FVec2::UP));
    }

    #[test]
    fn a_rotated_basis_is_orthogonal_and_of_unit_length() {
        for rotation in ["0.35", "1.2", "-2.5", "3.1"] {
            let p = Platform::new(at("3", "-2"), Fix::lit(rotation), shape("4", "1", "0.5"));
            let (right, up) = (p.right(), p.up());
            // Exactly perpendicular by construction, but the dot product of two
            // inexact values is not exactly zero: a fixed-point multiply floors,
            // so `x * -y` and `-(x * y)` can differ by a raw unit.
            let skew = right.dot(up).to_bits().abs();
            assert!(skew <= 2, "basis skewed by {skew} raw units");
            let err = Fix::ONE.to_bits() - right.sqr_magnitude().to_bits();
            assert!(err.abs() <= TRIG_ERROR_BOUND * 2, "off by {err} raw units");
        }
    }

    /// The round trip is not exact on a rotated platform and cannot be: the
    /// basis itself is a few raw units off unit length, and that error is
    /// multiplied by how far the point is from the centre. It is deterministic,
    /// it is always tiny in world units, and callers must not assume a point
    /// survives the trip bit for bit.
    #[test]
    fn local_and_world_coordinates_round_trip() {
        for rotation in ["0", "0.7", "-1.9"] {
            let p = Platform::new(at("-6", "12"), Fix::lit(rotation), shape("4", "1", "0.5"));
            for point in [at("0", "0"), at("-6", "13.5"), at("2.25", "-8")] {
                let drift = p.to_world(p.to_local(point)).distance(point);
                assert!(
                    drift < Fix::lit("0.000001"),
                    "{rotation}: {point:?} drifted by {drift}"
                );
            }
        }
        // An unrotated platform has an exact basis, so it does round-trip.
        let p = flat();
        assert_eq!(p.to_world(p.to_local(at("2.25", "-8"))), at("2.25", "-8"));
    }

    /// All eight zones, on an axis-aligned platform. The four faces are exact;
    /// the four corner normals are diagonals of a rounded rectangle whose
    /// corner is at (4, 1), so they are checked by direction rather than by
    /// exact bits.
    #[test]
    fn normal_at_answers_all_eight_zones() {
        let p = flat();

        assert_eq!(p.normal_at(at("0", "5")), FVec2::UP, "top face");
        assert_eq!(p.normal_at(at("-3.5", "1.5")), FVec2::UP, "top face, left");
        assert_eq!(p.normal_at(at("0", "-5")), FVec2::DOWN, "bottom face");
        assert_eq!(p.normal_at(at("9", "0")), FVec2::RIGHT, "right face");
        assert_eq!(p.normal_at(at("-9", "0.75")), FVec2::LEFT, "left face");

        for (x, y, sign_x, sign_y) in [
            ("6", "3", 1, 1),
            ("6", "-3", 1, -1),
            ("-6", "3", -1, 1),
            ("-6", "-3", -1, -1),
        ] {
            let n = p.normal_at(at(x, y));
            assert_eq!(n.x.signum(), Fix::from_num(sign_x), "corner {x},{y}");
            assert_eq!(n.y.signum(), Fix::from_num(sign_y), "corner {x},{y}");
            // A corner normal is a unit vector, up to the truncation that
            // `normalized_safe` documents.
            let err = Fix::ONE.to_bits() - n.sqr_magnitude().to_bits();
            assert!((0..=8).contains(&err), "corner normal off by {err}");
        }
    }

    /// The same eight zones on a rotated platform: every query point is built
    /// in the platform's own frame, so the answers must be the rotated basis
    /// vectors.
    #[test]
    fn normal_at_answers_all_eight_zones_on_a_rotated_platform() {
        let p = Platform::new(at("7", "-3"), Fix::lit("0.9"), shape("4", "1", "0.5"));
        let (right, up) = (p.right(), p.up());

        for (local, expected) in [
            (at("0", "5"), up),
            (at("0", "-5"), -up),
            (at("9", "0"), right),
            (at("-9", "0"), -right),
        ] {
            let got = p.normal_at(p.to_world(local));
            assert!(near(got, expected), "{local:?} gave {got:?}");
        }

        for (local, sx, sy) in [
            (at("6", "3"), 1, 1),
            (at("6", "-3"), 1, -1),
            (at("-6", "3"), -1, 1),
            (at("-6", "-3"), -1, -1),
        ] {
            let got = p.normal_at(p.to_world(local));
            let expected = (right * Fix::from_num(sx) + up * Fix::from_num(sy)).normalized_safe();
            assert!(near(got, expected), "corner {local:?} gave {got:?}");
        }
    }

    /// The boundary rule that keeps the landing query and the normal from
    /// disagreeing: a point above the very end of the flat top is on the top
    /// face, not on the corner arc.
    #[test]
    fn the_end_of_the_flat_top_belongs_to_the_top_face() {
        let p = flat();
        assert_eq!(p.normal_at(at("4", "1.5")), FVec2::UP);
        assert_eq!(p.normal_at(at("-4", "1.5")), FVec2::UP);
        assert_eq!(p.normal_at(at("4", "1")), FVec2::UP);
        // A hair further out and it is the arc.
        assert_ne!(p.normal_at(at("4.0000001", "1.5")), FVec2::UP);
    }

    #[test]
    fn a_point_inside_the_solid_still_gets_an_answer() {
        assert_eq!(flat().normal_at(FVec2::ZERO), FVec2::UP);
        assert_eq!(flat().normal_at(at("4", "1")), FVec2::UP);
    }

    /// Four straight runs between the corners plus four quarter-arcs that make
    /// one circle. The extents exclude the radius, which is what makes this
    /// arithmetic as simple as it is.
    #[test]
    fn the_perimeter_is_the_straights_plus_one_circle() {
        let p = flat();
        let expected = Fix::lit("4") * (Fix::lit("4") + Fix::lit("1")) + Fix::lit("0.5") * Fix::TAU;
        assert_eq!(p.perimeter(), expected);

        // A sharp-cornered rectangle is just the straights.
        let sharp = Platform::new(FVec2::ZERO, Fix::ZERO, shape("4", "1", "0"));
        assert_eq!(sharp.perimeter(), Fix::lit("20"));
        assert_eq!(sharp.top_face_end(), Fix::lit("8") / Fix::lit("20"));
    }

    /// Surface positions run clockwise from the left end of the top face, so
    /// walking right along the ground increases the coordinate.
    #[test]
    fn surface_positions_start_at_the_left_end_of_the_top_face() {
        let p = flat();
        let surface_y = Fix::lit("1.5");

        assert_eq!(p.surface_point(Fix::ZERO), Some(at("-4", "1.5")));

        let end = p.surface_point(p.top_face_end()).expect("the far end");
        assert_eq!(end.y, surface_y);
        assert!((end.x - Fix::lit("4")).abs() < Fix::lit("0.000001"));

        let middle = p.surface_point(p.top_face_end() / Fix::lit("2")).unwrap();
        assert!(middle.x.abs() < Fix::lit("0.000001"), "{middle:?}");
        assert_eq!(middle.y, surface_y);
    }

    /// Everything past the top face belongs to the surface-walking phase and
    /// must say so rather than answering with something plausible.
    #[test]
    fn surface_positions_off_the_top_face_are_none() {
        let p = flat();
        assert_eq!(p.surface_point(Fix::lit("-0.01")), None);
        assert_eq!(p.surface_point(p.top_face_end() + Fix::lit("0.01")), None);
        assert_eq!(p.surface_point(Fix::lit("0.99")), None);
    }

    #[test]
    fn a_surface_position_round_trips_through_a_world_point() {
        for rotation in ["0", "0.6", "-2.2"] {
            let p = Platform::new(at("2", "9"), Fix::lit(rotation), shape("4", "1", "0.5"));
            for step in 0..=8 {
                let local_pos = p.top_face_end() * Fix::from_num(step) / Fix::from_num(8);
                let point = p.surface_point(local_pos).expect("on the top face");
                let back = p.top_face_local_pos(point).expect("above the top face");
                let err = (back - local_pos).abs();
                assert!(err < Fix::lit("0.0000001"), "{rotation} step {step}: {err}");
            }
        }
    }

    #[test]
    fn a_point_beyond_the_flat_top_has_no_surface_position() {
        let p = flat();
        assert_eq!(p.top_face_local_pos(at("4.5", "1.5")), None);
        assert_eq!(p.top_face_local_pos(at("-4.5", "1.5")), None);
        assert!(p.top_face_local_pos(at("4", "1.5")).is_some());
    }

    #[test]
    fn a_downward_segment_crosses_the_top_face() {
        let p = flat();
        // Starts one unit above the surface and travels two units down.
        let t = p.top_face_crossing(at("0", "2.5"), at("0", "-2")).unwrap();
        assert_eq!(t, Fix::lit("0.5"));
    }

    #[test]
    fn a_segment_that_stops_short_does_not_cross() {
        let p = flat();
        assert_eq!(p.top_face_crossing(at("0", "2.5"), at("0", "-0.9")), None);
    }

    /// A body travelling upwards through a platform must not attach to it.
    #[test]
    fn an_upward_segment_never_crosses() {
        let p = flat();
        assert_eq!(p.top_face_crossing(at("0", "-5"), at("0", "9")), None);
        assert_eq!(p.top_face_crossing(at("0", "1.5"), at("0", "1")), None);
    }

    #[test]
    fn a_segment_beside_the_platform_does_not_cross() {
        let p = flat();
        assert_eq!(p.top_face_crossing(at("6", "5"), at("0", "-9")), None);
        assert_eq!(p.top_face_crossing(at("-4.5", "5"), at("0", "-9")), None);
    }

    /// The crossing test must not form a quotient it cannot represent. A
    /// near-horizontal segment far above the surface is exactly that case, and
    /// a fixed-point overflow is a panic in debug and a silently wrong number
    /// in release.
    #[test]
    fn a_near_horizontal_segment_far_above_does_not_overflow() {
        let p = flat();
        let far = at("0", "1000000");
        assert_eq!(
            p.top_face_crossing(far, FVec2::new(Fix::lit("50"), Fix::from_bits(-1))),
            None
        );
        assert_eq!(
            p.top_face_crossing(far, FVec2::new(Fix::from_bits(-1), Fix::from_bits(-1))),
            None
        );
    }

    #[test]
    fn a_rotated_platform_is_crossed_in_its_own_frame() {
        let p = Platform::new(at("0", "0"), Fix::lit("0.5"), shape("4", "1", "0.5"));
        // Straight down onto the middle of a tilted top face: the crossing is
        // wherever the local frame says it is, and it exists.
        let t = p.top_face_crossing(at("0", "9"), at("0", "-12")).unwrap();
        assert!(t > Fix::ZERO && t < Fix::ONE, "t was {t}");
    }

    #[test]
    fn platform_kinds_have_distinct_stable_tags() {
        assert_eq!(PlatformKind::Normal.tag(), 0);
        assert_eq!(PlatformKind::Ice.tag(), 1);
    }
}
