// MathTriangleBuilder: Interactive builder for triangles with classical centers
use super::buildable::{Buildable, BuilderCreator, BuilderProgress};
use crate::eventresult::EventPropagation;
use crate::geometry::{MathFigure, TriangleCenterType};
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
}

impl BuilderCreator for MathTriangleBuilder {
    fn start(element: Element, _now: Instant) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
        }
    }
}

impl MathTriangleBuilder {
    fn build_math_figure(&self) -> MathFigure {
        let min_x = self.start.x.min(self.current.x);
        let max_x = self.start.x.max(self.current.x);
        let min_y = self.start.y.min(self.current.y);
        let max_y = self.start.y.max(self.current.y);

        let width = max_x - min_x;
        let height = max_y - min_y;

        let (a, b, c) = if width < 30.0 || height < 30.0 {
            // Default scalene triangle if simply clicked or tiny drag
            let cx = self.start.x;
            let cy = self.start.y;
            let w = 240.0;
            let h = 180.0;
            (
                Vector2::new(cx - 30.0, cy - h / 2.0),
                Vector2::new(cx - w / 2.0, cy + h / 2.0),
                Vector2::new(cx + w / 2.0, cy + h / 2.0),
            )
        } else {
            (
                Vector2::new(min_x + 0.35 * width, min_y),
                Vector2::new(min_x, max_y),
                Vector2::new(max_x, max_y),
            )
        };

        let mut fig = MathFigure::new();
        let (pa, pb, pc, _, _, _) = fig.build_triangle(a, b, c, Some(["A", "B", "C"]));
        // Add 4 special centers (Centroid G, Circumcenter O, Orthocenter H, Incenter I)
        fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Centroid, true);
        fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Circumcenter, true);
        fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Orthocenter, true);
        fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Incenter, true);

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
            PenEvent::Down { element, .. } => {
                self.current = constraints.constrain(element.pos - self.start) + self.start;
                BuilderProgress::InProgress
            }
            PenEvent::Up { .. } => {
                BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
            }
            PenEvent::KeyPressed { keyboard_key, .. } => match keyboard_key {
                KeyboardKey::Escape => BuilderProgress::Finished(vec![]),
                KeyboardKey::CarriageReturn | KeyboardKey::Linefeed => {
                    BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
                }
                _ => BuilderProgress::InProgress,
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
