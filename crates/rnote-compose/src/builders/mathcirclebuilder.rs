// MathCircleBuilder: Interactive builder for circles with explicit center point and label
use super::buildable::{Buildable, BuilderCreator, BuilderProgress};
use crate::eventresult::EventPropagation;
use crate::geometry::MathFigure;
use crate::penevent::{KeyboardKey, PenEvent, PenState};
use crate::penpath::Element;
use crate::style::{indicators, Composer};
use crate::{Constraints, EventResult, Shape, Style};
use p2d::bounding_volume::{Aabb, BoundingVolume};
use p2d::math::Vector2;
use piet::RenderContext;
use std::time::Instant;

/// Interactive builder for geometric circles with center point and label.
#[derive(Debug, Clone)]
pub struct MathCircleBuilder {
    start: Vector2,
    current: Vector2,
    center_label: String,
    boundary_label: String,
}

impl BuilderCreator for MathCircleBuilder {
    fn start(element: Element, _now: Instant) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
            center_label: "O".to_string(),
            boundary_label: "M".to_string(),
        }
    }
}

impl MathCircleBuilder {
    /// Create builder with custom center and boundary labels.
    pub fn start_with_labels(
        element: Element,
        center_label: String,
        boundary_label: String,
    ) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
            center_label,
            boundary_label,
        }
    }

    fn build_math_figure(&self) -> MathFigure {
        let radius = (self.current - self.start).length();
        let (center_pos, final_radius) = if radius < 15.0 {
            // Default circle if simply clicked
            (self.start, 80.0)
        } else {
            (self.start, radius)
        };

        let mut fig = MathFigure::new();
        fig.build_circle(
            center_pos,
            final_radius,
            Some(&self.center_label),
            Some(&self.boundary_label),
        );
        fig
    }
}

impl Buildable for MathCircleBuilder {
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
