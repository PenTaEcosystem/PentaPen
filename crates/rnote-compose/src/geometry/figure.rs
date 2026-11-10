use super::construction::{
    circle_circle_intersection, line_circle_intersection, line_line_intersection,
    tangents_from_point_to_circle,
};
use super::entity::{
    CircleId, ConstraintId, GeomCircle, GeomLine, GeomPoint, LineId, LineStyle, LineType, PointId,
    RightAngleMark,
};
use super::solver::{Constraint, ConstraintSolver};
use super::triangle::{Triangle, TriangleCenterType};
use crate::Transformable;
use crate::ext::DPose2Ext;
use crate::shapes::Shapeable;
use kurbo::Shape;
use p2d::bounding_volume::Aabb;
use p2d::glamx::prelude::DPose2;
use p2d::math::Vector2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A mathematical geometric figure with entities (points, lines, circles) and constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename = "mathfigure")]
pub struct MathFigure {
    /// Points in the figure
    pub points: HashMap<PointId, GeomPoint>,
    /// Lines, rays and segments
    pub lines: HashMap<LineId, GeomLine>,
    /// Circles
    pub circles: HashMap<CircleId, GeomCircle>,
    /// Right angle marks at perpendicular corners
    pub right_angles: Vec<RightAngleMark>,
    /// Active geometric constraints
    pub constraints: Vec<Constraint>,
    /// Counter for unique PointId
    pub next_point_id: usize,
    /// Counter for unique LineId
    pub next_line_id: usize,
    /// Counter for unique CircleId
    pub next_circle_id: usize,
    /// Counter for unique ConstraintId
    pub next_constraint_id: usize,
}

impl Default for MathFigure {
    fn default() -> Self {
        Self {
            points: HashMap::new(),
            lines: HashMap::new(),
            circles: HashMap::new(),
            right_angles: Vec::new(),
            constraints: Vec::new(),
            next_point_id: 1,
            next_line_id: 1,
            next_circle_id: 1,
            next_constraint_id: 1,
        }
    }
}

impl MathFigure {
    /// Create a new empty math figure.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a point at (pos.x, pos.y) with optional label.
    pub fn add_point(&mut self, pos: Vector2, label: Option<String>) -> PointId {
        let id = PointId(self.next_point_id);
        self.next_point_id += 1;
        self.points.insert(id, GeomPoint::new(id, pos, label));
        id
    }

    /// Add a line segment between p1 and p2.
    pub fn add_segment(&mut self, p1: PointId, p2: PointId) -> LineId {
        let id = LineId(self.next_line_id);
        self.next_line_id += 1;
        self.lines.insert(id, GeomLine::new_segment(id, p1, p2));
        id
    }

    /// Add an auxiliary (dashed) line between p1 and p2.
    pub fn add_auxiliary_line(&mut self, p1: PointId, p2: PointId, line_type: LineType) -> LineId {
        let id = LineId(self.next_line_id);
        self.next_line_id += 1;
        self.lines
            .insert(id, GeomLine::new_auxiliary(id, p1, p2, line_type));
        id
    }

    /// Add a circle with center and fixed radius.
    pub fn add_circle_radius(&mut self, center: PointId, radius: f64) -> CircleId {
        let id = CircleId(self.next_circle_id);
        self.next_circle_id += 1;
        self.circles
            .insert(id, GeomCircle::new_with_radius(id, center, radius));
        id
    }

    /// Add a circle defined by center and passing through point `through`.
    pub fn add_circle_through(&mut self, center: PointId, through: PointId) -> CircleId {
        let c_pos = self.points.get(&center).map(|p| p.pos).unwrap_or_default();
        let t_pos = self.points.get(&through).map(|p| p.pos).unwrap_or_default();
        let r = (t_pos - c_pos).length();

        let id = CircleId(self.next_circle_id);
        self.next_circle_id += 1;
        self.circles
            .insert(id, GeomCircle::new_through_point(id, center, through, r));
        id
    }

    /// Add a right angle visual glyph at vertex between arm1 and arm2.
    pub fn add_right_angle(&mut self, vertex: PointId, arm1: PointId, arm2: PointId, size: f64) {
        self.right_angles.push(RightAngleMark {
            vertex,
            arm1,
            arm2,
            size,
        });
    }

    /// Add a geometric constraint.
    pub fn add_constraint(&mut self, constraint: Constraint) -> ConstraintId {
        let cid = constraint.id();
        self.constraints.push(constraint);
        cid
    }

    /// Allocate a new ConstraintId.
    pub fn next_constraint_id(&mut self) -> ConstraintId {
        let id = ConstraintId(self.next_constraint_id);
        self.next_constraint_id += 1;
        id
    }

    /// Helper to construct a triangle ABC with 3 sides.
    pub fn build_triangle(
        &mut self,
        a: Vector2,
        b: Vector2,
        c: Vector2,
        labels: Option<[&str; 3]>,
    ) -> (PointId, PointId, PointId, LineId, LineId, LineId) {
        let (la, lb, lc) = match labels {
            Some([x, y, z]) => (
                Some(x.to_string()),
                Some(y.to_string()),
                Some(z.to_string()),
            ),
            None => (
                Some("A".to_string()),
                Some("B".to_string()),
                Some("C".to_string()),
            ),
        };

        let pa = self.add_point(a, la);
        let pb = self.add_point(b, lb);
        let pc = self.add_point(c, lc);

        let lab = self.add_segment(pa, pb);
        let lbc = self.add_segment(pb, pc);
        let lca = self.add_segment(pc, pa);

        (pa, pb, pc, lab, lbc, lca)
    }

    /// Helper: Construct one of the 4 special triangle centers (G, H, O, I) with auxiliary lines.
    pub fn add_triangle_center(
        &mut self,
        a_id: PointId,
        b_id: PointId,
        c_id: PointId,
        center_type: TriangleCenterType,
        add_auxiliary_lines: bool,
    ) -> Option<PointId> {
        let a = self.points.get(&a_id)?.pos;
        let b = self.points.get(&b_id)?.pos;
        let c = self.points.get(&c_id)?.pos;
        let tri = Triangle::new(a, b, c);

        match center_type {
            TriangleCenterType::Centroid => {
                let g_pos = tri.centroid();
                let g_id = self.add_point(g_pos, Some("G".to_string()));
                if add_auxiliary_lines {
                    let m_bc = self.add_point(tri.midpoint_bc(), None);
                    let m_ca = self.add_point(tri.midpoint_ca(), None);
                    let m_ab = self.add_point(tri.midpoint_ab(), None);
                    self.add_auxiliary_line(a_id, m_bc, LineType::Segment);
                    self.add_auxiliary_line(b_id, m_ca, LineType::Segment);
                    self.add_auxiliary_line(c_id, m_ab, LineType::Segment);
                }
                Some(g_id)
            }
            TriangleCenterType::Circumcenter => {
                let o_pos = tri.circumcenter()?;
                let o_id = self.add_point(o_pos, Some("O".to_string()));
                if let Some(r) = tri.circumradius() {
                    let mut circ =
                        GeomCircle::new_with_radius(CircleId(self.next_circle_id), o_id, r);
                    self.next_circle_id += 1;
                    circ.style = LineStyle::Dashed;
                    self.circles.insert(circ.id, circ);
                }
                Some(o_id)
            }
            TriangleCenterType::Orthocenter => {
                let h_pos = tri.orthocenter()?;
                let h_id = self.add_point(h_pos, Some("H".to_string()));
                if add_auxiliary_lines {
                    if let Some(fa) = tri.altitude_foot_a() {
                        let fa_id = self.add_point(fa, None);
                        self.add_auxiliary_line(a_id, fa_id, LineType::Segment);
                        self.add_right_angle(fa_id, a_id, c_id, 8.0);
                    }
                    if let Some(fb) = tri.altitude_foot_b() {
                        let fb_id = self.add_point(fb, None);
                        self.add_auxiliary_line(b_id, fb_id, LineType::Segment);
                        self.add_right_angle(fb_id, b_id, a_id, 8.0);
                    }
                }
                Some(h_id)
            }
            TriangleCenterType::Incenter => {
                let i_pos = tri.incenter()?;
                let i_id = self.add_point(i_pos, Some("I".to_string()));
                if let Some(r) = tri.inradius() {
                    let mut circ =
                        GeomCircle::new_with_radius(CircleId(self.next_circle_id), i_id, r);
                    self.next_circle_id += 1;
                    circ.style = LineStyle::Dashed;
                    self.circles.insert(circ.id, circ);
                }
                if add_auxiliary_lines {
                    if let Some(da) = tri.bisector_foot_a() {
                        let da_id = self.add_point(da, None);
                        self.add_auxiliary_line(a_id, da_id, LineType::Segment);
                    }
                    if let Some(db) = tri.bisector_foot_b() {
                        let db_id = self.add_point(db, None);
                        self.add_auxiliary_line(b_id, db_id, LineType::Segment);
                    }
                }
                Some(i_id)
            }
        }
    }

    /// Construct tangents from a point P to a circle.
    pub fn add_tangents_from_point(&mut self, p_id: PointId, circle_id: CircleId) -> Vec<LineId> {
        let p_pos = match self.points.get(&p_id) {
            Some(p) => p.pos,
            None => return Vec::new(),
        };
        let (center_pos, radius) = match self.circles.get(&circle_id) {
            Some(c) => match self.points.get(&c.center) {
                Some(cp) => (cp.pos, c.radius),
                None => return Vec::new(),
            },
            None => return Vec::new(),
        };

        let t_points = tangents_from_point_to_circle(p_pos, center_pos, radius);
        let mut lines = Vec::new();
        for t_pos in t_points {
            let t_id = self.add_point(t_pos, None);
            let l_id = self.add_segment(p_id, t_id);
            lines.push(l_id);
        }
        lines
    }

    /// Helper to construct a Circle with an explicit Center point (with custom label) and a boundary point.
    pub fn build_circle(
        &mut self,
        center_pos: Vector2,
        radius: f64,
        center_label: Option<&str>,
        boundary_label: Option<&str>,
    ) -> (PointId, PointId, CircleId, LineId) {
        let c_label = center_label.unwrap_or("O").to_string();
        let b_label = boundary_label.unwrap_or("M").to_string();

        let center_id = self.add_point(center_pos, Some(c_label));
        let boundary_pos = center_pos + Vector2::new(radius, 0.0);
        let boundary_id = self.add_point(boundary_pos, Some(b_label));

        let circle_id = self.add_circle_through(center_id, boundary_id);
        let radius_line_id = self.add_auxiliary_line(center_id, boundary_id, LineType::Segment);

        (center_id, boundary_id, circle_id, radius_line_id)
    }

    /// Helper to construct a Line segment between two points.
    pub fn build_line(
        &mut self,
        p1_pos: Vector2,
        p2_pos: Vector2,
        p1_label: Option<&str>,
        p2_label: Option<&str>,
    ) -> (PointId, PointId, LineId) {
        let p1_id = self.add_point(p1_pos, p1_label.map(|s| s.to_string()));
        let p2_id = self.add_point(p2_pos, p2_label.map(|s| s.to_string()));
        let line_id = self.add_segment(p1_id, p2_id);
        (p1_id, p2_id, line_id)
    }

    /// Helper to find if a point already exists near a coordinate (within tolerance).
    pub fn find_point_near(&self, pos: Vector2, tolerance: f64) -> Option<PointId> {
        let tol_sq = tolerance * tolerance;
        for (&id, pt) in &self.points {
            if (pt.pos - pos).length_squared() <= tol_sq {
                return Some(id);
            }
        }
        None
    }

    /// Find a point by its label (case-insensitive).
    pub fn find_point_by_label(&self, label: &str) -> Option<PointId> {
        let norm = label.trim().to_uppercase();
        for (&id, pt) in &self.points {
            if let Some(ref l) = pt.label {
                if l.trim().to_uppercase() == norm {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Construct a Rectangle ABCD starting at top-left `p` with `width` and `height`.
    pub fn build_rectangle(
        &mut self,
        p: Vector2,
        width: f64,
        height: f64,
        labels: Option<[&str; 4]>,
    ) -> (PointId, PointId, PointId, PointId) {
        let (la, lb, lc, ld) = match labels {
            Some([a, b, c, d]) => (
                Some(a.to_string()),
                Some(b.to_string()),
                Some(c.to_string()),
                Some(d.to_string()),
            ),
            None => (
                Some("A".to_string()),
                Some("B".to_string()),
                Some("C".to_string()),
                Some("D".to_string()),
            ),
        };

        let pa = self.add_point(p, la);
        let pb = self.add_point(p + Vector2::new(width, 0.0), lb);
        let pc = self.add_point(p + Vector2::new(width, height), lc);
        let pd = self.add_point(p + Vector2::new(0.0, height), ld);

        self.add_segment(pa, pb);
        self.add_segment(pb, pc);
        self.add_segment(pc, pd);
        self.add_segment(pd, pa);

        self.add_right_angle(pa, pb, pd, 8.0);
        self.add_right_angle(pb, pc, pa, 8.0);
        self.add_right_angle(pc, pd, pb, 8.0);
        self.add_right_angle(pd, pa, pc, 8.0);

        (pa, pb, pc, pd)
    }

    /// Construct a Square ABCD starting at top-left `p` with `side`.
    pub fn build_square(
        &mut self,
        p: Vector2,
        side: f64,
        labels: Option<[&str; 4]>,
    ) -> (PointId, PointId, PointId, PointId) {
        self.build_rectangle(p, side, side, labels)
    }

    /// Add an altitude from vertex A onto side BC of triangle ABC.
    pub fn add_altitude(
        &mut self,
        a_id: PointId,
        b_id: PointId,
        c_id: PointId,
        foot_label: Option<&str>,
    ) -> Option<(PointId, LineId)> {
        let a = self.points.get(&a_id)?.pos;
        let b = self.points.get(&b_id)?.pos;
        let c = self.points.get(&c_id)?.pos;
        let tri = Triangle::new(a, b, c);
        let foot_pos = tri.altitude_foot_a()?;

        let label = foot_label.unwrap_or("H").to_string();
        let foot_id = self.add_point(foot_pos, Some(label));
        let line_id = self.add_auxiliary_line(a_id, foot_id, LineType::Segment);
        self.add_right_angle(foot_id, a_id, c_id, 8.0);

        Some((foot_id, line_id))
    }

    /// Add a median from vertex A to midpoint of BC.
    pub fn add_median(
        &mut self,
        a_id: PointId,
        b_id: PointId,
        c_id: PointId,
        midpoint_label: Option<&str>,
    ) -> Option<(PointId, LineId)> {
        let a = self.points.get(&a_id)?.pos;
        let b = self.points.get(&b_id)?.pos;
        let c = self.points.get(&c_id)?.pos;
        let tri = Triangle::new(a, b, c);
        let m_pos = tri.midpoint_bc();

        let label = midpoint_label.unwrap_or("M").to_string();
        let m_id = self.add_point(m_pos, Some(label));
        let line_id = self.add_auxiliary_line(a_id, m_id, LineType::Segment);

        Some((m_id, line_id))
    }

    /// Add an interior angle bisector from vertex A to side BC.
    pub fn add_angle_bisector(
        &mut self,
        a_id: PointId,
        b_id: PointId,
        c_id: PointId,
        foot_label: Option<&str>,
    ) -> Option<(PointId, LineId)> {
        let a = self.points.get(&a_id)?.pos;
        let b = self.points.get(&b_id)?.pos;
        let c = self.points.get(&c_id)?.pos;
        let tri = Triangle::new(a, b, c);
        let d_pos = tri.bisector_foot_a()?;

        let label = foot_label.unwrap_or("D").to_string();
        let d_id = self.add_point(d_pos, Some(label));
        let line_id = self.add_auxiliary_line(a_id, d_id, LineType::Segment);

        Some((d_id, line_id))
    }

    /// Automatically compute all intersections (line-line, line-circle, circle-circle)
    /// and add new labeled intersection points to the figure.
    pub fn auto_detect_intersections(&mut self) -> Vec<PointId> {
        let mut new_points = Vec::new();
        let tolerance = 0.5;

        // 1. Line - Line intersections
        let line_pairs: Vec<(LineId, PointId, PointId, Vector2, Vector2)> = self
            .lines
            .iter()
            .filter_map(|(&id, line)| {
                let p1 = self.points.get(&line.p1)?.pos;
                let p2 = self.points.get(&line.p2)?.pos;
                Some((id, line.p1, line.p2, p1, p2))
            })
            .collect();

        let num_lines = line_pairs.len();
        for i in 0..num_lines {
            for j in (i + 1)..num_lines {
                let (_, p1_id, p2_id, p1, p2) = line_pairs[i];
                let (_, q1_id, q2_id, q1, q2) = line_pairs[j];

                // Skip if sharing an endpoint
                if p1_id == q1_id || p1_id == q2_id || p2_id == q1_id || p2_id == q2_id {
                    continue;
                }

                if let Some(inter) = line_line_intersection(p1, p2, q1, q2) {
                    let d1 = (p2 - p1).length();
                    let d2 = (q2 - q1).length();
                    let t1 = (inter - p1).dot(p2 - p1) / (d1 * d1);
                    let t2 = (inter - q1).dot(q2 - q1) / (d2 * d2);

                    if t1 >= -0.01 && t1 <= 1.01 && t2 >= -0.01 && t2 <= 1.01 {
                        if self.find_point_near(inter, tolerance).is_none() {
                            let pt_id = self.add_point(inter, None);
                            new_points.push(pt_id);
                        }
                    }
                }
            }
        }

        // 2. Line - Circle intersections
        let circles_data: Vec<(CircleId, Vector2, f64)> = self
            .circles
            .iter()
            .filter_map(|(&id, circ)| {
                let c_pos = self.points.get(&circ.center)?.pos;
                Some((id, c_pos, circ.radius))
            })
            .collect();

        for &(_, _, _, p1, p2) in &line_pairs {
            for &(_, c_pos, radius) in &circles_data {
                let inters = line_circle_intersection(p1, p2, c_pos, radius);
                for inter in inters {
                    let d1 = (p2 - p1).length();
                    let t = (inter - p1).dot(p2 - p1) / (d1 * d1);
                    if t >= -0.05 && t <= 1.05 {
                        if self.find_point_near(inter, tolerance).is_none() {
                            let pt_id = self.add_point(inter, None);
                            new_points.push(pt_id);
                        }
                    }
                }
            }
        }

        // 3. Circle - Circle intersections
        let num_circ = circles_data.len();
        for i in 0..num_circ {
            for j in (i + 1)..num_circ {
                let (_, c1, r1) = circles_data[i];
                let (_, c2, r2) = circles_data[j];
                let inters = circle_circle_intersection(c1, r1, c2, r2);
                for inter in inters {
                    if self.find_point_near(inter, tolerance).is_none() {
                        let pt_id = self.add_point(inter, None);
                        new_points.push(pt_id);
                    }
                }
            }
        }

        new_points
    }

    /// Automatically find any triangle formed by lines and add 4 classical centers (Orthocenter H, etc.)
    pub fn auto_detect_triangles_and_centers(&mut self, add_auxiliary_lines: bool) {
        // Collect adjacency graph of segments
        let mut adj: HashMap<PointId, Vec<PointId>> = HashMap::new();
        for line in self.lines.values() {
            adj.entry(line.p1).or_default().push(line.p2);
            adj.entry(line.p2).or_default().push(line.p1);
        }

        let pts: Vec<PointId> = adj.keys().copied().collect();
        let n = pts.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let p1 = pts[i];
                let p2 = pts[j];
                if !adj.get(&p1).map_or(false, |v| v.contains(&p2)) {
                    continue;
                }
                for k in (j + 1)..n {
                    let p3 = pts[k];
                    if adj.get(&p1).map_or(false, |v| v.contains(&p3))
                        && adj.get(&p2).map_or(false, |v| v.contains(&p3))
                    {
                        // Found a triangle p1-p2-p3
                        self.add_triangle_center(
                            p1,
                            p2,
                            p3,
                            TriangleCenterType::Centroid,
                            add_auxiliary_lines,
                        );
                        self.add_triangle_center(
                            p1,
                            p2,
                            p3,
                            TriangleCenterType::Circumcenter,
                            add_auxiliary_lines,
                        );
                        self.add_triangle_center(
                            p1,
                            p2,
                            p3,
                            TriangleCenterType::Orthocenter,
                            add_auxiliary_lines,
                        );
                        self.add_triangle_center(
                            p1,
                            p2,
                            p3,
                            TriangleCenterType::Incenter,
                            add_auxiliary_lines,
                        );
                        return; // Add for the first triangle found
                    }
                }
            }
        }
    }

    /// Update coordinates of a dragged point and solve all constraints in real-time.
    pub fn drag_point(&mut self, point_id: PointId, target: Vector2) -> bool {
        if !self.points.contains_key(&point_id) {
            return false;
        }

        // Build position map and update dragged point position to target
        let mut positions: HashMap<PointId, Vector2> =
            self.points.iter().map(|(&id, pt)| (id, pt.pos)).collect();
        if let Some(p) = positions.get_mut(&point_id) {
            *p = target;
        }

        let mut solver = ConstraintSolver::new();
        for c in &self.constraints {
            solver.add_constraint(c.clone());
        }

        // Fixed points: points that are marked fixed
        let fixed: Vec<PointId> = self
            .points
            .iter()
            .filter(|(id, pt)| pt.fixed && **id != point_id)
            .map(|(&id, _)| id)
            .collect();

        let converged = solver.solve(&mut positions, &fixed, 25, 1e-4);

        // Update positions
        for (&id, &new_pos) in &positions {
            if let Some(pt) = self.points.get_mut(&id) {
                pt.pos = new_pos;
            }
        }

        // Update circles through points
        for circle in self.circles.values_mut() {
            if let Some(tp_id) = circle.through_point {
                if let (Some(cp), Some(tp)) =
                    (self.points.get(&circle.center), self.points.get(&tp_id))
                {
                    circle.radius = (tp.pos - cp.pos).length();
                }
            }
        }

        converged
    }
}

impl Transformable for MathFigure {
    fn translate(&mut self, offset: Vector2) {
        for pt in self.points.values_mut() {
            pt.pos += offset;
        }
    }

    fn rotate(&mut self, angle: f64, center: Vector2) {
        let pose = DPose2::from_rotation_wrt_center(angle, center);
        for pt in self.points.values_mut() {
            pt.pos = pose.transform_point(pt.pos);
        }
    }

    fn scale(&mut self, scale: Vector2) {
        for pt in self.points.values_mut() {
            pt.pos *= scale;
        }
        for circ in self.circles.values_mut() {
            circ.radius *= (scale.x.abs() + scale.y.abs()) * 0.5;
        }
    }
}

impl Shapeable for MathFigure {
    fn bounds(&self) -> Aabb {
        if self.points.is_empty() {
            return Aabb::new(Vector2::new(0.0, 0.0), Vector2::new(0.0, 0.0));
        }

        let first = self.points.values().next().unwrap().pos;
        let mut aabb = Aabb::new(first, first);

        for pt in self.points.values() {
            aabb.take_point(pt.pos);
        }

        for circ in self.circles.values() {
            if let Some(c) = self.points.get(&circ.center) {
                let r = circ.radius;
                aabb.take_point(c.pos + Vector2::new(r, r));
                aabb.take_point(c.pos - Vector2::new(r, r));
            }
        }

        aabb
    }

    fn hitboxes(&self) -> Vec<Aabb> {
        let mut hitboxes = Vec::new();
        // Points hitboxes (small radius around each point for selecting/dragging vertices)
        const PT_RADIUS: f64 = 6.0;
        for pt in self.points.values() {
            let offset = Vector2::new(PT_RADIUS, PT_RADIUS);
            hitboxes.push(Aabb::new(pt.pos - offset, pt.pos + offset));
        }

        // Lines hitboxes
        for line in self.lines.values() {
            if let (Some(p1), Some(p2)) = (self.points.get(&line.p1), self.points.get(&line.p2)) {
                let mut box_line = Aabb::new(p1.pos, p1.pos);
                box_line.take_point(p2.pos);
                hitboxes.push(box_line);
            }
        }

        // Circles hitboxes
        for circ in self.circles.values() {
            if let Some(c) = self.points.get(&circ.center) {
                let r = circ.radius;
                hitboxes.push(Aabb::new(
                    c.pos - Vector2::new(r, r),
                    c.pos + Vector2::new(r, r),
                ));
            }
        }

        if hitboxes.is_empty() {
            hitboxes.push(self.bounds());
        }

        hitboxes
    }

    fn outline_path(&self) -> kurbo::BezPath {
        let mut path = kurbo::BezPath::new();

        // Add line segments
        for line in self.lines.values() {
            if let (Some(p1), Some(p2)) = (self.points.get(&line.p1), self.points.get(&line.p2)) {
                path.move_to(kurbo::Point::new(p1.pos.x, p1.pos.y));
                path.line_to(kurbo::Point::new(p2.pos.x, p2.pos.y));
            }
        }

        // Add circles
        for circ in self.circles.values() {
            if let Some(c) = self.points.get(&circ.center) {
                let circle = kurbo::Circle::new(kurbo::Point::new(c.pos.x, c.pos.y), circ.radius);
                path.extend(circle.to_path(0.1));
            }
        }

        // Add right angle marks
        for ra in &self.right_angles {
            if let (Some(v), Some(a1), Some(a2)) = (
                self.points.get(&ra.vertex),
                self.points.get(&ra.arm1),
                self.points.get(&ra.arm2),
            ) {
                let d1 = (a1.pos - v.pos).normalize();
                let d2 = (a2.pos - v.pos).normalize();
                let corner1 = v.pos + d1 * ra.size;
                let corner_mid = v.pos + (d1 + d2) * ra.size;
                let corner2 = v.pos + d2 * ra.size;

                path.move_to(kurbo::Point::new(corner1.x, corner1.y));
                path.line_to(kurbo::Point::new(corner_mid.x, corner_mid.y));
                path.line_to(kurbo::Point::new(corner2.x, corner2.y));
            }
        }

        path
    }
}
