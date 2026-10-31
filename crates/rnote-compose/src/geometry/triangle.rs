// Triangle geometry and the 4 special centers (Centroid, Orthocenter, Circumcenter, Incenter)
use p2d::math::Vector2;

/// The 4 classical triangle centers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriangleCenterType {
    /// Trọng tâm G (Centroid - intersection of medians)
    Centroid,
    /// Trực tâm H (Orthocenter - intersection of altitudes)
    Orthocenter,
    /// Tâm đường tròn ngoại tiếp O (Circumcenter - intersection of perpendicular bisectors)
    Circumcenter,
    /// Tâm đường tròn nội tiếp I (Incenter - intersection of interior angle bisectors)
    Incenter,
}

/// Geometric properties and calculations of a triangle ABC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    /// Vertex A
    pub a: Vector2,
    /// Vertex B
    pub b: Vector2,
    /// Vertex C
    pub c: Vector2,
}

impl Triangle {
    /// Create a new triangle from 3 vertices.
    pub fn new(a: Vector2, b: Vector2, c: Vector2) -> Self {
        Self { a, b, c }
    }

    /// Length of side a (BC).
    pub fn side_a(&self) -> f64 {
        (self.c - self.b).length()
    }

    /// Length of side b (CA).
    pub fn side_b(&self) -> f64 {
        (self.a - self.c).length()
    }

    /// Length of side c (AB).
    pub fn side_c(&self) -> f64 {
        (self.b - self.a).length()
    }

    /// Semi-perimeter p = (a + b + c) / 2.
    pub fn semi_perimeter(&self) -> f64 {
        (self.side_a() + self.side_b() + self.side_c()) / 2.0
    }

    /// Signed area using cross product: 0.5 * ((xB - xA)(yC - yA) - (xC - xA)(yB - yA)).
    pub fn signed_area(&self) -> f64 {
        0.5 * ((self.b.x - self.a.x) * (self.c.y - self.a.y) - (self.c.x - self.a.x) * (self.b.y - self.a.y))
    }

    /// Unsigned Area of triangle ABC.
    pub fn area(&self) -> f64 {
        self.signed_area().abs()
    }

    /// Whether the 3 vertices are collinear (degenerate triangle).
    pub fn is_degenerate(&self) -> bool {
        self.area() < 1e-9
    }

    /// Trọng tâm G (Centroid): G = (A + B + C) / 3.
    pub fn centroid(&self) -> Vector2 {
        (self.a + self.b + self.c) / 3.0
    }

    /// Tâm đường tròn ngoại tiếp O (Circumcenter).
    /// Returns None if vertices are collinear.
    pub fn circumcenter(&self) -> Option<Vector2> {
        let d = 4.0 * self.signed_area();
        if d.abs() < 1e-11 {
            return None;
        }

        let a2 = self.a.x * self.a.x + self.a.y * self.a.y;
        let b2 = self.b.x * self.b.x + self.b.y * self.b.y;
        let c2 = self.c.x * self.c.x + self.c.y * self.c.y;

        let ox = (a2 * (self.b.y - self.c.y) + b2 * (self.c.y - self.a.y) + c2 * (self.a.y - self.b.y)) / d;
        let oy = (a2 * (self.c.x - self.b.x) + b2 * (self.a.x - self.c.x) + c2 * (self.b.x - self.a.x)) / d;

        Some(Vector2::new(ox, oy))
    }

    /// Bán kính đường tròn ngoại tiếp R = abc / (4 * Area).
    pub fn circumradius(&self) -> Option<f64> {
        let area = self.area();
        if area < 1e-11 {
            None
        } else {
            Some((self.side_a() * self.side_b() * self.side_c()) / (4.0 * area))
        }
    }

    /// Trực tâm H (Orthocenter): H = 3G - 2O (Euler relation).
    pub fn orthocenter(&self) -> Option<Vector2> {
        let g = self.centroid();
        let o = self.circumcenter()?;
        Some(3.0 * g - 2.0 * o)
    }

    /// Tâm đường tròn nội tiếp I (Incenter): I = (aA + bB + cC) / (a + b + c).
    pub fn incenter(&self) -> Option<Vector2> {
        let a = self.side_a();
        let b = self.side_b();
        let c = self.side_c();
        let perimeter = a + b + c;
        if perimeter < 1e-11 {
            return None;
        }
        Some((a * self.a + b * self.b + c * self.c) / perimeter)
    }

    /// Bán kính đường tròn nội tiếp r = Area / p.
    pub fn inradius(&self) -> Option<f64> {
        let p = self.semi_perimeter();
        if p < 1e-11 {
            None
        } else {
            Some(self.area() / p)
        }
    }

    /// Midpoint of side BC.
    pub fn midpoint_bc(&self) -> Vector2 {
        (self.b + self.c) * 0.5
    }

    /// Midpoint of side CA.
    pub fn midpoint_ca(&self) -> Vector2 {
        (self.c + self.a) * 0.5
    }

    /// Midpoint of side AB.
    pub fn midpoint_ab(&self) -> Vector2 {
        (self.a + self.b) * 0.5
    }

    /// Foot of altitude from A onto side BC.
    pub fn altitude_foot_a(&self) -> Option<Vector2> {
        let bc = self.c - self.b;
        let len_sq = bc.length_squared();
        if len_sq < 1e-12 {
            return None;
        }
        let t = (self.a - self.b).dot(bc) / len_sq;
        Some(self.b + bc * t)
    }

    /// Foot of altitude from B onto side CA.
    pub fn altitude_foot_b(&self) -> Option<Vector2> {
        let ca = self.a - self.c;
        let len_sq = ca.length_squared();
        if len_sq < 1e-12 {
            return None;
        }
        let t = (self.b - self.c).dot(ca) / len_sq;
        Some(self.c + ca * t)
    }

    /// Foot of altitude from C onto side AB.
    pub fn altitude_foot_c(&self) -> Option<Vector2> {
        let ab = self.b - self.a;
        let len_sq = ab.length_squared();
        if len_sq < 1e-12 {
            return None;
        }
        let t = (self.c - self.a).dot(ab) / len_sq;
        Some(self.a + ab * t)
    }

    /// Foot of internal angle bisector from A onto side BC.
    /// D divides BC in ratio c : b, so D = (b*B + c*C) / (b + c).
    pub fn bisector_foot_a(&self) -> Option<Vector2> {
        let b = self.side_b();
        let c = self.side_c();
        let sum = b + c;
        if sum < 1e-11 {
            return None;
        }
        Some((b * self.b + c * self.c) / sum)
    }

    /// Foot of internal angle bisector from B onto side CA.
    pub fn bisector_foot_b(&self) -> Option<Vector2> {
        let c = self.side_c();
        let a = self.side_a();
        let sum = c + a;
        if sum < 1e-11 {
            return None;
        }
        Some((c * self.c + a * self.a) / sum)
    }

    /// Foot of internal angle bisector from C onto side AB.
    pub fn bisector_foot_c(&self) -> Option<Vector2> {
        let a = self.side_a();
        let b = self.side_b();
        let sum = a + b;
        if sum < 1e-11 {
            return None;
        }
        Some((a * self.a + b * self.b) / sum)
    }
}
