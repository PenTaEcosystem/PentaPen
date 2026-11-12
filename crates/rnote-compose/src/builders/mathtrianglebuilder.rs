// MathTriangleBuilder: Interactive builder for triangles with classical centers
use super::buildable::{Buildable, BuilderCreator, BuilderProgress};
use crate::eventresult::EventPropagation;
use crate::geometry::MathFigure;
use crate::penevent::{KeyboardKey, PenEvent, PenState};
use crate::penpath::Element;
use crate::style::{Composer, indicators};
use crate::{Constraints, EventResult, Shape, Style};
use p2d::bounding_volume::{Aabb, BoundingVolume};
use p2d::math::Vector2;
use piet::RenderContext;
use std::time::Instant;

/// The polygon a [MathTriangleBuilder] draws. The drag spans its bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MathShapeKind {
    /// Any triangle
    #[default]
    Triangle,
    /// Right angle at the bottom left vertex
    RightTriangle,
    /// Apex above the middle of the base
    IsoscelesTriangle,
    /// All sides equal
    EquilateralTriangle,
    /// Square
    Square,
    /// Rectangle with vertex points
    Rectangle,
    /// Parallelogram
    Parallelogram,
    /// Rhombus
    Rhombus,
    /// Isosceles trapezoid
    Trapezoid,
    /// Regular pentagon
    Pentagon,
    /// Regular hexagon
    Hexagon,
}

/// Interactive builder for triangles and other polygons made of figure points and segments.
#[derive(Debug, Clone)]
pub struct MathTriangleBuilder {
    start: Vector2,
    current: Vector2,
    labels: Option<[String; 3]>,
    kind: MathShapeKind,
    equilateral: bool,
}

impl BuilderCreator for MathTriangleBuilder {
    fn start(element: Element, _now: Instant) -> Self {
        Self::start_with_labels(element, ["A", "B", "C"].map(String::from))
    }
}

impl MathTriangleBuilder {
    /// Start a triangle whose vertices get the given names.
    pub fn start_with_labels(element: Element, labels: [String; 3]) -> Self {
        Self {
            labels: Some(labels),
            ..Self::start_with_kind(element, MathShapeKind::Triangle)
        }
    }

    /// Start a shape of the given kind; its vertices get no names.
    pub fn start_with_kind(element: Element, kind: MathShapeKind) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
            labels: None,
            kind,
            equilateral: false,
        }
    }

    /// Vertices in drawing order.
    fn vertices(&self) -> Vec<Vector2> {
        let mut lo = self.start.min(self.current);
        let mut hi = self.start.max(self.current);
        // A click or a tiny drag draws a default sized shape around the click
        if (hi - lo).x < 30.0 && (hi - lo).y < 30.0 {
            lo = self.start - Vector2::new(120.0, 90.0);
            hi = self.start + Vector2::new(120.0, 90.0);
        }
        let (w, h) = (hi.x - lo.x, hi.y - lo.y);
        let c = (lo + hi) * 0.5;
        let v = Vector2::new;
        let kind = match self.kind {
            MathShapeKind::Triangle if self.equilateral => MathShapeKind::EquilateralTriangle,
            k => k,
        };
        match kind {
            MathShapeKind::Triangle => vec![v(lo.x + 0.35 * w, lo.y), v(lo.x, hi.y), v(hi.x, hi.y)],
            MathShapeKind::RightTriangle => vec![v(lo.x, lo.y), v(lo.x, hi.y), v(hi.x, hi.y)],
            MathShapeKind::IsoscelesTriangle => vec![v(c.x, lo.y), v(lo.x, hi.y), v(hi.x, hi.y)],
            MathShapeKind::EquilateralTriangle => {
                let side = w.max(40.0);
                let bottom = lo.y + side * 3.0_f64.sqrt() * 0.5;
                vec![
                    v(c.x, lo.y),
                    v(c.x - side * 0.5, bottom),
                    v(c.x + side * 0.5, bottom),
                ]
            }
            MathShapeKind::Square => {
                // Grows from the press point in the drag direction
                let s = w.max(h);
                let x0 = if self.current.x >= self.start.x || w < 30.0 {
                    lo.x
                } else {
                    hi.x - s
                };
                let y0 = if self.current.y >= self.start.y || h < 30.0 {
                    lo.y
                } else {
                    hi.y - s
                };
                vec![v(x0, y0), v(x0 + s, y0), v(x0 + s, y0 + s), v(x0, y0 + s)]
            }
            MathShapeKind::Rectangle => {
                vec![v(lo.x, lo.y), v(hi.x, lo.y), v(hi.x, hi.y), v(lo.x, hi.y)]
            }
            MathShapeKind::Parallelogram => {
                let o = 0.25 * w;
                vec![
                    v(lo.x + o, lo.y),
                    v(hi.x, lo.y),
                    v(hi.x - o, hi.y),
                    v(lo.x, hi.y),
                ]
            }
            MathShapeKind::Rhombus => vec![v(c.x, lo.y), v(hi.x, c.y), v(c.x, hi.y), v(lo.x, c.y)],
            MathShapeKind::Trapezoid => {
                let o = 0.25 * w;
                vec![
                    v(lo.x + o, lo.y),
                    v(hi.x - o, lo.y),
                    v(hi.x, hi.y),
                    v(lo.x, hi.y),
                ]
            }
            MathShapeKind::Pentagon | MathShapeKind::Hexagon => {
                let n = if kind == MathShapeKind::Pentagon {
                    5
                } else {
                    6
                };
                let r = 0.5 * w.min(h);
                (0..n)
                    .map(|i| {
                        let a = -std::f64::consts::FRAC_PI_2
                            + std::f64::consts::TAU * i as f64 / n as f64;
                        c + Vector2::new(a.cos(), a.sin()) * r
                    })
                    .collect()
            }
        }
    }

    fn build_math_figure(&self) -> MathFigure {
        let v = self.vertices();
        // The special centers are added by the shaper when "show centers" is on
        let mut fig = MathFigure::new();
        if v.len() == 3 {
            let labels = self
                .labels
                .as_ref()
                .map(|[a, b, c]| [a.as_str(), b.as_str(), c.as_str()]);
            let (pa, pb, pc, ..) =
                fig.build_triangle(v[0], v[1], v[2], labels.or(Some(["", "", ""])));
            for p in [pa, pb, pc] {
                let pt = fig.points.get_mut(&p).unwrap();
                if pt.label.as_deref() == Some("") {
                    pt.label = None;
                }
            }
            if self.kind == MathShapeKind::RightTriangle {
                fig.add_right_angle(pb, pa, pc, 12.0);
            }
        } else {
            let ids: Vec<_> = v.iter().map(|&p| fig.add_point(p, None)).collect();
            for i in 0..ids.len() {
                fig.add_segment(ids[i], ids[(i + 1) % ids.len()]);
            }
        }
        fig
    }
}

impl Buildable for MathTriangleBuilder {
    type Emit = Shape;

    fn handle_event(
        &mut self,
        event: PenEvent,
        _now: Instant,
        constraints: Constraints,
    ) -> EventResult<BuilderProgress<Self::Emit>> {
        let progress = match event {
            PenEvent::Down { element, .. } | PenEvent::Proximity { element, .. } => {
                self.equilateral = constraints.enabled;
                self.current = element.pos;
                BuilderProgress::InProgress
            }
            PenEvent::Up { element, .. } => {
                self.equilateral = constraints.enabled;
                if (element.pos - self.start).length() >= 1.0 {
                    self.current = element.pos;
                }
                BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
            }
            PenEvent::KeyPressed { keyboard_key, .. } => match keyboard_key {
                KeyboardKey::Escape => BuilderProgress::Finished(vec![]),
                KeyboardKey::CarriageReturn | KeyboardKey::Linefeed => {
                    BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
                }
                _ => {
                    self.equilateral = constraints.enabled;
                    BuilderProgress::InProgress
                }
            },
            PenEvent::Cancel => BuilderProgress::Finished(vec![]),
            _ => BuilderProgress::InProgress,
        };

        EventResult {
            handled: true,
            propagate: EventPropagation::Stop,
            progress,
        }
    }

    fn bounds(&self, style: &Style, zoom: f64) -> Option<Aabb> {
        let fig = self.build_math_figure();
        Some(
            fig.composed_bounds(style)
                .loosened(indicators::POS_INDICATOR_RADIUS / zoom),
        )
    }

    fn draw_styled(&self, cx: &mut piet_cairo::CairoRenderContext, style: &Style, zoom: f64) {
        cx.save().unwrap();
        let fig = self.build_math_figure();
        fig.draw_composed(cx, style);
        indicators::draw_pos_indicator(cx, PenState::Down, self.start, zoom);
        indicators::draw_pos_indicator(cx, PenState::Down, self.current, zoom);
        cx.restore().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_a_closed_unnamed_polygon() {
        use MathShapeKind::*;
        for (kind, n) in [
            (Triangle, 3),
            (RightTriangle, 3),
            (IsoscelesTriangle, 3),
            (EquilateralTriangle, 3),
            (Square, 4),
            (Rectangle, 4),
            (Parallelogram, 4),
            (Rhombus, 4),
            (Trapezoid, 4),
            (Pentagon, 5),
            (Hexagon, 6),
        ] {
            let mut b =
                MathTriangleBuilder::start_with_kind(Element::new(Vector2::ZERO, 1.0), kind);
            b.current = Vector2::new(200.0, 120.0);
            let fig = b.build_math_figure();
            assert_eq!((fig.points.len(), fig.lines.len()), (n, n), "{kind:?}");
            assert!(fig.points.values().all(|p| p.label.is_none()), "{kind:?}");
        }
    }

    #[test]
    fn square_and_right_triangle_shapes() {
        let mut b = MathTriangleBuilder::start_with_kind(
            Element::new(Vector2::ZERO, 1.0),
            MathShapeKind::Square,
        );
        b.current = Vector2::new(-200.0, 120.0);
        let v = b.vertices();
        let side = |i: usize| (v[(i + 1) % 4] - v[i]).length();
        assert!((0..4).all(|i| (side(i) - 200.0).abs() < 1e-9));
        // grows to the left, from the press point
        assert!(v.iter().all(|p| p.x <= 1e-9 && p.y >= -1e-9));

        let mut b = MathTriangleBuilder::start_with_kind(
            Element::new(Vector2::ZERO, 1.0),
            MathShapeKind::RightTriangle,
        );
        b.current = Vector2::new(150.0, 100.0);
        let fig = b.build_math_figure();
        assert_eq!(fig.right_angles.len(), 1);
        let v = b.vertices();
        assert!((v[0] - v[1]).dot(v[2] - v[1]).abs() < 1e-9);
    }
}
