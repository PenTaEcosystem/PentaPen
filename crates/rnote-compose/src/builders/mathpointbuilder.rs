// MathPointBuilder: Interactive builder for placing named geometric points
use super::buildable::{Buildable, BuilderCreator, BuilderProgress};
use crate::eventresult::EventPropagation;
use crate::ext::Vector2Ext;
use crate::geometry::MathFigure;
use crate::penevent::PenEvent;
use crate::penpath::Element;
use crate::{Constraints, EventResult, Shape, Style};
use p2d::bounding_volume::Aabb;
use p2d::math::Vector2;
use piet::{RenderContext, Text, TextLayoutBuilder};
use std::time::Instant;

/// Interactive builder for placing single named geometric points.
#[derive(Debug, Clone)]
pub struct MathPointBuilder {
    pos: Vector2,
    label: String,
}

impl BuilderCreator for MathPointBuilder {
    fn start(element: Element, _now: Instant) -> Self {
        Self {
            pos: element.pos,
            label: "A".to_string(),
        }
    }
}

impl MathPointBuilder {
    /// Start with a specific point label.
    pub fn start_with_label(element: Element, label: String) -> Self {
        Self {
            pos: element.pos,
            label,
        }
    }
}

impl Buildable for MathPointBuilder {
    type Emit = Shape;

    fn handle_event(
        &mut self,
        event: PenEvent,
        _now: Instant,
        _constraints: Constraints,
    ) -> EventResult<BuilderProgress<Self::Emit>> {
        let progress = match event {
            PenEvent::Down { element, .. } => {
                self.pos = element.pos;
                BuilderProgress::InProgress
            }
            PenEvent::Up { element, .. } => {
                let mut fig = MathFigure::new();
                let label = if self.label.trim().is_empty() {
                    "A".to_string()
                } else {
                    self.label.trim().to_string()
                };
                fig.add_point(element.pos, Some(label));
                BuilderProgress::Finished(vec![Shape::MathFigure(fig)])
            }
            _ => BuilderProgress::InProgress,
        };

        EventResult {
            handled: true,
            propagate: EventPropagation::Stop,
            progress,
        }
    }

    fn bounds(&self, _style: &Style, zoom: f64) -> Option<Aabb> {
        let r = 24.0 / zoom;
        Some(Aabb::new(
            self.pos - Vector2::splat(r),
            self.pos + Vector2::splat(r),
        ))
    }

    fn draw_styled(&self, cx: &mut piet_cairo::CairoRenderContext, _style: &Style, zoom: f64) {
        let pt = self.pos.to_kurbo_point();
        let circle = kurbo::Circle::new(pt, 5.0 / zoom);
        cx.fill(circle, &piet::Color::rgba8(28, 113, 216, 255));
        cx.stroke(circle, &piet::Color::rgba8(255, 255, 255, 255), 1.5 / zoom);

        if let Ok(layout) = cx
            .text()
            .new_text_layout(self.label.clone())
            .font(piet::FontFamily::SANS_SERIF, 13.0 / zoom)
            .text_color(piet::Color::rgba8(28, 113, 216, 255))
            .build()
        {
            let label_pos = kurbo::Point::new(pt.x + 6.0 / zoom, pt.y - 16.0 / zoom);
            cx.draw_text(&layout, label_pos);
        }
    }
}
