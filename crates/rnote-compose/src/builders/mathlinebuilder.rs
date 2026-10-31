// MathLineBuilder: Interactive builder for lines/segments with endpoints and auto-intersection
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

/// Interactive builder for geometric lines with endpoint markers and intersection detection.
#[derive(Debug, Clone)]
pub struct MathLineBuilder {
    start: Vector2,
    current: Vector2,
    p1_label: Option<String>,
    p2_label: Option<String>,
    auto_intersections: bool,
}

impl BuilderCreator for MathLineBuilder {
    fn start(element: Element, _now: Instant) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
            p1_label: Some("A".to_string()),
            p2_label: Some("B".to_string()),
            auto_intersections: true,
        }
    }
}

impl MathLineBuilder {
    /// Create builder with custom endpoint labels and intersection detection.
    pub fn start_with_config(
        element: Element,
        p1_label: Option<String>,
        p2_label: Option<String>,
        auto_intersections: bool,
    ) -> Self {
        Self {
            start: element.pos,
            current: element.pos,
            p1_label,
            p2_label,
            auto_intersections,
        }
    }

    fn build_math_figure(&self) -> MathFigure {
        let diff = self.current - self.start;
        let (p1, p2) = if diff.length() < 15.0 {
            // Default 150px horizontal line if just clicked
            (self.start, self.start + Vector2::new(150.0, 0.0))
        } else {
            (self.start, self.current)
        };

        let mut fig = MathFigure::new();
        fig.build_line(
            p1,
            p2,
            self.p1_label.as_deref(),
            self.p2_label.as_deref(),
        );

        if self.auto_intersections {
            fig.auto_detect_intersections();
        }

        fig
    }
}

impl Buildable for MathLineBuilder {
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
