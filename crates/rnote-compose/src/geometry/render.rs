// Rendering logic for MathFigure using Piet
use super::entity::LineStyle;
use super::figure::MathFigure;
use p2d::math::Vector2;
use piet::{Color, RenderContext, StrokeStyle, Text, TextLayoutBuilder};

/// Render a MathFigure onto a Piet RenderContext.
pub fn draw_math_figure(
    figure: &MathFigure,
    cx: &mut impl RenderContext,
    stroke_color: Color,
    stroke_width: f64,
) {
    cx.save().unwrap();

    let stroke_brush = cx.solid_brush(stroke_color.clone());
    let solid_style = StrokeStyle::new();
    let dashed_style = StrokeStyle::new().dash_pattern(&[6.0, 4.0]);
    let dotted_style = StrokeStyle::new().dash_pattern(&[2.0, 3.0]);

    // 1. Draw Lines
    for line in figure.lines.values() {
        if !line.visible {
            continue;
        }
        if let (Some(p1), Some(p2)) = (figure.points.get(&line.p1), figure.points.get(&line.p2)) {
            let kurbo_line = kurbo::Line::new(
                kurbo::Point::new(p1.pos.x, p1.pos.y),
                kurbo::Point::new(p2.pos.x, p2.pos.y),
            );

            let style = match line.style {
                LineStyle::Solid => &solid_style,
                LineStyle::Dashed => &dashed_style,
                LineStyle::Dotted => &dotted_style,
            };

            cx.stroke_styled(kurbo_line, &stroke_brush, stroke_width, style);

            // Draw tick marks if any
            if line.tick_marks > 0 {
                let mid = (p1.pos + p2.pos) * 0.5;
                let dir = (p2.pos - p1.pos).normalize();
                let normal = Vector2::new(-dir.y, dir.x);
                let tick_len = 5.0;

                match line.tick_marks {
                    1 => {
                        let t1 = mid - normal * tick_len;
                        let t2 = mid + normal * tick_len;
                        cx.stroke(
                            kurbo::Line::new(
                                kurbo::Point::new(t1.x, t1.y),
                                kurbo::Point::new(t2.x, t2.y),
                            ),
                            &stroke_brush,
                            stroke_width,
                        );
                    }
                    _ => {
                        for offset in [-2.5, 2.5] {
                            let center = mid + dir * offset;
                            let t1 = center - normal * tick_len;
                            let t2 = center + normal * tick_len;
                            cx.stroke(
                                kurbo::Line::new(
                                    kurbo::Point::new(t1.x, t1.y),
                                    kurbo::Point::new(t2.x, t2.y),
                                ),
                                &stroke_brush,
                                stroke_width,
                            );
                        }
                    }
                }
            }
        }
    }

    // 2. Draw Circles
    for circ in figure.circles.values() {
        if !circ.visible {
            continue;
        }
        if let Some(c) = figure.points.get(&circ.center) {
            let circle = kurbo::Circle::new(kurbo::Point::new(c.pos.x, c.pos.y), circ.radius);
            let style = match circ.style {
                LineStyle::Solid => &solid_style,
                LineStyle::Dashed => &dashed_style,
                LineStyle::Dotted => &dotted_style,
            };
            cx.stroke_styled(circle, &stroke_brush, stroke_width, style);
        }
    }

    // 3. Draw Right Angle Marks
    for ra in &figure.right_angles {
        if let (Some(v), Some(a1), Some(a2)) = (
            figure.points.get(&ra.vertex),
            figure.points.get(&ra.arm1),
            figure.points.get(&ra.arm2),
        ) {
            let d1 = (a1.pos - v.pos).normalize();
            let d2 = (a2.pos - v.pos).normalize();
            let c1 = v.pos + d1 * ra.size;
            let c_mid = v.pos + (d1 + d2) * ra.size;
            let c2 = v.pos + d2 * ra.size;

            let mut path = kurbo::BezPath::new();
            path.move_to(kurbo::Point::new(c1.x, c1.y));
            path.line_to(kurbo::Point::new(c_mid.x, c_mid.y));
            path.line_to(kurbo::Point::new(c2.x, c2.y));

            cx.stroke(path, &stroke_brush, stroke_width * 0.8);
        }
    }

    // 4. Draw Points & Labels
    const POINT_DOT_RADIUS: f64 = 2.8;
    for pt in figure.points.values() {
        if !pt.visible {
            continue;
        }
        // Small filled circle for point vertex
        let dot = kurbo::Circle::new(
            kurbo::Point::new(pt.pos.x, pt.pos.y),
            POINT_DOT_RADIUS,
        );
        cx.fill(dot, &stroke_brush);

        // Point label
        if let Some(ref text) = pt.label {
            if let Ok(layout) = cx
                .text()
                .new_text_layout(text.clone())
                .font(piet::FontFamily::SANS_SERIF, 13.0)
                .text_color(stroke_color.clone())
                .build()
            {
                // Offset label slightly upper-right from vertex
                let label_pos = kurbo::Point::new(pt.pos.x + 6.0, pt.pos.y - 16.0);
                cx.draw_text(&layout, label_pos);
            }
        }
    }

    cx.restore().unwrap();
}
