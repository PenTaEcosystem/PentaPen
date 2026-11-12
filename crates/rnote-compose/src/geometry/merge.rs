//! Drawing more into an existing figure.

use super::MathFigure;
use super::entity::{PointId, RightAngleMark};
use std::collections::HashMap;

/// Points closer than this are the same point.
const SAME_POINT: f64 = 0.5;

impl MathFigure {
    /// Add the points, lines, circles and right angle marks of `other` to this figure.
    /// A point of `other` lying on a point of this figure becomes that point (it keeps its name
    /// and visibility), so a line drawn from A ends up attached to A. Lines and circles this
    /// figure already has are not added twice.
    pub fn absorb(&mut self, other: &MathFigure) {
        let mut map: HashMap<PointId, PointId> = HashMap::new();
        let mut ids: Vec<_> = other.points.keys().copied().collect();
        ids.sort();
        for id in ids {
            let p = &other.points[&id];
            let mine = self.find_point_near(p.pos, SAME_POINT).unwrap_or_else(|| {
                let new = self.add_point(p.pos, p.label.clone());
                let q = self.points.get_mut(&new).unwrap();
                q.visible = p.visible;
                q.fixed = p.fixed;
                new
            });
            map.insert(id, mine);
        }

        let mut lines: Vec<_> = other.lines.values().collect();
        lines.sort_by_key(|l| l.id);
        for l in lines {
            let (Some(&a), Some(&b)) = (map.get(&l.p1), map.get(&l.p2)) else {
                continue;
            };
            let exists = self
                .lines
                .values()
                .any(|m| (m.p1 == a && m.p2 == b) || (m.p1 == b && m.p2 == a));
            if a == b || exists {
                continue;
            }
            let new = self.add_segment(a, b);
            let m = self.lines.get_mut(&new).unwrap();
            m.line_type = l.line_type;
            m.style = l.style;
            m.tick_marks = l.tick_marks;
            m.visible = l.visible;
        }

        let mut circles: Vec<_> = other.circles.values().collect();
        circles.sort_by_key(|c| c.id);
        for c in circles {
            let Some(&center) = map.get(&c.center) else {
                continue;
            };
            let through = c.through_point.and_then(|t| map.get(&t).copied());
            let exists = self
                .circles
                .values()
                .any(|m| m.center == center && (m.radius - c.radius).abs() < SAME_POINT);
            if exists {
                continue;
            }
            let new = match through {
                Some(t) => self.add_circle_through(center, t),
                None => self.add_circle_radius(center, c.radius),
            };
            let m = self.circles.get_mut(&new).unwrap();
            m.radius = c.radius;
            m.style = c.style;
            m.visible = c.visible;
        }

        for ra in &other.right_angles {
            if let (Some(&vertex), Some(&arm1), Some(&arm2)) =
                (map.get(&ra.vertex), map.get(&ra.arm1), map.get(&ra.arm2))
            {
                let mark = RightAngleMark {
                    vertex,
                    arm1,
                    arm2,
                    size: ra.size,
                };
                if !self.right_angles.contains(&mark) {
                    self.right_angles.push(mark);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2d::math::Vector2;

    #[test]
    fn absorbed_lines_attach_to_existing_points() {
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(100.0, 0.0), Some("B".into()));
        fig.add_segment(a, b);

        // A line drawn from A (its start merged: hidden and unnamed) to a new point C
        let mut more = MathFigure::new();
        let s = more.add_point(Vector2::new(0.0, 0.0), None);
        more.points.get_mut(&s).unwrap().visible = false;
        let c = more.add_point(Vector2::new(0.0, 80.0), Some("C".into()));
        more.add_segment(s, c);
        // and a circle around B, plus AB again
        let b2 = more.add_point(Vector2::new(100.0, 0.0), None);
        more.add_circle_radius(b2, 30.0);
        more.add_segment(s, b2);

        fig.absorb(&more);
        assert_eq!(fig.points.len(), 3);
        assert!(fig.points[&a].visible && fig.points[&a].label.as_deref() == Some("A"));
        assert_eq!(fig.lines.len(), 2);
        let c = fig.find_point_by_label("C").unwrap();
        assert!(fig.lines.values().any(|l| l.p1 == a && l.p2 == c));
        assert_eq!(fig.circles.len(), 1);
        assert_eq!(fig.circles.values().next().unwrap().center, b);

        // Absorbing the same again changes nothing
        let before = fig.clone();
        fig.absorb(&more);
        assert_eq!(fig, before);
    }
}
