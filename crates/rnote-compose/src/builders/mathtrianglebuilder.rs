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

/// Interactive builder for geometric triangles with 4 classical centers.
#[derive(Debug, Clone)]
pub struct MathTriangleBuilder {
    start: Vector2,
    current: Vector2,
    labels: [String; 3],
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
            start: element.pos,
            current: element.pos,
            labels,
            equilateral: false,
        }
    }

    fn build_math_figure(&self) -> MathFigure {
        let min_x = self.start.x.min(self.current.x);
        let max_x = self.start.x.max(self.current.x);
        let min_y = self.start.y.min(self.current.y);
        let max_y = self.start.y.max(self.current.y);

        let width = max_x - min_x;
        let height = max_y - min_y;

        let (a, b, c) = if width < 30.0 && height < 30.0 {
            // Default triangle if simply clicked or tiny drag
            let cx = self.start.x;
            let cy = self.start.y;
            let w = 240.0;
            let h = 180.0;
            (
                Vector2::new(cx - 30.0, cy - h / 2.0),
                Vector2::new(cx - w / 2.0, cy + h / 2.0),
                Vector2::new(cx + w / 2.0, cy + h / 2.0),
            )
        } else if self.equilateral {
            let side = width.max(height).max(40.0);
            let h = side * 3.0_f64.sqrt() * 0.5;
            let cx = (min_x + max_x) * 0.5;
            let bottom_y = min_y + h;
            (
                Vector2::new(cx, min_y),
                Vector2::new(cx - side * 0.5, bottom_y),
                Vector2::new(cx + side * 0.5, bottom_y),
            )
        } else {
            (
                Vector2::new(min_x + 0.35 * width, min_y),
                Vector2::new(min_x, max_y),
                Vector2::new(max_x, max_y),
            )
        };

        // The special centers are added by the shaper when "show centers" is on
        let mut fig = MathFigure::new();
        let [la, lb, lc] = &self.labels;
        fig.build_triangle(a, b, c, Some([la.as_str(), lb.as_str(), lc.as_str()]));
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
