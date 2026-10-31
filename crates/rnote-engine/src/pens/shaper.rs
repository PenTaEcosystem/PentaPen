// Imports
use super::PenBehaviour;
use super::PenStyle;
use crate::engine::{EngineView, EngineViewMut};
use crate::pens::pensconfig::ShaperConfig;
use crate::store::StrokeKey;
use crate::strokes::ShapeStroke;
use crate::strokes::Stroke;
use crate::{DrawableOnDoc, WidgetFlags};
use p2d::bounding_volume::Aabb;
use p2d::math::Vector2;
use piet::{RenderContext, Text, TextLayoutBuilder};
use rnote_compose::Transformable;
use rnote_compose::builders::buildable::{Buildable, BuilderCreator, BuilderProgress};
use rnote_compose::builders::{
    ArrowBuilder, CoordSystem2DBuilder, CoordSystem3DBuilder, CubBezBuilder, EllipseBuilder,
    FociEllipseBuilder, GridBuilder, LineBuilder, MathCircleBuilder, MathLineBuilder,
    MathPointBuilder, MathTriangleBuilder, PolygonBuilder, PolylineBuilder, QuadBezBuilder,
    QuadrantCoordSystem2DBuilder, RectangleBuilder, ShapeBuilderType,
};
use rnote_compose::eventresult::{EventPropagation, EventResult};
use rnote_compose::ext::Vector2Ext;
use rnote_compose::penevent::{KeyboardKey, ModifierKey, PenEvent, PenProgress};
use rnote_compose::penpath::Element;
use rnote_compose::shapes::Shapeable;
use rnote_compose::shapes::geometry_snap::{
    CircleCircleResult, LineCircleResult, SnapCandidate, SnapKind, SnapPriority,
    compute_chord_midpoint, compute_circle_circle_intersections,
    compute_line_circle_intersections, compute_line_line_intersection,
    compute_segment_midpoint, find_magnetic_snap,
};
use rnote_compose::Shape;
use std::time::Instant;

/// Kinds of manipulation handles for selected shapes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandleKind {
    /// Line start point (A)
    LineStart,
    /// Line end point (B)
    LineEnd,
    /// Line midpoint (M) - translates the line parallelly
    LineMid,
    /// Circle/ellipse center handle (O) - translates the circle
    CircleCenter,
    /// Circle/ellipse perimeter handle - radial stretching
    CirclePerimeter(usize),
}

/// Data for a shape currently selected for interactive manipulation.
#[derive(Debug, Clone)]
pub struct SelectedShapeData {
    pub key: StrokeKey,
}

#[derive(Debug)]
enum ShaperState {
    Idle,
    BuildShape {
        builder: Box<dyn Buildable<Emit = Shape>>,
    },
    Manipulating {
        selected: SelectedShapeData,
        active_handle: Option<HandleKind>,
        last_drag_pos: Option<Vector2>,
    },
}

#[derive(Debug)]
pub struct Shaper {
    state: ShaperState,
    hovered_stroke: Option<StrokeKey>,
    active_snap: Option<SnapCandidate>,
    last_cursor_pos: Option<Vector2>,
}

impl Default for Shaper {
    fn default() -> Self {
        Self {
            state: ShaperState::Idle,
            hovered_stroke: None,
            active_snap: None,
            last_cursor_pos: None,
        }
    }
}

impl PenBehaviour for Shaper {
    fn init(&mut self, _engine_view: &EngineView) -> WidgetFlags {
        WidgetFlags::default()
    }

    fn deinit(&mut self) -> WidgetFlags {
        WidgetFlags::default()
    }

    fn style(&self) -> PenStyle {
        PenStyle::Shaper
    }

    fn update_state(&mut self, _engine_view: &mut EngineViewMut) -> WidgetFlags {
        WidgetFlags::default()
    }

    fn handle_event(
        &mut self,
        event: PenEvent,
        now: Instant,
        engine_view: &mut EngineViewMut,
    ) -> (EventResult<PenProgress>, WidgetFlags) {
        let mut widget_flags = WidgetFlags::default();
        let zoom = engine_view.camera.total_zoom().max(0.01);
        let hit_threshold = 16.0 / zoom;
        let snap_radius = 18.0 / zoom;

        // Track cursor and hover state on Proximity events
        if let PenEvent::Proximity { element, .. } = &event {
            self.last_cursor_pos = Some(element.pos);
            let candidates = collect_snap_candidates(engine_view.store, None);
            self.active_snap = find_magnetic_snap(element.pos, &candidates, snap_radius);

            let new_hover = hit_test_stroke(element.pos, engine_view.store, hit_threshold)
                .map(|(k, _)| k);
            if new_hover != self.hovered_stroke || self.active_snap.is_some() {
                widget_flags.redraw = true;
            }
            self.hovered_stroke = new_hover;

            return (
                EventResult {
                    handled: false,
                    propagate: EventPropagation::Proceed,
                    progress: PenProgress::Idle,
                },
                widget_flags,
            );
        }

        let event_result = match (&mut self.state, event) {
            // ==========================================
            // IDLE STATE
            // ==========================================
            (ShaperState::Idle, PenEvent::Down { mut element, .. }) => {
                // Check if user clicked directly on an existing shape (Line or Ellipse)
                if let Some((hit_key, shape)) =
                    hit_test_stroke(element.pos, engine_view.store, hit_threshold)
                {
                    if matches!(shape, Shape::Ellipse(_))
                        || match shape {
                            Shape::MathFigure(ref fig) => {
                                !fig.circles.is_empty() || !fig.points.is_empty()
                            }
                            _ => false,
                        }
                    {
                        widget_flags.open_circle_settings = true;
                    }
                    self.state = ShaperState::Manipulating {
                        selected: SelectedShapeData { key: hit_key },
                        active_handle: None,
                        last_drag_pos: Some(element.pos),
                    };
                    self.active_snap = None;
                    widget_flags.redraw = true;

                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::InProgress,
                    }
                } else {
                    // Clicked empty space: start building a new shape
                    engine_view
                        .config
                        .pens_config
                        .shaper_config
                        .new_style_seeds();

                    let candidates = collect_snap_candidates(engine_view.store, None);
                    if let Some(snap) = find_magnetic_snap(element.pos, &candidates, snap_radius) {
                        element.pos = snap.point;
                        self.active_snap = Some(snap);
                    } else {
                        element.pos = engine_view
                            .document
                            .snap_position(element.pos, engine_view.config);
                        element.pos = snap_to_math_figure_points(element.pos, engine_view.store);
                        self.active_snap = None;
                    }

                    self.state = ShaperState::BuildShape {
                        builder: new_builder(
                            &engine_view.config.pens_config.shaper_config,
                            element,
                            now,
                        ),
                    };

                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::InProgress,
                    }
                }
            }

            (
                ShaperState::Idle,
                PenEvent::KeyPressed {
                    keyboard_key,
                    modifier_keys,
                },
            ) if modifier_keys.is_empty() => {
                let shaper_config = &mut engine_view.config.pens_config.shaper_config;
                let handled = match keyboard_key {
                    KeyboardKey::Unicode('l') | KeyboardKey::Unicode('L') => {
                        shaper_config.builder_type = ShapeBuilderType::MathLine;
                        true
                    }
                    KeyboardKey::Unicode('r') | KeyboardKey::Unicode('R') => {
                        shaper_config.builder_type = ShapeBuilderType::MathCircle;
                        true
                    }
                    KeyboardKey::Unicode('s') | KeyboardKey::Unicode('S') => {
                        shaper_config.builder_type = ShapeBuilderType::Rectangle;
                        true
                    }
                    KeyboardKey::Unicode('p') | KeyboardKey::Unicode('P') => {
                        shaper_config.builder_type = ShapeBuilderType::MathPoint;
                        true
                    }
                    _ => false,
                };
                if handled {
                    widget_flags.refresh_ui = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Idle,
                    }
                } else {
                    EventResult {
                        handled: false,
                        propagate: EventPropagation::Proceed,
                        progress: PenProgress::Idle,
                    }
                }
            }

            (ShaperState::Idle, _) => EventResult {
                handled: false,
                propagate: EventPropagation::Proceed,
                progress: PenProgress::Idle,
            },

            // ==========================================
            // MANIPULATING STATE (Interactive Handles)
            // ==========================================
            (
                ShaperState::Manipulating {
                    selected,
                    active_handle,
                    last_drag_pos,
                },
                PenEvent::Down { mut element, .. },
            ) => {
                if active_handle.is_none() {
                    // Check if clicked on a handle of the currently selected shape
                    let handle = engine_view
                        .store
                        .get_stroke_ref(selected.key)
                        .and_then(|st| match st {
                            Stroke::ShapeStroke(ss) => {
                                hit_test_handle(element.pos, &ss.shape, hit_threshold)
                            }
                            _ => None,
                        });

                    if let Some(h) = handle {
                        if matches!(h, HandleKind::CircleCenter | HandleKind::CirclePerimeter(_)) {
                            widget_flags.open_circle_settings = true;
                        }
                        *active_handle = Some(h);
                        *last_drag_pos = Some(element.pos);
                    } else if let Some((other_key, other_shape)) =
                        hit_test_stroke(element.pos, engine_view.store, hit_threshold)
                    {
                        // Switched selection to another shape
                        if matches!(other_shape, Shape::Ellipse(_))
                            || match other_shape {
                                Shape::MathFigure(ref fig) => {
                                    !fig.circles.is_empty() || !fig.points.is_empty()
                                }
                                _ => false,
                            }
                        {
                            widget_flags.open_circle_settings = true;
                        }
                        selected.key = other_key;
                        *active_handle = None;
                        *last_drag_pos = Some(element.pos);
                        widget_flags.redraw = true;
                    } else {
                        // Clicked outside: deselect and start new shape
                        engine_view
                            .config
                            .pens_config
                            .shaper_config
                            .new_style_seeds();
                        element.pos = engine_view
                            .document
                            .snap_position(element.pos, engine_view.config);
                        self.state = ShaperState::BuildShape {
                            builder: new_builder(
                                &engine_view.config.pens_config.shaper_config,
                                element,
                                now,
                            ),
                        };
                        widget_flags.redraw = true;
                        return (
                            EventResult {
                                handled: true,
                                propagate: EventPropagation::Stop,
                                progress: PenProgress::InProgress,
                            },
                            widget_flags,
                        );
                    }
                }

                // Dragging the active handle
                if let Some(handle) = *active_handle {
                    // Check magnetic snap while dragging handle
                    let candidates = collect_snap_candidates(engine_view.store, None);
                    if let Some(snap) = find_magnetic_snap(element.pos, &candidates, snap_radius) {
                        element.pos = snap.point;
                        self.active_snap = Some(snap);
                    } else {
                        self.active_snap = None;
                    }

                    apply_handle_drag(
                        selected.key,
                        handle,
                        element.pos,
                        *last_drag_pos,
                        engine_view,
                    );
                    *last_drag_pos = Some(element.pos);
                    widget_flags.redraw = true;
                    widget_flags.store_modified = true;
                }

                EventResult {
                    handled: true,
                    propagate: EventPropagation::Stop,
                    progress: PenProgress::InProgress,
                }
            }

            (
                ShaperState::Manipulating {
                    active_handle,
                    last_drag_pos,
                    ..
                },
                PenEvent::Up { .. },
            ) => {
                if active_handle.is_some() {
                    widget_flags |= engine_view.store.record(Instant::now());
                    *active_handle = None;
                    *last_drag_pos = None;
                    self.active_snap = None;
                    widget_flags.redraw = true;
                }

                EventResult {
                    handled: true,
                    propagate: EventPropagation::Stop,
                    progress: PenProgress::Idle,
                }
            }

            (
                ShaperState::Manipulating { selected, .. },
                PenEvent::KeyPressed {
                    keyboard_key,
                    modifier_keys,
                },
            ) if modifier_keys.is_empty() => match keyboard_key {
                KeyboardKey::Escape => {
                    self.state = ShaperState::Idle;
                    self.active_snap = None;
                    widget_flags.redraw = true;

                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Finished,
                    }
                }
                KeyboardKey::Unicode('l') | KeyboardKey::Unicode('L') => {
                    engine_view.config.pens_config.shaper_config.builder_type =
                        ShapeBuilderType::MathLine;
                    self.state = ShaperState::Idle;
                    widget_flags.refresh_ui = true;
                    widget_flags.redraw = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Idle,
                    }
                }
                KeyboardKey::Unicode('r') | KeyboardKey::Unicode('R') => {
                    engine_view.config.pens_config.shaper_config.builder_type =
                        ShapeBuilderType::MathCircle;
                    self.state = ShaperState::Idle;
                    widget_flags.refresh_ui = true;
                    widget_flags.redraw = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Idle,
                    }
                }
                KeyboardKey::Unicode('s') | KeyboardKey::Unicode('S') => {
                    engine_view.config.pens_config.shaper_config.builder_type =
                        ShapeBuilderType::Rectangle;
                    self.state = ShaperState::Idle;
                    widget_flags.refresh_ui = true;
                    widget_flags.redraw = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Idle,
                    }
                }
                KeyboardKey::Unicode('p') | KeyboardKey::Unicode('P') => {
                    engine_view.config.pens_config.shaper_config.builder_type =
                        ShapeBuilderType::MathPoint;
                    self.state = ShaperState::Idle;
                    widget_flags.refresh_ui = true;
                    widget_flags.redraw = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::Idle,
                    }
                }
                KeyboardKey::Unicode(c) if c.is_alphanumeric() => {
                    let new_label = c.to_uppercase().to_string();
                    engine_view.config.pens_config.shaper_config.math_center_label =
                        new_label.clone();
                    if let Some(Stroke::ShapeStroke(ss)) =
                        engine_view.store.get_stroke_mut(selected.key)
                    {
                        if let Shape::MathFigure(fig) = &mut ss.shape {
                            for pt in fig.points.values_mut() {
                                pt.label = Some(new_label.clone());
                            }
                            widget_flags.store_modified = true;
                        }
                    }
                    widget_flags.refresh_ui = true;
                    widget_flags.redraw = true;
                    EventResult {
                        handled: true,
                        propagate: EventPropagation::Stop,
                        progress: PenProgress::InProgress,
                    }
                }
                _ => EventResult {
                    handled: false,
                    propagate: EventPropagation::Proceed,
                    progress: PenProgress::Idle,
                },
            },

            (ShaperState::Manipulating { .. }, _) => EventResult {
                handled: false,
                propagate: EventPropagation::Proceed,
                progress: PenProgress::Idle,
            },

            // ==========================================
            // BUILD SHAPE STATE
            // ==========================================
            (ShaperState::BuildShape { .. }, PenEvent::Cancel) => {
                self.state = ShaperState::Idle;
                self.active_snap = None;

                EventResult {
                    handled: false,
                    propagate: EventPropagation::Stop,
                    progress: PenProgress::Finished,
                }
            }

            (ShaperState::BuildShape { builder }, mut event) => {
                let mut constraints = engine_view
                    .config
                    .pens_config
                    .shaper_config
                    .constraints
                    .clone();
                constraints.enabled = match event {
                    PenEvent::Down {
                        ref modifier_keys, ..
                    }
                    | PenEvent::Up {
                        ref modifier_keys, ..
                    }
                    | PenEvent::Proximity {
                        ref modifier_keys, ..
                    }
                    | PenEvent::KeyPressed {
                        ref modifier_keys, ..
                    } => constraints.enabled ^ modifier_keys.contains(&ModifierKey::KeyboardCtrl),
                    PenEvent::Text { .. } | PenEvent::Cancel => false,
                };

                // Magnetic Snapping during building
                match &mut event {
                    PenEvent::Down { element, .. } | PenEvent::Up { element, .. } => {
                        let candidates = collect_snap_candidates(engine_view.store, None);
                        if let Some(snap) =
                            find_magnetic_snap(element.pos, &candidates, snap_radius)
                        {
                            element.pos = snap.point;
                            self.active_snap = Some(snap);
                        } else {
                            element.pos = engine_view
                                .document
                                .snap_position(element.pos, engine_view.config);
                            element.pos =
                                snap_to_math_figure_points(element.pos, engine_view.store);
                            self.active_snap = None;
                        }
                    }
                    _ => {}
                }

                let builder_result = builder.handle_event(event.clone(), now, constraints);
                let handled = builder_result.handled;
                let propagate = builder_result.propagate;

                let mut progress = match builder_result.progress {
                    BuilderProgress::InProgress => PenProgress::InProgress,
                    BuilderProgress::EmitContinue(shapes) => {
                        let mut style = engine_view
                            .config
                            .pens_config
                            .shaper_config
                            .gen_style_for_current_options();
                        let shapes_emitted = !shapes.is_empty();

                        for shape in shapes {
                            let key = engine_view.store.insert_stroke(
                                Stroke::ShapeStroke(ShapeStroke::new(shape, style.clone())),
                                None,
                            );
                            style.advance_seed();
                            engine_view.store.regenerate_rendering_for_stroke(
                                key,
                                engine_view.camera.viewport(),
                                engine_view.camera.image_scale(),
                            );
                        }

                        if shapes_emitted {
                            widget_flags |= engine_view.store.record(Instant::now());
                            widget_flags.store_modified = true;
                        }
                        PenProgress::InProgress
                    }
                    BuilderProgress::Finished(shapes) => {
                        let mut style = engine_view
                            .config
                            .pens_config
                            .shaper_config
                            .gen_style_for_current_options();

                        let shapes_emitted = !shapes.is_empty();
                        for mut shape in shapes {
                            if let Shape::MathFigure(ref mut fig) = shape {
                                if engine_view
                                    .config
                                    .pens_config
                                    .shaper_config
                                    .math_auto_intersections
                                {
                                    fig.auto_detect_intersections();
                                }
                                if engine_view
                                    .config
                                    .pens_config
                                    .shaper_config
                                    .math_show_centers
                                {
                                    fig.auto_detect_triangles_and_centers(true);
                                }
                            }
                            let key = engine_view.store.insert_stroke(
                                Stroke::ShapeStroke(ShapeStroke::new(shape, style.clone())),
                                None,
                            );
                            style.advance_seed();
                            engine_view.store.regenerate_rendering_for_stroke(
                                key,
                                engine_view.camera.viewport(),
                                engine_view.camera.image_scale(),
                            );
                        }

                        self.state = ShaperState::Idle;
                        self.active_snap = None;

                        if engine_view.config.pens_config.shaper_config.builder_type
                            == ShapeBuilderType::MathPoint
                        {
                            engine_view
                                .config
                                .pens_config
                                .shaper_config
                                .advance_point_label();
                            widget_flags.refresh_ui = true;
                        }

                        if shapes_emitted {
                            widget_flags |= engine_view
                                .document
                                .resize_autoexpand(engine_view.store, engine_view.camera)
                                | engine_view.store.record(Instant::now());
                            widget_flags.store_modified = true;
                        }
                        PenProgress::Finished
                    }
                };

                // When esc is pressed, reset state
                if let PenEvent::KeyPressed {
                    keyboard_key,
                    modifier_keys,
                } = event
                    && keyboard_key == KeyboardKey::Escape
                    && modifier_keys.is_empty()
                {
                    self.state = ShaperState::Idle;
                    self.active_snap = None;
                    progress = PenProgress::Finished;
                }

                EventResult {
                    handled,
                    propagate,
                    progress,
                }
            }
        };

        (event_result, widget_flags)
    }
}

impl DrawableOnDoc for Shaper {
    fn bounds_on_doc(&self, engine_view: &EngineView) -> Option<Aabb> {
        // Redraw whole viewport when handles or glowing halo or snap overlay are active
        Some(engine_view.camera.viewport())
    }

    fn draw_on_doc(
        &self,
        cx: &mut piet_cairo::CairoRenderContext,
        engine_view: &EngineView,
    ) -> anyhow::Result<()> {
        cx.save().map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let zoom = engine_view.camera.total_zoom().max(0.01);
        let style = engine_view
            .config
            .pens_config
            .shaper_config
            .gen_style_for_current_options();

        // 1. Hover Glowing Halo
        if let Some(hover_key) = self.hovered_stroke {
            if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_ref(hover_key) {
                draw_shape_glow(cx, &ss.shape, zoom)?;
            }
        }

        // 2. Manipulating handles if a shape is selected
        if let ShaperState::Manipulating { selected, .. } = &self.state {
            if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_ref(selected.key) {
                draw_manipulating_handles(cx, &ss.shape, zoom)?;
            }
        }

        // 3. Current builder shape
        if let ShaperState::BuildShape { builder } = &self.state {
            builder.draw_styled(cx, &style, zoom);
        }

        // 4. Magnetic Snap Visual Overlay
        if let Some(snap) = self.active_snap {
            draw_snap_overlay(cx, snap, zoom)?;
        }

        cx.restore().map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(())
    }
}

// ==========================================
// RENDERING HELPERS
// ==========================================

fn draw_shape_glow(
    cx: &mut piet_cairo::CairoRenderContext,
    shape: &Shape,
    zoom: f64,
) -> anyhow::Result<()> {
    let glow_color = piet::Color::rgba8(53, 132, 228, 90);
    let glow_width = 14.0 / zoom;

    match shape {
        Shape::Line(line) => {
            let kurbo_line =
                kurbo::Line::new(line.start.to_kurbo_point(), line.end.to_kurbo_point());
            cx.stroke(kurbo_line, &glow_color, glow_width);
        }
        Shape::Ellipse(ellipse) => {
            let path = ellipse.outline_path();
            cx.stroke(path, &glow_color, glow_width);
        }
        Shape::MathFigure(fig) => {
            for pt in fig.points.values() {
                let circle = kurbo::Circle::new(pt.pos.to_kurbo_point(), 8.0 / zoom);
                cx.fill(circle, &piet::Color::rgba8(53, 132, 228, 120));
            }
        }
        _ => {}
    }
    Ok(())
}

fn draw_manipulating_handles(
    cx: &mut piet_cairo::CairoRenderContext,
    shape: &Shape,
    zoom: f64,
) -> anyhow::Result<()> {
    let border_color = piet::Color::rgba8(28, 113, 216, 255);
    let white_fill = piet::Color::rgba8(255, 255, 255, 255);
    let line_dash_color = piet::Color::rgba8(53, 132, 228, 160);

    match shape {
        Shape::Line(line) => {
            // Draw connecting highlight
            let k_line =
                kurbo::Line::new(line.start.to_kurbo_point(), line.end.to_kurbo_point());
            cx.stroke(k_line, &line_dash_color, 2.0 / zoom);

            // Endpoint A handle
            let circle_a = kurbo::Circle::new(line.start.to_kurbo_point(), 6.0 / zoom);
            cx.fill(circle_a, &white_fill);
            cx.stroke(circle_a, &border_color, 2.0 / zoom);

            // Endpoint B handle
            let circle_b = kurbo::Circle::new(line.end.to_kurbo_point(), 6.0 / zoom);
            cx.fill(circle_b, &white_fill);
            cx.stroke(circle_b, &border_color, 2.0 / zoom);

            // Midpoint M handle (square)
            let m = (line.start + line.end) * 0.5;
            let half = 4.5 / zoom;
            let rect_m = kurbo::Rect::new(m.x - half, m.y - half, m.x + half, m.y + half);
            cx.fill(rect_m, &white_fill);
            cx.stroke(
                rect_m,
                &piet::Color::rgba8(46, 194, 126, 255),
                2.0 / zoom,
            );
        }
        Shape::Ellipse(ellipse) => {
            let center = ellipse.affine.translation;
            let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;

            // Highlight ring
            cx.stroke(ellipse.outline_path(), &line_dash_color, 1.5 / zoom);

            // Center handle O
            let circle_o = kurbo::Circle::new(center.to_kurbo_point(), 6.0 / zoom);
            cx.fill(circle_o, &white_fill);
            cx.stroke(circle_o, &border_color, 2.0 / zoom);
            let dot_o = kurbo::Circle::new(center.to_kurbo_point(), 2.0 / zoom);
            cx.fill(dot_o, &border_color);

            // Radius guide line to right handle
            let right_p = center + Vector2::new(r, 0.0);
            cx.stroke(
                kurbo::Line::new(center.to_kurbo_point(), right_p.to_kurbo_point()),
                &line_dash_color,
                1.5 / zoom,
            );

            // Center badge
            if let Ok(layout) = cx
                .text()
                .new_text_layout("⚙ Tâm: O (Click cài đặt)".to_string())
                .font(piet::FontFamily::SANS_SERIF, 11.5 / zoom)
                .text_color(piet::Color::rgba8(28, 113, 216, 255))
                .build()
            {
                let badge_pos = kurbo::Point::new(center.x - 45.0 / zoom, center.y - 20.0 / zoom);
                cx.draw_text(&layout, badge_pos);
            }

            // 4 Perimeter handles
            let cardinal = [
                center + Vector2::new(r, 0.0),
                center - Vector2::new(r, 0.0),
                center + Vector2::new(0.0, r),
                center - Vector2::new(0.0, r),
            ];
            for p in cardinal {
                let h = kurbo::Circle::new(p.to_kurbo_point(), 5.0 / zoom);
                cx.fill(h, &white_fill);
                cx.stroke(h, &border_color, 2.0 / zoom);
            }
        }
        Shape::MathFigure(fig) => {
            // Circles in MathFigure
            for c in fig.circles.values() {
                if let Some(center) = fig.points.get(&c.center) {
                    let perimeter_pos = c
                        .through_point
                        .and_then(|tp| fig.points.get(&tp))
                        .map(|p| p.pos)
                        .unwrap_or_else(|| center.pos + Vector2::new(c.radius, 0.0));
                    let r = (perimeter_pos - center.pos).length().max(c.radius);
                    let circle_k = kurbo::Circle::new(center.pos.to_kurbo_point(), r);
                    cx.stroke(circle_k, &line_dash_color, 1.5 / zoom);

                    // Center handle
                    let circle_o = kurbo::Circle::new(center.pos.to_kurbo_point(), 6.0 / zoom);
                    cx.fill(circle_o, &white_fill);
                    cx.stroke(circle_o, &border_color, 2.0 / zoom);
                    let dot_o = kurbo::Circle::new(center.pos.to_kurbo_point(), 2.0 / zoom);
                    cx.fill(dot_o, &border_color);

                    // Perimeter handle
                    let circle_p = kurbo::Circle::new(perimeter_pos.to_kurbo_point(), 5.0 / zoom);
                    cx.fill(circle_p, &white_fill);
                    cx.stroke(circle_p, &border_color, 2.0 / zoom);

                    // Radius guide
                    cx.stroke(
                        kurbo::Line::new(
                            center.pos.to_kurbo_point(),
                            perimeter_pos.to_kurbo_point(),
                        ),
                        &line_dash_color,
                        1.5 / zoom,
                    );

                    let c_lbl = center.label.as_deref().unwrap_or("O");
                    if let Ok(layout) = cx
                        .text()
                        .new_text_layout(format!("⚙ Tâm: {c_lbl} (Click cài đặt)"))
                        .font(piet::FontFamily::SANS_SERIF, 11.5 / zoom)
                        .text_color(piet::Color::rgba8(28, 113, 216, 255))
                        .build()
                    {
                        let badge_pos =
                            kurbo::Point::new(center.pos.x - 45.0 / zoom, center.pos.y - 20.0 / zoom);
                        cx.draw_text(&layout, badge_pos);
                    }
                }
            }

            // Lines in MathFigure
            for l in fig.lines.values() {
                if let (Some(p1), Some(p2)) = (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                    let k_line =
                        kurbo::Line::new(p1.pos.to_kurbo_point(), p2.pos.to_kurbo_point());
                    cx.stroke(k_line, &line_dash_color, 2.0 / zoom);

                    let circle_a = kurbo::Circle::new(p1.pos.to_kurbo_point(), 6.0 / zoom);
                    cx.fill(circle_a, &white_fill);
                    cx.stroke(circle_a, &border_color, 2.0 / zoom);

                    let circle_b = kurbo::Circle::new(p2.pos.to_kurbo_point(), 6.0 / zoom);
                    cx.fill(circle_b, &white_fill);
                    cx.stroke(circle_b, &border_color, 2.0 / zoom);

                    let m = (p1.pos + p2.pos) * 0.5;
                    let half = 4.5 / zoom;
                    let rect_m = kurbo::Rect::new(m.x - half, m.y - half, m.x + half, m.y + half);
                    cx.fill(rect_m, &white_fill);
                    cx.stroke(rect_m, &piet::Color::rgba8(46, 194, 126, 255), 2.0 / zoom);
                }
            }

            // Standalone points (MathPoint)
            if fig.circles.is_empty() && fig.lines.is_empty() {
                for p in fig.points.values() {
                    let circle_pt = kurbo::Circle::new(p.pos.to_kurbo_point(), 6.0 / zoom);
                    cx.fill(circle_pt, &white_fill);
                    cx.stroke(circle_pt, &border_color, 2.0 / zoom);

                    let lbl = p.label.as_deref().unwrap_or("A");
                    if let Ok(layout) = cx
                        .text()
                        .new_text_layout(format!("Điểm {lbl}"))
                        .font(piet::FontFamily::SANS_SERIF, 12.0 / zoom)
                        .text_color(piet::Color::rgba8(28, 113, 216, 255))
                        .build()
                    {
                        let badge_pos =
                            kurbo::Point::new(p.pos.x + 8.0 / zoom, p.pos.y - 18.0 / zoom);
                        cx.draw_text(&layout, badge_pos);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn draw_snap_overlay(
    cx: &mut piet_cairo::CairoRenderContext,
    snap: SnapCandidate,
    zoom: f64,
) -> anyhow::Result<()> {
    let pt = snap.point.to_kurbo_point();

    match snap.priority {
        SnapPriority::Intersection => {
            // Diamond for intersection
            let d = 7.0 / zoom;
            let mut diamond = kurbo::BezPath::new();
            diamond.move_to(kurbo::Point::new(pt.x, pt.y - d));
            diamond.line_to(kurbo::Point::new(pt.x + d, pt.y));
            diamond.line_to(kurbo::Point::new(pt.x, pt.y + d));
            diamond.line_to(kurbo::Point::new(pt.x - d, pt.y));
            diamond.close_path();

            cx.fill(&diamond, &piet::Color::rgba8(224, 27, 36, 210));
            cx.stroke(
                &diamond,
                &piet::Color::rgba8(255, 255, 255, 255),
                2.0 / zoom,
            );
        }
        SnapPriority::Tangent => {
            // Tangent Diamond
            let d = 7.0 / zoom;
            let mut diamond = kurbo::BezPath::new();
            diamond.move_to(kurbo::Point::new(pt.x, pt.y - d));
            diamond.line_to(kurbo::Point::new(pt.x + d, pt.y));
            diamond.line_to(kurbo::Point::new(pt.x, pt.y + d));
            diamond.line_to(kurbo::Point::new(pt.x - d, pt.y));
            diamond.close_path();
            cx.fill(&diamond, &piet::Color::rgba8(230, 97, 0, 220));
            cx.stroke(
                &diamond,
                &piet::Color::rgba8(255, 255, 255, 255),
                2.0 / zoom,
            );

            // Right-angle corner indicator
            if let SnapKind::Tangent { normal, .. } = snap.kind {
                let tangent = Vector2::new(-normal.y, normal.x);
                let s = 9.0 / zoom;
                let c1 = snap.point + normal * s;
                let c2 = snap.point + (normal + tangent) * s;
                let c3 = snap.point + tangent * s;
                let mut glyph = kurbo::BezPath::new();
                glyph.move_to(c1.to_kurbo_point());
                glyph.line_to(c2.to_kurbo_point());
                glyph.line_to(c3.to_kurbo_point());
                cx.stroke(glyph, &piet::Color::rgba8(230, 97, 0, 255), 2.0 / zoom);
            }
        }
        SnapPriority::Midpoint => {
            // Circular marker + tick mark
            let circle = kurbo::Circle::new(pt, 5.5 / zoom);
            cx.fill(circle, &piet::Color::rgba8(46, 194, 126, 220));
            cx.stroke(
                circle,
                &piet::Color::rgba8(255, 255, 255, 255),
                2.0 / zoom,
            );

            // Tick mark
            let t_len = 7.0 / zoom;
            let tick = kurbo::Line::new(
                kurbo::Point::new(pt.x - t_len, pt.y - t_len),
                kurbo::Point::new(pt.x + t_len, pt.y + t_len),
            );
            cx.stroke(
                tick,
                &piet::Color::rgba8(46, 194, 126, 255),
                2.0 / zoom,
            );
        }
        SnapPriority::KeyPoint => {
            let circle = kurbo::Circle::new(pt, 5.0 / zoom);
            cx.fill(circle, &piet::Color::rgba8(53, 132, 228, 220));
            cx.stroke(
                circle,
                &piet::Color::rgba8(255, 255, 255, 255),
                1.5 / zoom,
            );
        }
        SnapPriority::Perimeter => {
            let circle = kurbo::Circle::new(pt, 4.0 / zoom);
            cx.fill(circle, &piet::Color::rgba8(154, 153, 150, 180));
            cx.stroke(
                circle,
                &piet::Color::rgba8(255, 255, 255, 255),
                1.0 / zoom,
            );
        }
    }
    Ok(())
}

// ==========================================
// HIT TESTING & MANIPULATION HELPERS
// ==========================================

fn hit_test_stroke(
    pos: Vector2,
    store: &crate::store::StrokeStore,
    threshold: f64,
) -> Option<(StrokeKey, Shape)> {
    for key in store.stroke_keys_unordered() {
        if let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) {
            let hit = match &ss.shape {
                Shape::Line(line) => dist_to_segment(pos, line.start, line.end) <= threshold,
                Shape::Ellipse(ellipse) => {
                    let center = ellipse.affine.translation;
                    let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;
                    let d_center = (pos - center).length();
                    d_center <= threshold || (d_center - r).abs() <= threshold
                }
                Shape::MathFigure(fig) => {
                    let hit_pt = fig.points.values().any(|p| (p.pos - pos).length() <= threshold);
                    let hit_line = fig.lines.values().any(|l| {
                        if let (Some(p1), Some(p2)) = (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                            dist_to_segment(pos, p1.pos, p2.pos) <= threshold
                        } else {
                            false
                        }
                    });
                    let hit_circle = fig.circles.values().any(|c| {
                        if let Some(center) = fig.points.get(&c.center) {
                            let r = c
                                .through_point
                                .and_then(|tp| fig.points.get(&tp))
                                .map(|p| (p.pos - center.pos).length())
                                .unwrap_or(c.radius);
                            let d = (pos - center.pos).length();
                            d <= threshold || (d - r).abs() <= threshold
                        } else {
                            false
                        }
                    });
                    hit_pt || hit_line || hit_circle
                }
                _ => false,
            };
            if hit {
                return Some((key, ss.shape.clone()));
            }
        }
    }
    None
}

fn hit_test_handle(pos: Vector2, shape: &Shape, threshold: f64) -> Option<HandleKind> {
    match shape {
        Shape::Line(line) => {
            if (pos - line.start).length() <= threshold {
                return Some(HandleKind::LineStart);
            }
            if (pos - line.end).length() <= threshold {
                return Some(HandleKind::LineEnd);
            }
            let m = (line.start + line.end) * 0.5;
            if (pos - m).length() <= threshold {
                return Some(HandleKind::LineMid);
            }
            if dist_to_segment(pos, line.start, line.end) <= threshold {
                return Some(HandleKind::LineMid);
            }
            None
        }
        Shape::Ellipse(ellipse) => {
            let center = ellipse.affine.translation;
            let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;
            if (pos - center).length() <= threshold {
                return Some(HandleKind::CircleCenter);
            }
            let cardinal = [
                center + Vector2::new(r, 0.0),
                center - Vector2::new(r, 0.0),
                center + Vector2::new(0.0, r),
                center - Vector2::new(0.0, r),
            ];
            for (i, &p) in cardinal.iter().enumerate() {
                if (pos - p).length() <= threshold {
                    return Some(HandleKind::CirclePerimeter(i));
                }
            }
            if ((pos - center).length() - r).abs() <= threshold {
                return Some(HandleKind::CirclePerimeter(0));
            }
            None
        }
        Shape::MathFigure(fig) => {
            for c in fig.circles.values() {
                if let Some(center) = fig.points.get(&c.center) {
                    let r = c
                        .through_point
                        .and_then(|tp| fig.points.get(&tp))
                        .map(|p| (p.pos - center.pos).length())
                        .unwrap_or(c.radius);
                    if (pos - center.pos).length() <= threshold {
                        return Some(HandleKind::CircleCenter);
                    }
                    if let Some(through) = c.through_point.and_then(|tp| fig.points.get(&tp)) {
                        if (pos - through.pos).length() <= threshold {
                            return Some(HandleKind::CirclePerimeter(0));
                        }
                    }
                    if ((pos - center.pos).length() - r).abs() <= threshold {
                        return Some(HandleKind::CirclePerimeter(0));
                    }
                }
            }
            for l in fig.lines.values() {
                if let (Some(p1), Some(p2)) = (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                    if (pos - p1.pos).length() <= threshold {
                        return Some(HandleKind::LineStart);
                    }
                    if (pos - p2.pos).length() <= threshold {
                        return Some(HandleKind::LineEnd);
                    }
                    let m = (p1.pos + p2.pos) * 0.5;
                    if (pos - m).length() <= threshold
                        || dist_to_segment(pos, p1.pos, p2.pos) <= threshold
                    {
                        return Some(HandleKind::LineMid);
                    }
                }
            }
            for p in fig.points.values() {
                if (pos - p.pos).length() <= threshold {
                    return Some(HandleKind::CircleCenter);
                }
            }
            None
        }
        _ => None,
    }
}

fn apply_handle_drag(
    key: StrokeKey,
    handle: HandleKind,
    current_pos: Vector2,
    last_drag_pos: Option<Vector2>,
    engine_view: &mut EngineViewMut,
) {
    if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_mut(key) {
        match (&mut ss.shape, handle) {
            (Shape::Line(line), HandleKind::LineStart) => {
                line.start = current_pos;
            }
            (Shape::Line(line), HandleKind::LineEnd) => {
                line.end = current_pos;
            }
            (Shape::Line(line), HandleKind::LineMid) => {
                if let Some(prev) = last_drag_pos {
                    line.translate(current_pos - prev);
                }
            }
            (Shape::Ellipse(ellipse), HandleKind::CircleCenter) => {
                if let Some(prev) = last_drag_pos {
                    ellipse.translate(current_pos - prev);
                }
            }
            (Shape::Ellipse(ellipse), HandleKind::CirclePerimeter(_)) => {
                let center = ellipse.affine.translation;
                let new_radius = (current_pos - center).length().max(2.0);
                ellipse.radii = Vector2::new(new_radius, new_radius);
            }
            (Shape::MathFigure(fig), HandleKind::CircleCenter) => {
                if let Some(prev) = last_drag_pos {
                    fig.translate(current_pos - prev);
                }
            }
            (Shape::MathFigure(fig), HandleKind::CirclePerimeter(_)) => {
                for c in fig.circles.values_mut() {
                    if let Some(center) = fig.points.get(&c.center).map(|p| p.pos) {
                        let diff = current_pos - center;
                        let new_r = diff.length().max(5.0);
                        c.radius = new_r;
                        if let Some(through_id) = c.through_point {
                            if let Some(through) = fig.points.get_mut(&through_id) {
                                let dir = if new_r > 1e-6 {
                                    diff.normalize()
                                } else {
                                    Vector2::new(1.0, 0.0)
                                };
                                through.pos = center + dir * new_r;
                            }
                        }
                    }
                }
            }
            (Shape::MathFigure(fig), HandleKind::LineStart) => {
                if let Some(l) = fig.lines.values().next() {
                    let p1_id = l.p1;
                    if let Some(p) = fig.points.get_mut(&p1_id) {
                        p.pos = current_pos;
                    }
                }
            }
            (Shape::MathFigure(fig), HandleKind::LineEnd) => {
                if let Some(l) = fig.lines.values().next() {
                    let p2_id = l.p2;
                    if let Some(p) = fig.points.get_mut(&p2_id) {
                        p.pos = current_pos;
                    }
                }
            }
            (Shape::MathFigure(fig), HandleKind::LineMid) => {
                if let Some(prev) = last_drag_pos {
                    fig.translate(current_pos - prev);
                }
            }
            _ => {}
        }
    }
    engine_view.store.update_geometry_for_strokes(&[key]);
    engine_view.store.regenerate_rendering_for_stroke(
        key,
        engine_view.camera.viewport(),
        engine_view.camera.image_scale(),
    );
}

fn dist_to_segment(p: Vector2, a: Vector2, b: Vector2) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-9 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    let proj = a + ab * t;
    (p - proj).length()
}

fn collect_snap_candidates(
    store: &crate::store::StrokeStore,
    _current_line_start: Option<Vector2>,
) -> Vec<SnapCandidate> {
    let mut candidates = Vec::new();
    let mut lines: Vec<(Vector2, Vector2)> = Vec::new();
    let mut circles: Vec<(Vector2, f64)> = Vec::new();

    for key in store.stroke_keys_unordered() {
        if let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) {
            match &ss.shape {
                Shape::Line(line) => {
                    candidates.push(SnapCandidate::new(
                        line.start,
                        SnapKind::Point {
                            point: line.start,
                            label: None,
                        },
                        SnapPriority::KeyPoint,
                    ));
                    candidates.push(SnapCandidate::new(
                        line.end,
                        SnapKind::Point {
                            point: line.end,
                            label: None,
                        },
                        SnapPriority::KeyPoint,
                    ));
                    let m = compute_segment_midpoint(line.start, line.end);
                    candidates.push(SnapCandidate::new(
                        m,
                        SnapKind::Midpoint {
                            point: m,
                            a: line.start,
                            b: line.end,
                        },
                        SnapPriority::Midpoint,
                    ));
                    lines.push((line.start, line.end));
                }
                Shape::Ellipse(ellipse) => {
                    let center = ellipse.affine.translation;
                    let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;
                    candidates.push(SnapCandidate::new(
                        center,
                        SnapKind::Point {
                            point: center,
                            label: Some('O'),
                        },
                        SnapPriority::KeyPoint,
                    ));
                    if r > 1.0 {
                        circles.push((center, r));
                        let cardinal = [
                            center + Vector2::new(r, 0.0),
                            center - Vector2::new(r, 0.0),
                            center + Vector2::new(0.0, r),
                            center - Vector2::new(0.0, r),
                        ];
                        for p in cardinal {
                            candidates.push(SnapCandidate::new(
                                p,
                                SnapKind::OnPerimeter { point: p },
                                SnapPriority::Perimeter,
                            ));
                        }
                    }
                }
                Shape::MathFigure(fig) => {
                    for pt in fig.points.values() {
                        candidates.push(SnapCandidate::new(
                            pt.pos,
                            SnapKind::Point {
                                point: pt.pos,
                                label: pt.label.as_ref().and_then(|s| s.chars().next()),
                            },
                            SnapPriority::KeyPoint,
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    // Pairwise line-line intersections
    for i in 0..lines.len() {
        for j in (i + 1)..lines.len() {
            if let Some(pt) =
                compute_line_line_intersection(lines[i].0, lines[i].1, lines[j].0, lines[j].1)
            {
                candidates.push(SnapCandidate::new(
                    pt,
                    SnapKind::Intersection {
                        p1: pt,
                        p2: None,
                    },
                    SnapPriority::Intersection,
                ));
            }
        }
    }

    // Line-circle intersections and tangents
    for &(a, b) in &lines {
        for &(center, radius) in &circles {
            match compute_line_circle_intersections(a, b, center, radius, 3.0) {
                LineCircleResult::Tangent(pt) => {
                    let normal = (pt - center).normalize_or_zero();
                    candidates.push(SnapCandidate::new(
                        pt,
                        SnapKind::Tangent { point: pt, normal },
                        SnapPriority::Tangent,
                    ));
                }
                LineCircleResult::Secant(i1, i2) => {
                    candidates.push(SnapCandidate::new(
                        i1,
                        SnapKind::Intersection {
                            p1: i1,
                            p2: Some(i2),
                        },
                        SnapPriority::Intersection,
                    ));
                    candidates.push(SnapCandidate::new(
                        i2,
                        SnapKind::Intersection {
                            p1: i2,
                            p2: Some(i1),
                        },
                        SnapPriority::Intersection,
                    ));
                    let h = compute_chord_midpoint(center, a, b);
                    candidates.push(SnapCandidate::new(
                        h,
                        SnapKind::ChordMidpoint { point: h, center },
                        SnapPriority::Midpoint,
                    ));
                }
                LineCircleResult::Disjoint => {}
            }
        }
    }

    // Circle-circle intersections
    for i in 0..circles.len() {
        for j in (i + 1)..circles.len() {
            let (c1, r1) = circles[i];
            let (c2, r2) = circles[j];
            match compute_circle_circle_intersections(c1, r1, c2, r2, 3.0) {
                CircleCircleResult::Tangent(pt) => {
                    let normal = (c2 - c1).normalize_or_zero();
                    candidates.push(SnapCandidate::new(
                        pt,
                        SnapKind::Tangent { point: pt, normal },
                        SnapPriority::Tangent,
                    ));
                }
                CircleCircleResult::Secant(i1, i2) => {
                    candidates.push(SnapCandidate::new(
                        i1,
                        SnapKind::Intersection {
                            p1: i1,
                            p2: Some(i2),
                        },
                        SnapPriority::Intersection,
                    ));
                    candidates.push(SnapCandidate::new(
                        i2,
                        SnapKind::Intersection {
                            p1: i2,
                            p2: Some(i1),
                        },
                        SnapPriority::Intersection,
                    ));
                }
                CircleCircleResult::Disjoint => {}
            }
        }
    }

    candidates
}

fn snap_to_math_figure_points(pos: Vector2, store: &crate::store::StrokeStore) -> Vector2 {
    let snap_threshold_sq = 16.0 * 16.0;
    let mut closest_dist_sq = f64::MAX;
    let mut snapped_pos = pos;

    for key in store.stroke_keys_unordered() {
        if let Some(Stroke::ShapeStroke(shape_stroke)) = store.get_stroke_ref(key) {
            if let Shape::MathFigure(ref fig) = shape_stroke.shape {
                for pt in fig.points.values() {
                    let d_sq = (pt.pos - pos).length_squared();
                    if d_sq <= snap_threshold_sq && d_sq < closest_dist_sq {
                        closest_dist_sq = d_sq;
                        snapped_pos = pt.pos;
                    }
                }
            }
        }
    }
    snapped_pos
}

fn new_builder(
    shaper_config: &ShaperConfig,
    element: Element,
    now: Instant,
) -> Box<dyn Buildable<Emit = Shape>> {
    match shaper_config.builder_type {
        ShapeBuilderType::Arrow => Box::new(ArrowBuilder::start(element, now)),
        ShapeBuilderType::Line => Box::new(LineBuilder::start(element, now)),
        ShapeBuilderType::Rectangle => Box::new(RectangleBuilder::start(element, now)),
        ShapeBuilderType::Grid => Box::new(GridBuilder::start(element, now)),
        ShapeBuilderType::CoordSystem2D => Box::new(CoordSystem2DBuilder::start(element, now)),
        ShapeBuilderType::CoordSystem3D => Box::new(CoordSystem3DBuilder::start(element, now)),
        ShapeBuilderType::QuadrantCoordSystem2D => {
            Box::new(QuadrantCoordSystem2DBuilder::start(element, now))
        }
        ShapeBuilderType::Ellipse => Box::new(EllipseBuilder::start(element, now)),
        ShapeBuilderType::FociEllipse => Box::new(FociEllipseBuilder::start(element, now)),
        ShapeBuilderType::QuadBez => Box::new(QuadBezBuilder::start(element, now)),
        ShapeBuilderType::CubBez => Box::new(CubBezBuilder::start(element, now)),
        ShapeBuilderType::Polyline => Box::new(PolylineBuilder::start(element, now)),
        ShapeBuilderType::Polygon => Box::new(PolygonBuilder::start(element, now)),
        ShapeBuilderType::MathTriangle => Box::new(MathTriangleBuilder::start(element, now)),
        ShapeBuilderType::MathCircle => Box::new(MathCircleBuilder::start_with_labels(
            element,
            shaper_config.math_center_label.clone(),
            "M".to_string(),
        )),
        ShapeBuilderType::MathPoint => Box::new(MathPointBuilder::start_with_label(
            element,
            shaper_config.math_center_label.clone(),
        )),
        ShapeBuilderType::MathLine => Box::new(MathLineBuilder::start_with_config(
            element,
            Some("A".to_string()),
            Some("B".to_string()),
            shaper_config.math_auto_intersections,
        )),
    }
}
