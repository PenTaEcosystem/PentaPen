// Ruler-and-compass geometric constructions (Geo-Aid architecture)
use p2d::math::Vector2;

/// A 2D ray or line parameter representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineDef {
    /// A point on the line
    pub point: Vector2,
    /// Direction unit vector
    pub dir: Vector2,
}

impl LineDef {
    /// Create a line through two points A and B.
    pub fn from_two_points(a: Vector2, b: Vector2) -> Option<Self> {
        let diff = b - a;
        let len = diff.length();
        if len < 1e-11 {
            None
        } else {
            Some(Self {
                point: a,
                dir: diff / len,
            })
        }
    }

    /// Normal unit vector perpendicular to the line (rotated +90 degrees).
    pub fn normal(&self) -> Vector2 {
        Vector2::new(-self.dir.y, self.dir.x)
    }

    /// Project a point P onto this line.
    pub fn project(&self, p: Vector2) -> Vector2 {
        let t = (p - self.point).dot(self.dir);
        self.point + self.dir * t
    }

    /// Distance from a point P to this line.
    pub fn distance_to_point(&self, p: Vector2) -> f64 {
        ((p - self.point).dot(self.normal())).abs()
    }
}

/// Midpoint between two points.
pub fn midpoint(a: Vector2, b: Vector2) -> Vector2 {
    (a + b) * 0.5
}

/// Perpendicular bisector of segment AB.
/// Returns a LineDef passing through the midpoint with direction perpendicular to AB.
pub fn perpendicular_bisector(a: Vector2, b: Vector2) -> Option<LineDef> {
    let diff = b - a;
    let len = diff.length();
    if len < 1e-11 {
        return None;
    }
    let mid = midpoint(a, b);
    let dir = Vector2::new(-diff.y, diff.x) / len;
    Some(LineDef { point: mid, dir })
}

/// Parallel line passing through point P with direction of line AB.
pub fn parallel_line(p: Vector2, a: Vector2, b: Vector2) -> Option<LineDef> {
    let base = LineDef::from_two_points(a, b)?;
    Some(LineDef {
        point: p,
        dir: base.dir,
    })
}

/// Perpendicular line passing through point P to line AB.
pub fn perpendicular_line(p: Vector2, a: Vector2, b: Vector2) -> Option<LineDef> {
    let base = LineDef::from_two_points(a, b)?;
    Some(LineDef {
        point: p,
        dir: base.normal(),
    })
}

/// Interior angle bisector of angle BAC (originating at A).
pub fn angle_bisector(b: Vector2, a: Vector2, c: Vector2) -> Option<LineDef> {
    let v1 = b - a;
    let v2 = c - a;
    let len1 = v1.length();
    let len2 = v2.length();
    if len1 < 1e-11 || len2 < 1e-11 {
        return None;
    }
    let u1 = v1 / len1;
    let u2 = v2 / len2;
    let sum = u1 + u2;
    let sum_len = sum.length();
    if sum_len < 1e-11 {
        // Arms are pointing in opposite directions (180 deg)
        let dir = Vector2::new(-u1.y, u1.x);
        Some(LineDef { point: a, dir })
    } else {
        Some(LineDef {
            point: a,
            dir: sum / sum_len,
        })
    }
}

/// Intersection point between two infinite lines (A1-A2) and (B1-B2).
pub fn line_line_intersection(a1: Vector2, a2: Vector2, b1: Vector2, b2: Vector2) -> Option<Vector2> {
    let da = a2 - a1;
    let db = b2 - b1;
    let denom = da.x * db.y - da.y * db.x;
    if denom.abs() < 1e-11 {
        return None; // Parallel or collinear
    }
    let d = b1 - a1;
    let t = (d.x * db.y - d.y * db.x) / denom;
    Some(a1 + da * t)
}

/// Tangent points on a circle (center, radius) from an external point P.
/// Returns 0 points if P is inside the circle, 1 point if P is on the circle, or 2 points if outside.
pub fn tangents_from_point_to_circle(p: Vector2, center: Vector2, radius: f64) -> Vec<Vector2> {
    if radius <= 0.0 {
        return Vec::new();
    }
    let d_vec = p - center;
    let d = d_vec.length();

    if d < radius - 1e-9 {
        // Point is strictly inside circle
        Vec::new()
    } else if (d - radius).abs() <= 1e-9 {
        // Point is on circle: single tangent point at P
        vec![p]
    } else {
        // Point is outside circle: 2 tangent points
        let alpha = d_vec.y.atan2(d_vec.x);
        let phi = (radius / d).acos();

        let t1 = center + Vector2::new((alpha + phi).cos(), (alpha + phi).sin()) * radius;
        let t2 = center + Vector2::new((alpha - phi).cos(), (alpha - phi).sin()) * radius;
        vec![t1, t2]
    }
}

/// Intersection points between an infinite line (A1-A2) and a circle (center, radius).
pub fn line_circle_intersection(a1: Vector2, a2: Vector2, center: Vector2, radius: f64) -> Vec<Vector2> {
    if radius <= 0.0 {
        return Vec::new();
    }
    let line = match LineDef::from_two_points(a1, a2) {
        Some(l) => l,
        None => return Vec::new(),
    };

    let proj = line.project(center);
    let h = (center - proj).length();

    if h > radius + 1e-9 {
        Vec::new()
    } else if (h - radius).abs() <= 1e-9 {
        vec![proj]
    } else {
        let half_chord = (radius * radius - h * h).sqrt();
        vec![proj + line.dir * half_chord, proj - line.dir * half_chord]
    }
}

/// Intersection points between two circles C1(center1, r1) and C2(center2, r2).
pub fn circle_circle_intersection(
    center1: Vector2,
    r1: f64,
    center2: Vector2,
    r2: f64,
) -> Vec<Vector2> {
    if r1 <= 0.0 || r2 <= 0.0 {
        return Vec::new();
    }
    let diff = center2 - center1;
    let d = diff.length();

    if d < 1e-11 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
        // Concentric, too far apart, or one contains the other
        return Vec::new();
    }

    if (d - (r1 + r2)).abs() <= 1e-9 {
        // Externally tangent at 1 point
        return vec![center1 + diff * (r1 / d)];
    }
    if (d - (r1 - r2).abs()).abs() <= 1e-9 {
        // Internally tangent at 1 point
        return vec![center1 + diff * (r1 / d)];
    }

    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h_sq = r1 * r1 - a * a;
    let h = if h_sq > 0.0 { h_sq.sqrt() } else { 0.0 };

    let p = center1 + diff * (a / d);
    let normal = Vector2::new(-diff.y, diff.x) * (h / d);

    vec![p + normal, p - normal]
}
