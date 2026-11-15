//! Dependent points: how a constructed point follows the points it was built from.
//!
//! A midpoint, an altitude foot, a triangle center, an intersection or the 3rd and 4th vertex
//! of a square are recorded with the rule that built them, and recomputed (in the order they
//! were made, so they may depend on each other) whenever a point of the figure moves.

use super::MathFigure;
use super::construction::{
    circle_circle_intersection, line_circle_intersection, line_line_intersection,
    tangents_from_point_to_circle,
};
use super::entity::{CircleId, PointId};
use super::triangle::{Triangle, TriangleCenterType};
use p2d::math::Vector2;
use serde::{Deserialize, Serialize};

/// The rule a dependent point is computed with.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Derivation {
    /// Midpoint of AB
    Midpoint { a: PointId, b: PointId },
    /// Foot of the perpendicular from P to line AB
    Foot { p: PointId, a: PointId, b: PointId },
    /// Foot of the interior bisector from A on side BC
    BisectorFoot { a: PointId, b: PointId, c: PointId },
    /// A center of triangle ABC
    Center {
        kind: TriangleCenterType,
        a: PointId,
        b: PointId,
        c: PointId,
    },
    /// a + u·(b − a) + v·(b − a) turned 90°: keeps a shape similar (square, regular polygon...)
    Affine { a: PointId, b: PointId, u: f64, v: f64 },
    /// Intersection of lines AB and CD
    LineLine {
        a: PointId,
        b: PointId,
        c: PointId,
        d: PointId,
    },
    /// Intersection of line AB with a circle (the one nearest to where the point was)
    LineCircle {
        a: PointId,
        b: PointId,
        circle: CircleId,
    },
    /// Intersection of two circles (the one nearest to where the point was)
    CircleCircle { c1: CircleId, c2: CircleId },
    /// Touching point of a tangent from P to a circle (the one nearest to where the point was)
    Tangent { p: PointId, circle: CircleId },
}

/// A dependent point and its rule.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Derived {
    pub point: PointId,
    pub how: Derivation,
}

fn perp(v: Vector2) -> Vector2 {
    Vector2::new(-v.y, v.x)
}

impl Derivation {
    /// The points this rule reads.
    pub fn inputs(&self) -> Vec<PointId> {
        match *self {
            Self::Midpoint { a, b } | Self::Affine { a, b, .. } | Self::LineCircle { a, b, .. } => {
                vec![a, b]
            }
            Self::Foot { p, a, b } => vec![p, a, b],
            Self::BisectorFoot { a, b, c } | Self::Center { a, b, c, .. } => vec![a, b, c],
            Self::LineLine { a, b, c, d } => vec![a, b, c, d],
            Self::CircleCircle { .. } => vec![],
            Self::Tangent { p, .. } => vec![p],
        }
    }

    /// The circles this rule reads.
    pub fn circles(&self) -> Vec<CircleId> {
        match *self {
            Self::LineCircle { circle, .. } | Self::Tangent { circle, .. } => vec![circle],
            Self::CircleCircle { c1, c2 } => vec![c1, c2],
            _ => vec![],
        }
    }

    /// The same rule on other point and circle ids (when a figure is merged into another).
    pub fn mapped(
        &self,
        pt: impl Fn(PointId) -> Option<PointId>,
        circ: impl Fn(CircleId) -> Option<CircleId>,
    ) -> Option<Self> {
        Some(match *self {
            Self::Midpoint { a, b } => Self::Midpoint { a: pt(a)?, b: pt(b)? },
            Self::Foot { p, a, b } => Self::Foot {
                p: pt(p)?,
                a: pt(a)?,
                b: pt(b)?,
            },
            Self::BisectorFoot { a, b, c } => Self::BisectorFoot {
                a: pt(a)?,
                b: pt(b)?,
                c: pt(c)?,
            },
            Self::Center { kind, a, b, c } => Self::Center {
                kind,
                a: pt(a)?,
                b: pt(b)?,
                c: pt(c)?,
            },
            Self::Affine { a, b, u, v } => Self::Affine {
                a: pt(a)?,
                b: pt(b)?,
                u,
                v,
            },
            Self::LineLine { a, b, c, d } => Self::LineLine {
                a: pt(a)?,
                b: pt(b)?,
                c: pt(c)?,
                d: pt(d)?,
            },
            Self::LineCircle { a, b, circle } => Self::LineCircle {
                a: pt(a)?,
                b: pt(b)?,
                circle: circ(circle)?,
            },
            Self::CircleCircle { c1, c2 } => Self::CircleCircle {
                c1: circ(c1)?,
                c2: circ(c2)?,
            },
            Self::Tangent { p, circle } => Self::Tangent {
                p: pt(p)?,
                circle: circ(circle)?,
            },
        })
    }
}

impl MathFigure {
    /// Record that `point` is computed by `how` from now on.
    pub fn derive(&mut self, point: PointId, how: Derivation) {
        self.derived.retain(|d| d.point != point);
        self.derived.push(Derived { point, how });
    }

    /// Record `point` as fixed relative to the base AB (square, regular polygon, special triangle).
    pub fn derive_affine(&mut self, point: PointId, a: PointId, b: PointId) {
        let (Some(p), Some(pa), Some(pb)) = (
            self.points.get(&point).map(|p| p.pos),
            self.points.get(&a).map(|p| p.pos),
            self.points.get(&b).map(|p| p.pos),
        ) else {
            return;
        };
        let d = pb - pa;
        let len_sq = d.length_squared();
        if len_sq < 1e-9 {
            return;
        }
        let (u, v) = ((p - pa).dot(d) / len_sq, (p - pa).dot(perp(d)) / len_sq);
        self.derive(point, Derivation::Affine { a, b, u, v });
    }

    /// Whether `point` is computed from other points (so dragging it has no effect).
    pub fn is_derived(&self, point: PointId) -> bool {
        self.derived.iter().any(|d| d.point == point)
    }

    /// Circle center and radius; the radius follows the through point when there is one.
    fn circle_now(&self, id: CircleId) -> Option<(Vector2, f64)> {
        let c = self.circles.get(&id)?;
        let center = self.points.get(&c.center)?.pos;
        let r = c
            .through_point
            .and_then(|t| self.points.get(&t))
            .map_or(c.radius, |t| (t.pos - center).length());
        Some((center, r))
    }

    /// Recompute all dependent points and the circle radii, e.g. after a point was dragged.
    /// Rules whose points or circles were deleted are dropped.
    pub fn update_derived(&mut self) {
        self.derived.retain(|d| {
            self.points.contains_key(&d.point)
                && d.how.inputs().iter().all(|p| self.points.contains_key(p))
                && d.how.circles().iter().all(|c| self.circles.contains_key(c))
        });
        for i in 0..self.derived.len() {
            let Derived { point, how } = self.derived[i];
            let Some(old) = self.points.get(&point).map(|p| p.pos) else {
                continue;
            };
            let pos = |id: PointId| self.points[&id].pos;
            let nearest = |cands: Vec<Vector2>| {
                cands
                    .into_iter()
                    .min_by(|x, y| (*x - old).length().total_cmp(&(*y - old).length()))
            };
            let new = match how {
                Derivation::Midpoint { a, b } => Some((pos(a) + pos(b)) * 0.5),
                Derivation::Foot { p, a, b } => {
                    let (pa, d) = (pos(a), pos(b) - pos(a));
                    (d.length_squared() > 1e-9)
                        .then(|| pa + d * ((pos(p) - pa).dot(d) / d.length_squared()))
                }
                Derivation::BisectorFoot { a, b, c } => {
                    Triangle::new(pos(a), pos(b), pos(c)).bisector_foot_a()
                }
                Derivation::Center { kind, a, b, c } => {
                    let t = Triangle::new(pos(a), pos(b), pos(c));
                    match kind {
                        TriangleCenterType::Centroid => Some(t.centroid()),
                        TriangleCenterType::Orthocenter => t.orthocenter(),
                        TriangleCenterType::Circumcenter => t.circumcenter(),
                        TriangleCenterType::Incenter => t.incenter(),
                    }
                }
                Derivation::Affine { a, b, u, v } => {
                    let d = pos(b) - pos(a);
                    Some(pos(a) + d * u + perp(d) * v)
                }
                Derivation::LineLine { a, b, c, d } => {
                    line_line_intersection(pos(a), pos(b), pos(c), pos(d))
                }
                Derivation::LineCircle { a, b, circle } => self
                    .circle_now(circle)
                    .and_then(|(c, r)| nearest(line_circle_intersection(pos(a), pos(b), c, r))),
                Derivation::CircleCircle { c1, c2 } => {
                    match (self.circle_now(c1), self.circle_now(c2)) {
                        (Some((p1, r1)), Some((p2, r2))) => {
                            nearest(circle_circle_intersection(p1, r1, p2, r2))
                        }
                        _ => None,
                    }
                }
                Derivation::Tangent { p, circle } => self
                    .circle_now(circle)
                    .and_then(|(c, r)| nearest(tangents_from_point_to_circle(pos(p), c, r))),
            };
            // No solution right now (parallel lines, circles apart): the point waits where it was
            if let Some(new) = new.filter(|v| v.is_finite())
                && let Some(p) = self.points.get_mut(&point)
            {
                p.pos = new;
            }
        }
        let radii: Vec<(CircleId, f64)> = self
            .circles
            .keys()
            .filter_map(|&id| Some((id, self.circle_now(id)?.1)))
            .collect();
        for (id, r) in radii {
            self.circles.get_mut(&id).unwrap().radius = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependent_points_follow_a_dragged_vertex() {
        let mut fig = MathFigure::new();
        let (a, b, c, ..) = fig.build_triangle(
            Vector2::new(0.0, 0.0),
            Vector2::new(100.0, 0.0),
            Vector2::new(30.0, 80.0),
            None,
        );
        let (h, _) = fig.add_altitude(c, a, b, Some("H")).unwrap();
        let m = fig.add_point(Vector2::new(50.0, 0.0), Some("M".into()));
        fig.derive(m, Derivation::Midpoint { a, b });
        let o = fig
            .add_triangle_center(a, b, c, TriangleCenterType::Circumcenter, false)
            .unwrap();

        fig.drag_point(b, Vector2::new(200.0, 40.0));
        let p = |id: PointId| fig.points[&id].pos;
        let check = |fig: &MathFigure| {
            let p = |id: PointId| fig.points[&id].pos;
            (p(m) - (p(a) + p(b)) * 0.5).length() < 1e-9
        };
        assert!((p(m) - (p(a) + p(b)) * 0.5).length() < 1e-9);
        // CH stays perpendicular to AB, H on AB
        assert!((p(c) - p(h)).dot(p(b) - p(a)).abs() < 1e-6);
        assert!((p(h) - p(a)).perp_dot(p(b) - p(a)).abs() < 1e-6);
        // The circumcircle still passes through all three vertices
        let circle = fig.circles.values().find(|x| x.center == o).unwrap();
        for v in [a, b, c] {
            assert!(((p(v) - p(o)).length() - circle.radius).abs() < 1e-6);
        }
        // A derived point cannot be dragged away from its rule
        fig.drag_point(m, Vector2::new(-500.0, -500.0));
        assert!(check(&fig));
    }

    #[test]
    fn square_stays_a_square() {
        let mut fig = MathFigure::new();
        let (a, b, c, d) = fig.build_square(Vector2::new(0.0, 0.0), 100.0, None);
        fig.drag_point(b, Vector2::new(60.0, 80.0));
        let p = |id: PointId| fig.points[&id].pos;
        let side = (p(b) - p(a)).length();
        for (x, y) in [(b, c), (c, d), (d, a)] {
            assert!(((p(y) - p(x)).length() - side).abs() < 1e-6);
        }
        assert!((p(b) - p(a)).dot(p(d) - p(a)).abs() < 1e-6);
    }
}
