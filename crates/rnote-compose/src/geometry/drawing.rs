//! Frames of a figure being drawn by hand, element by element (the AI draws this way).

use super::MathFigure;
use super::entity::RightAngleMark;
use std::collections::HashSet;

impl MathFigure {
    /// The figure appearing piece by piece: each new segment grows from its first point in
    /// `line_steps` frames, each new circle appears after its center, right angle marks and the
    /// untouched points come last. Elements of `base` (the figure before an edit, same ids) are
    /// there from the first frame. The last frame is the figure itself.
    pub fn drawing_frames(&self, base: Option<&MathFigure>, line_steps: usize) -> Vec<MathFigure> {
        let line_steps = line_steps.max(1);
        let old_points: HashSet<_> = base
            .map(|b| b.points.keys().copied().collect())
            .unwrap_or_default();
        let old_lines: HashSet<_> = base
            .map(|b| b.lines.keys().copied().collect())
            .unwrap_or_default();
        let old_circles: HashSet<_> = base
            .map(|b| b.circles.keys().copied().collect())
            .unwrap_or_default();
        let old_marks: Vec<RightAngleMark> =
            base.map(|b| b.right_angles.clone()).unwrap_or_default();

        let mut lines: Vec<_> = self
            .lines
            .keys()
            .filter(|id| !old_lines.contains(id))
            .copied()
            .collect();
        lines.sort();
        let mut circles: Vec<_> = self
            .circles
            .keys()
            .filter(|id| !old_circles.contains(id))
            .copied()
            .collect();
        circles.sort();

        // Start: everything new hidden
        let mut cur = self.clone();
        for p in cur.points.values_mut() {
            if !old_points.contains(&p.id) {
                p.visible = false;
            }
        }
        for id in &lines {
            cur.lines.get_mut(id).unwrap().visible = false;
        }
        for id in &circles {
            cur.circles.get_mut(id).unwrap().visible = false;
        }
        cur.right_angles.retain(|m| old_marks.contains(m));

        let show_point = |cur: &mut MathFigure, id| {
            if let (Some(p), Some(orig)) = (cur.points.get_mut(&id), self.points.get(&id)) {
                p.visible = orig.visible;
            }
        };
        let mut frames = vec![cur.clone()];
        for id in lines {
            let l = &self.lines[&id];
            let (Some(a), Some(b)) = (self.points.get(&l.p1), self.points.get(&l.p2)) else {
                continue;
            };
            show_point(&mut cur, l.p1);
            for k in 1..line_steps {
                // A throwaway tip point and segment p1 -> tip, the real line stays hidden meanwhile
                let mut f = cur.clone();
                let t = k as f64 / line_steps as f64;
                let tip = f.add_point(a.pos + (b.pos - a.pos) * t, None);
                let seg = f.add_segment(l.p1, tip);
                let s = f.lines.get_mut(&seg).unwrap();
                s.style = l.style;
                s.line_type = l.line_type;
                frames.push(f);
            }
            cur.lines.get_mut(&id).unwrap().visible = l.visible;
            show_point(&mut cur, l.p2);
            frames.push(cur.clone());
        }
        for id in circles {
            let c = &self.circles[&id];
            show_point(&mut cur, c.center);
            frames.push(cur.clone());
            cur.circles.get_mut(&id).unwrap().visible = c.visible;
            if let Some(t) = c.through_point {
                show_point(&mut cur, t);
            }
            frames.push(cur.clone());
        }
        if frames.last() != Some(self) {
            frames.push(self.clone());
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2d::math::Vector2;

    #[test]
    fn frames_grow_new_elements_and_end_on_the_figure() {
        let mut base = MathFigure::new();
        let a = base.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = base.add_point(Vector2::new(100.0, 0.0), Some("B".into()));
        base.add_segment(a, b);
        let mut fig = base.clone();
        let c = fig.add_point(Vector2::new(0.0, 100.0), Some("C".into()));
        fig.add_segment(b, c);
        fig.add_circle_radius(a, 30.0);

        let frames = fig.drawing_frames(Some(&base), 4);
        // start + 4 frames for BC + 2 for the circle (+ final, equal to the last one)
        assert_eq!(frames.len(), 7);
        let first = &frames[0];
        assert!(first.lines.values().filter(|l| l.visible).count() == 1);
        assert!(!first.points[&c].visible && first.points[&a].visible);
        assert!(first.circles.values().all(|c| !c.visible));
        // growing frames carry one extra tip point
        assert_eq!(frames[1].points.len(), fig.points.len() + 1);
        assert_eq!(frames.last().unwrap(), &fig);

        // No base: the first frame shows nothing
        let frames = fig.drawing_frames(None, 3);
        assert!(frames[0].points.values().all(|p| !p.visible));
        assert_eq!(frames.last().unwrap(), &fig);
    }
}
