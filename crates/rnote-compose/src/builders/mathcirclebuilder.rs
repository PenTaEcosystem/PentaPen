// MathCircleBuilder: Interactive builder for circles with explicit center point and label
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

    fn constrain_boundary(start: Vector2, pos: Vector2, snap_angle: bool) -> Vector2 {
        let diff = pos - start;
        let r = diff.length();
        if !snap_angle || r < 1e-6 {
            return pos;
        }
        let step = std::f64::consts::FRAC_PI_4;
        let angle = (diff.y.atan2(diff.x) / step).round() * step;
        start + Vector2::new(r * angle.cos(), r * angle.sin())
    }

    fn build_math_figure(&self) -> MathFigure {
        let diff = self.current - self.start;
        let radius = diff.length();
        let (center_pos, final_radius, dir) = if radius < 15.0 {
            // Default circle if simply clicked
            (self.start, 80.0, Vector2::new(1.0, 0.0))
        } else {
            (self.start, radius, diff / radius)
        };

        let mut fig = MathFigure::new();
        // An empty name (center on an existing point) stays unnamed instead of an empty label
        let center_label = (!self.center_label.is_empty()).then(|| self.center_label.clone());
        let center_id = fig.add_point(center_pos, center_label);
        let boundary_pos = center_pos + dir * final_radius;
        let boundary_id = fig.add_point(boundary_pos, Some(self.boundary_label.clone()));
        fig.add_circle_through(center_id, boundary_id);
        fig.add_auxiliary_line(
            center_id,
            boundary_id,
            crate::geometry::entity::LineType::Segment,
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
            PenEvent::Down { element, .. } | PenEvent::Proximity { element, .. } => {
                self.current =
                    Self::constrain_boundary(self.start, element.pos, constraints.enabled);
                BuilderProgress::InProgress
            }
            PenEvent::Up { element, .. } => {
                if (element.pos - self.start).length() >= 1.0 {
                    self.current =
                        Self::constrain_boundary(self.start, element.pos, constraints.enabled);
                }
                BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
            }
            PenEvent::KeyPressed { keyboard_key, .. } => match keyboard_key {
                KeyboardKey::Escape => BuilderProgress::Finished(vec![]),
                KeyboardKey::CarriageReturn | KeyboardKey::Linefeed => {
                    BuilderProgress::Finished(vec![Shape::MathFigure(self.build_math_figure())])
                }
                _ => {
                    self.current =
                        Self::constrain_boundary(self.start, self.current, constraints.enabled);
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
