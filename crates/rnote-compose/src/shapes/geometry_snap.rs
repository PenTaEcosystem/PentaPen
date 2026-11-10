#![allow(missing_docs)]

use p2d::math::Vector2;

/// Priority for magnetic snapping: lower number = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapPriority {
    /// Line-circle, line-line, or circle-circle intersection / tangent contact (Highest priority)
    Intersection = 1,
    /// Tangent contact point
    Tangent = 2,
    /// Segment or chord midpoint
    Midpoint = 3,
    /// Line endpoints or circle center
    KeyPoint = 4,
    /// Points along the perimeter / curve
    Perimeter = 5,
}

/// The specific geometric nature of the snap point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapKind {
    /// Line-line, line-circle, or circle-circle intersection
    Intersection { p1: Vector2, p2: Option<Vector2> },
    /// Tangent contact point with normal vector
    Tangent { point: Vector2, normal: Vector2 },
    /// Midpoint of line segment AB
    Midpoint {
        point: Vector2,
        a: Vector2,
        b: Vector2,
    },
    /// Chord midpoint (projection of circle center onto line)
    ChordMidpoint { point: Vector2, center: Vector2 },
    /// Circle center or line endpoint
    Point { point: Vector2, label: Option<char> },
    /// Sampled point on curve/perimeter
    OnPerimeter { point: Vector2 },
}

/// A magnetic snap candidate with position, geometric kind, and priority.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapCandidate {
    pub point: Vector2,
    pub kind: SnapKind,
    pub priority: SnapPriority,
}

impl SnapCandidate {
    pub fn new(point: Vector2, kind: SnapKind, priority: SnapPriority) -> Self {
        Self {
            point,
            kind,
            priority,
        }
    }
}

/// Result of line-circle intersection.
#[derive(Debug, Clone, PartialEq)]
pub enum LineCircleResult {
    /// Line does not intersect or touch circle
    Disjoint,
    /// Line is tangent to circle within eps_tangent tolerance
    Tangent(Vector2),
    /// Line intersects circle at two distinct secant points
    Secant(Vector2, Vector2),
}

/// Result of circle-circle intersection.
#[derive(Debug, Clone, PartialEq)]
pub enum CircleCircleResult {
    /// Circles do not intersect or are concentric
    Disjoint,
    /// Circles touch at exactly one tangent point
    Tangent(Vector2),
    /// Circles intersect at two distinct points
    Secant(Vector2, Vector2),
}

/// Compute segment midpoint M = (A + B) / 2.
pub fn compute_segment_midpoint(a: Vector2, b: Vector2) -> Vector2 {
    (a + b) * 0.5
}

/// Compute chord midpoint H: the orthogonal projection of center O onto the line passing through A and B.
pub fn compute_chord_midpoint(center: Vector2, a: Vector2, b: Vector2) -> Vector2 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-12 {
        return a;
    }
    let t = (center - a).dot(ab) / len_sq;
    a + ab * t
}

/// Compute line-circle intersection points with magnetic tangent threshold.
/// If the perpendicular distance d between center and line is within eps_tangent of radius R,
/// it snaps to a single Tangent point at the projected contact point.
pub fn compute_line_circle_intersections(
    a: Vector2,
    b: Vector2,
    center: Vector2,
    radius: f64,
    eps_tangent: f64,
) -> LineCircleResult {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-12 || radius <= 0.0 {
        return LineCircleResult::Disjoint;
    }

    // Foot of perpendicular from center onto line AB
    let t = (center - a).dot(ab) / len_sq;
    let h = a + ab * t;
    let dist = (center - h).length();

    // Check tangent snap condition: |dist - radius| <= eps_tangent
    if (dist - radius).abs() <= eps_tangent {
        // Tangent contact point on circle perimeter
        let contact = if dist > 1e-9 {
            center + (h - center) * (radius / dist)
        } else {
            h
        };
        LineCircleResult::Tangent(contact)
    } else if dist > radius {
        LineCircleResult::Disjoint
    } else {
        // Secant: dist < radius - eps_tangent
        let half_chord = (radius * radius - dist * dist).max(0.0).sqrt();
        let u = ab / ab.length();
        let i1 = h - u * half_chord;
        let i2 = h + u * half_chord;
        LineCircleResult::Secant(i1, i2)
    }
}

/// Compute intersections between two circles (c1, r1) and (c2, r2).
pub fn compute_circle_circle_intersections(
    c1: Vector2,
    r1: f64,
    c2: Vector2,
    r2: f64,
    eps_tangent: f64,
) -> CircleCircleResult {
    let d = (c2 - c1).length();
    if d < 1e-9 || r1 <= 0.0 || r2 <= 0.0 {
        return CircleCircleResult::Disjoint;
    }

    // Tangent externally or internally
    if (d - (r1 + r2)).abs() <= eps_tangent {
        let pt = c1 + (c2 - c1) * (r1 / d);
        return CircleCircleResult::Tangent(pt);
    }
    if (d - (r1 - r2).abs()).abs() <= eps_tangent {
        let sign = if r1 >= r2 { 1.0 } else { -1.0 };
        let pt = c1 + (c2 - c1) * (sign * r1 / d);
        return CircleCircleResult::Tangent(pt);
    }

    if d > r1 + r2 || d < (r1 - r2).abs() {
        return CircleCircleResult::Disjoint;
    }

    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();

    let p2 = c1 + (c2 - c1) * (a / d);
    let n = Vector2::new(-(c2.y - c1.y), c2.x - c1.x) / d;

    let i1 = p2 + n * h;
    let i2 = p2 - n * h;
    CircleCircleResult::Secant(i1, i2)
}

/// Compute line-line intersection point (infinite lines).
pub fn compute_line_line_intersection(
    a1: Vector2,
    b1: Vector2,
    a2: Vector2,
    b2: Vector2,
) -> Option<Vector2> {
    let d1 = b1 - a1;
    let d2 = b2 - a2;
    let cross = d1.x * d2.y - d1.y * d2.x;
    if cross.abs() < 1e-9 {
        return None;
    }
    let dp = a2 - a1;
    let t = (dp.x * d2.y - dp.y * d2.x) / cross;
    Some(a1 + d1 * t)
}

/// Find the highest-priority magnetic snap candidate within `snap_radius` of `cursor`.
/// Priority ordering:
/// Intersection (1) > Tangent (2) > Midpoint / ChordMidpoint (3) > KeyPoint (4) > Perimeter (5).
/// If multiple candidates share the same highest priority, the one closest to cursor is chosen.
pub fn find_magnetic_snap(
    cursor: Vector2,
    candidates: &[SnapCandidate],
    snap_radius: f64,
) -> Option<SnapCandidate> {
    let snap_radius_sq = snap_radius * snap_radius;

    let mut best: Option<(SnapCandidate, f64)> = None;

    for cand in candidates {
        let dist_sq = (cand.point - cursor).length_squared();
        if dist_sq <= snap_radius_sq {
            match best {
                None => {
                    best = Some((*cand, dist_sq));
                }
                Some((current_best, current_dist_sq)) => {
                    if cand.priority < current_best.priority {
                        // Higher priority (smaller enum value) wins unconditionally
                        best = Some((*cand, dist_sq));
                    } else if cand.priority == current_best.priority && dist_sq < current_dist_sq {
                        // Same priority: closer distance wins
                        best = Some((*cand, dist_sq));
                    }
                }
            }
        }
    }

    best.map(|(c, _)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midpoint() {
        let a = Vector2::new(0.0, 0.0);
        let b = Vector2::new(10.0, 20.0);
        let m = compute_segment_midpoint(a, b);
        assert!((m.x - 5.0).abs() < 1e-6);
        assert!((m.y - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_chord_midpoint() {
        let center = Vector2::new(0.0, 5.0);
        let a = Vector2::new(-10.0, 0.0);
        let b = Vector2::new(10.0, 0.0);
        let h = compute_chord_midpoint(center, a, b);
        assert!((h.x - 0.0).abs() < 1e-6);
        assert!((h.y - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_line_circle_secant() {
        let a = Vector2::new(-10.0, 3.0);
        let b = Vector2::new(10.0, 3.0);
        let center = Vector2::new(0.0, 0.0);
        let radius = 5.0;
        let res = compute_line_circle_intersections(a, b, center, radius, 0.1);
        match res {
            LineCircleResult::Secant(i1, i2) => {
                // Circle x^2 + y^2 = 25, y = 3 => x = +-4
                assert!((i1.y - 3.0).abs() < 1e-6);
                assert!((i2.y - 3.0).abs() < 1e-6);
                let x1 = i1.x.min(i2.x);
                let x2 = i1.x.max(i2.x);
                assert!((x1 - (-4.0)).abs() < 1e-6);
                assert!((x2 - 4.0).abs() < 1e-6);
            }
            _ => panic!("Expected secant, got {:?}", res),
        }
    }

    #[test]
    fn test_line_circle_tangent_snap() {
        let a = Vector2::new(-10.0, 5.05);
        let b = Vector2::new(10.0, 5.05);
        let center = Vector2::new(0.0, 0.0);
        let radius = 5.0;
        // dist is 5.05, within eps_tangent = 0.1
        let res = compute_line_circle_intersections(a, b, center, radius, 0.1);
        match res {
            LineCircleResult::Tangent(t) => {
                assert!((t.x - 0.0).abs() < 1e-6);
                assert!((t.y - 5.0).abs() < 1e-6);
            }
            _ => panic!("Expected tangent snap, got {:?}", res),
        }
    }

    #[test]
    fn test_line_circle_disjoint() {
        let a = Vector2::new(-10.0, 6.0);
        let b = Vector2::new(10.0, 6.0);
        let center = Vector2::new(0.0, 0.0);
        let radius = 5.0;
        let res = compute_line_circle_intersections(a, b, center, radius, 0.1);
        assert_eq!(res, LineCircleResult::Disjoint);
    }

    #[test]
    fn test_circle_circle_intersections() {
        let c1 = Vector2::new(0.0, 0.0);
        let r1 = 5.0;
        let c2 = Vector2::new(6.0, 0.0);
        let r2 = 5.0;
        let res = compute_circle_circle_intersections(c1, r1, c2, r2, 0.1);
        match res {
            CircleCircleResult::Secant(i1, i2) => {
                // Symmetrical about y-axis, x = 3, y = +-4
                assert!((i1.x - 3.0).abs() < 1e-6);
                assert!((i2.x - 3.0).abs() < 1e-6);
                assert!((i1.y.abs() - 4.0).abs() < 1e-6);
                assert!((i2.y.abs() - 4.0).abs() < 1e-6);
            }
            _ => panic!("Expected secant, got {:?}", res),
        }
    }

    #[test]
    fn test_magnetic_snap_priority() {
        let cursor = Vector2::new(10.0, 10.0);
        let candidates = vec![
            SnapCandidate::new(
                Vector2::new(10.5, 10.5),
                SnapKind::Point {
                    point: Vector2::new(10.5, 10.5),
                    label: Some('A'),
                },
                SnapPriority::KeyPoint,
            ),
            SnapCandidate::new(
                Vector2::new(12.0, 12.0),
                SnapKind::Intersection {
                    p1: Vector2::new(12.0, 12.0),
                    p2: None,
                },
                SnapPriority::Intersection,
            ),
        ];

        // Even though KeyPoint (dist ~0.7) is closer than Intersection (dist ~2.8),
        // Intersection has higher priority (1 vs 4) and should win!
        let snapped = find_magnetic_snap(cursor, &candidates, 10.0).expect("Should snap");
        assert_eq!(snapped.priority, SnapPriority::Intersection);
    }
}
