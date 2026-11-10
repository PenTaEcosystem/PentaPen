// Real-time 2D Geometric Constraint Solver (SolveSpace slvs architecture)
use super::entity::{ConstraintId, PointId};
use p2d::math::Vector2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Geometric constraint definitions.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    /// P1 and P2 coincide (P1 = P2)
    Coincident {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
    },
    /// Fixed distance between P1 and P2: |P1 - P2| = distance
    Distance {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
        distance: f64,
    },
    /// Equal lengths of segments (P1-P2) and (P3-P4): |P1 - P2| - |P3 - P4| = 0
    EqualLength {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
        p3: PointId,
        p4: PointId,
    },
    /// Lines (P1-P2) and (P3-P4) are perpendicular: (P2 - P1) . (P4 - P3) = 0
    Perpendicular {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
        p3: PointId,
        p4: PointId,
    },
    /// Lines (P1-P2) and (P3-P4) are parallel: (P2 - P1) x (P4 - P3) = 0
    Parallel {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
        p3: PointId,
        p4: PointId,
    },
    /// Point P lies on the infinite line passing through A and B
    PointOnLine {
        id: ConstraintId,
        p: PointId,
        a: PointId,
        b: PointId,
    },
    /// Point P lies on circle with center C and given radius: |P - C| - radius = 0
    PointOnCircle {
        id: ConstraintId,
        p: PointId,
        center: PointId,
        radius: f64,
    },
    /// Line (P1-P2) is horizontal: y1 - y2 = 0
    Horizontal {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
    },
    /// Line (P1-P2) is vertical: x1 - x2 = 0
    Vertical {
        id: ConstraintId,
        p1: PointId,
        p2: PointId,
    },
    /// Line (A-B) is tangent to circle (Center, Radius)
    TangentLineCircle {
        id: ConstraintId,
        a: PointId,
        b: PointId,
        center: PointId,
        radius: f64,
    },
    /// Pin point P to an exact target position (soft guide used during interactive dragging)
    PinPoint {
        id: ConstraintId,
        point: PointId,
        target: Vector2,
        weight: f64,
    },
}

impl Constraint {
    /// Get the constraint ID.
    pub fn id(&self) -> ConstraintId {
        match *self {
            Self::Coincident { id, .. }
            | Self::Distance { id, .. }
            | Self::EqualLength { id, .. }
            | Self::Perpendicular { id, .. }
            | Self::Parallel { id, .. }
            | Self::PointOnLine { id, .. }
            | Self::PointOnCircle { id, .. }
            | Self::Horizontal { id, .. }
            | Self::Vertical { id, .. }
            | Self::TangentLineCircle { id, .. }
            | Self::PinPoint { id, .. } => id,
        }
    }

    /// Whether this constraint is a soft guide constraint (like dragging)
    pub fn is_soft(&self) -> bool {
        matches!(self, Self::PinPoint { .. })
    }
}

/// A 2D non-linear geometric constraint solver system.
#[derive(Debug, Clone, Default)]
pub struct ConstraintSolver {
    constraints: Vec<Constraint>,
}

impl ConstraintSolver {
    /// Create a new solver instance.
    pub fn new() -> Self {
        Self {
            constraints: Vec::new(),
        }
    }

    /// Add a constraint to the solver.
    pub fn add_constraint(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
    }

    /// Clear all constraints.
    pub fn clear(&mut self) {
        self.constraints.clear();
    }

    /// Remove a constraint by ID.
    pub fn remove_constraint(&mut self, id: ConstraintId) {
        self.constraints.retain(|c| c.id() != id);
    }

    /// Solve the system of constraints modifying the provided point positions in-place.
    /// Returns true if converged within tolerance.
    pub fn solve(
        &self,
        positions: &mut HashMap<PointId, Vector2>,
        fixed_points: &[PointId],
        max_iterations: usize,
        tolerance: f64,
    ) -> bool {
        // Collect free (variable) points
        let mut free_points: Vec<PointId> = positions
            .keys()
            .copied()
            .filter(|id| !fixed_points.contains(id))
            .collect();
        free_points.sort();

        let n = free_points.len() * 2;
        if n == 0 || self.constraints.is_empty() {
            return true;
        }

        // Map PointId to index in parameter vector X
        let pt_to_idx: HashMap<PointId, usize> = free_points
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i * 2))
            .collect();

        // Parameter vector X: [x0, y0, x1, y1, ...]
        let mut x = vec![0.0; n];
        for (i, &pid) in free_points.iter().enumerate() {
            let pos = positions[&pid];
            x[i * 2] = pos.x;
            x[i * 2 + 1] = pos.y;
        }

        let mut lambda = 1e-3;

        for _iter in 0..max_iterations {
            // Check hard constraints convergence
            let hard_r = self.evaluate_residuals(&x, &pt_to_idx, positions, false);
            let hard_err: f64 = hard_r.iter().map(|v| v * v).sum();
            if hard_r.is_empty() || hard_err.sqrt() < tolerance {
                // Update final positions
                for (i, &pid) in free_points.iter().enumerate() {
                    positions.insert(pid, Vector2::new(x[i * 2], x[i * 2 + 1]));
                }
                return true;
            }

            // Full residual (including soft constraints for guidance)
            let r = self.evaluate_residuals(&x, &pt_to_idx, positions, true);
            let total_err_sq: f64 = r.iter().map(|v| v * v).sum();

            let m = r.len();
            // Compute Jacobian J (m x n) via numerical central differences
            let eps = 1e-7;
            let mut j = vec![vec![0.0; n]; m];
            for col in 0..n {
                let orig = x[col];
                x[col] = orig + eps;
                let r_plus = self.evaluate_residuals(&x, &pt_to_idx, positions, true);
                x[col] = orig - eps;
                let r_minus = self.evaluate_residuals(&x, &pt_to_idx, positions, true);
                x[col] = orig;

                for row in 0..m {
                    j[row][col] = (r_plus[row] - r_minus[row]) / (2.0 * eps);
                }
            }

            // Normal equations: (J^T * J + lambda * I) * delta = - J^T * r
            let mut a = vec![vec![0.0; n]; n];
            let mut b = vec![0.0; n];

            for i in 0..n {
                for k in 0..m {
                    b[i] -= j[k][i] * r[k];
                    for c in 0..n {
                        a[i][c] += j[k][i] * j[k][c];
                    }
                }
                a[i][i] += lambda;
            }

            // Solve A * delta = B using Gauss-Jordan elimination with partial pivoting
            if let Some(delta) = solve_linear_system(&a, &b) {
                // Trial step
                let mut x_new = vec![0.0; n];
                for i in 0..n {
                    x_new[i] = x[i] + delta[i];
                }
                let r_new = self.evaluate_residuals(&x_new, &pt_to_idx, positions, true);
                let new_err_sq: f64 = r_new.iter().map(|v| v * v).sum();

                if new_err_sq < total_err_sq {
                    x = x_new;
                    lambda = (lambda * 0.5).max(1e-7);
                } else {
                    lambda = (lambda * 4.0).min(1e5);
                }
            } else {
                lambda = (lambda * 10.0).min(1e5);
            }
        }

        // Apply best effort positions
        for (i, &pid) in free_points.iter().enumerate() {
            positions.insert(pid, Vector2::new(x[i * 2], x[i * 2 + 1]));
        }

        let final_hard_r = self.evaluate_residuals(&x, &pt_to_idx, positions, false);
        let final_err: f64 = final_hard_r.iter().map(|v| v * v).sum();
        final_hard_r.is_empty() || final_err.sqrt() < tolerance
    }

    fn get_point_pos(
        pid: PointId,
        x: &[f64],
        pt_to_idx: &HashMap<PointId, usize>,
        all_positions: &HashMap<PointId, Vector2>,
    ) -> Vector2 {
        if let Some(&idx) = pt_to_idx.get(&pid) {
            Vector2::new(x[idx], x[idx + 1])
        } else {
            all_positions[&pid]
        }
    }

    fn evaluate_residuals(
        &self,
        x: &[f64],
        pt_to_idx: &HashMap<PointId, usize>,
        all_positions: &HashMap<PointId, Vector2>,
        include_soft: bool,
    ) -> Vec<f64> {
        let mut r = Vec::new();

        for c in &self.constraints {
            if !include_soft && c.is_soft() {
                continue;
            }

            match *c {
                Constraint::Coincident { p1, p2, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    r.push(v1.x - v2.x);
                    r.push(v1.y - v2.y);
                }
                Constraint::Distance {
                    p1, p2, distance, ..
                } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    let actual_dist = (v2 - v1).length();
                    r.push(actual_dist - distance);
                }
                Constraint::EqualLength { p1, p2, p3, p4, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    let v3 = Self::get_point_pos(p3, x, pt_to_idx, all_positions);
                    let v4 = Self::get_point_pos(p4, x, pt_to_idx, all_positions);
                    let len1 = (v2 - v1).length();
                    let len2 = (v4 - v3).length();
                    r.push(len1 - len2);
                }
                Constraint::Perpendicular { p1, p2, p3, p4, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    let v3 = Self::get_point_pos(p3, x, pt_to_idx, all_positions);
                    let v4 = Self::get_point_pos(p4, x, pt_to_idx, all_positions);
                    let d1 = v2 - v1;
                    let d2 = v4 - v3;
                    let len1 = d1.length().max(1e-9);
                    r.push(d1.dot(d2) / len1);
                }
                Constraint::Parallel { p1, p2, p3, p4, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    let v3 = Self::get_point_pos(p3, x, pt_to_idx, all_positions);
                    let v4 = Self::get_point_pos(p4, x, pt_to_idx, all_positions);
                    let d1 = v2 - v1;
                    let d2 = v4 - v3;
                    let len1 = d1.length().max(1e-9);
                    let cross = d1.x * d2.y - d1.y * d2.x;
                    r.push(cross / len1);
                }
                Constraint::PointOnLine { p, a, b, .. } => {
                    let vp = Self::get_point_pos(p, x, pt_to_idx, all_positions);
                    let va = Self::get_point_pos(a, x, pt_to_idx, all_positions);
                    let vb = Self::get_point_pos(b, x, pt_to_idx, all_positions);
                    let dab = vb - va;
                    let len_ab = dab.length().max(1e-9);
                    let cross = (vp.x - va.x) * dab.y - (vp.y - va.y) * dab.x;
                    r.push(cross / len_ab);
                }
                Constraint::PointOnCircle {
                    p, center, radius, ..
                } => {
                    let vp = Self::get_point_pos(p, x, pt_to_idx, all_positions);
                    let vc = Self::get_point_pos(center, x, pt_to_idx, all_positions);
                    let d = (vp - vc).length();
                    r.push(d - radius);
                }
                Constraint::Horizontal { p1, p2, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    r.push(v1.y - v2.y);
                }
                Constraint::Vertical { p1, p2, .. } => {
                    let v1 = Self::get_point_pos(p1, x, pt_to_idx, all_positions);
                    let v2 = Self::get_point_pos(p2, x, pt_to_idx, all_positions);
                    r.push(v1.x - v2.x);
                }
                Constraint::TangentLineCircle {
                    a,
                    b,
                    center,
                    radius,
                    ..
                } => {
                    let va = Self::get_point_pos(a, x, pt_to_idx, all_positions);
                    let vb = Self::get_point_pos(b, x, pt_to_idx, all_positions);
                    let vc = Self::get_point_pos(center, x, pt_to_idx, all_positions);
                    let dab = vb - va;
                    let len_ab = dab.length().max(1e-9);
                    let dist = ((vc.x - va.x) * dab.y - (vc.y - va.y) * dab.x).abs() / len_ab;
                    r.push(dist - radius);
                }
                Constraint::PinPoint {
                    point,
                    target,
                    weight,
                    ..
                } => {
                    let v = Self::get_point_pos(point, x, pt_to_idx, all_positions);
                    r.push((v.x - target.x) * weight);
                    r.push((v.y - target.y) * weight);
                }
            }
        }

        r
    }
}

/// Solve linear system A * X = B using Gauss-Jordan elimination with partial pivoting.
fn solve_linear_system(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut mat = vec![vec![0.0; n + 1]; n];

    for i in 0..n {
        for j in 0..n {
            mat[i][j] = a[i][j];
        }
        mat[i][n] = b[i];
    }

    for col in 0..n {
        // Find pivot
        let mut max_row = col;
        let mut max_val = mat[col][col].abs();
        for r in (col + 1)..n {
            let val = mat[r][col].abs();
            if val > max_val {
                max_val = val;
                max_row = r;
            }
        }

        if max_val < 1e-12 {
            return None; // Singular or ill-conditioned
        }

        // Swap pivot row
        mat.swap(col, max_row);

        // Normalize pivot row
        let pivot = mat[col][col];
        for c in col..=n {
            mat[col][c] /= pivot;
        }

        // Eliminate column in other rows
        for r in 0..n {
            if r != col {
                let factor = mat[r][col];
                if factor.abs() > 1e-15 {
                    for c in col..=n {
                        mat[r][c] -= factor * mat[col][c];
                    }
                }
            }
        }
    }

    let mut result = vec![0.0; n];
    for i in 0..n {
        result[i] = mat[i][n];
    }
    Some(result)
}
