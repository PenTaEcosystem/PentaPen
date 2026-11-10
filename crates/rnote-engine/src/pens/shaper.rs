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
use piet::RenderContext;
use rnote_compose::Shape;
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
use rnote_compose::geometry::{CircleId, MathFigure, PointId};
use rnote_compose::penevent::{KeyboardKey, ModifierKey, PenEvent, PenProgress};
use rnote_compose::penpath::Element;
use rnote_compose::shapes::Shapeable;
use rnote_compose::shapes::geometry_snap::{
    CircleCircleResult, LineCircleResult, SnapCandidate, SnapKind, SnapPriority,
    compute_chord_midpoint, compute_circle_circle_intersections, compute_line_circle_intersections,
    compute_line_line_intersection, compute_segment_midpoint, find_magnetic_snap,
};
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
    /// A MathFigure point. Circle centers carry the figure, other points go through the solver
    FigurePoint(PointId),
    /// Perimeter of one MathFigure circle - changes only that radius
    FigureRadius(CircleId),
}

/// Data for a shape currently selected for interactive manipulation.
#[derive(Debug, Clone)]
pub struct SelectedShapeData {
    pub key: StrokeKey,
    pub point: Option<PointId>,
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
}

impl Default for Shaper {
    fn default() -> Self {
        Self {
            state: ShaperState::Idle,
            hovered_stroke: None,
            active_snap: None,
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

        // Hover feedback. While building, Proximity must reach the builder (polyline/polygon previews).
        if let PenEvent::Proximity { element, .. } = &event
            && !matches!(self.state, ShaperState::BuildShape { .. })
        {
            let prev_snap = self.active_snap;
            snap_pos(
                &mut self.active_snap,
                element.pos,
                engine_view,
                None,
                snap_radius,
            );

            let new_hover = hit_test_stroke(element.pos, engine_view.store, hit_threshold);
            if new_hover != self.hovered_stroke || self.active_snap != prev_snap {
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
                // Check if user clicked directly on an existing shape
                if let Some(hit_key) =
                    hit_test_stroke(element.pos, engine_view.store, hit_threshold)
                {
                    self.state = ShaperState::Manipulating {
                        selected: SelectedShapeData {
                            key: hit_key,
                            point: figure_of(engine_view.store, hit_key).and_then(|fig| {
                                nearest_figure_point(fig, element.pos, hit_threshold)
                            }),
                        },
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

                    element.pos = snap_pos(
                        &mut self.active_snap,
                        element.pos,
                        engine_view,
                        None,
                        snap_radius,
                    );

                    self.state = ShaperState::BuildShape {
                        builder: new_builder(
                            &engine_view.config.pens_config.shaper_config,
                            engine_view.store,
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
                let builder_type = shortcut_builder_type(keyboard_key);
                if let Some(builder_type) = builder_type {
                    engine_view.config.pens_config.shaper_config.builder_type = builder_type;
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
                        selected.point = match h {
                            HandleKind::FigurePoint(id) => Some(id),
                            _ => None,
                        };
                        widget_flags.redraw = true;
                        *active_handle = Some(h);
                        *last_drag_pos = Some(element.pos);
                        // A click selects the handle; only subsequent motion moves it.
                        return (
                            EventResult {
                                handled: true,
                                propagate: EventPropagation::Stop,
                                progress: PenProgress::InProgress,
                            },
                            widget_flags,
                        );
                    } else if let Some(other_key) =
                        hit_test_stroke(element.pos, engine_view.store, hit_threshold)
                    {
                        // Switched selection to another shape
                        selected.key = other_key;
                        selected.point = figure_of(engine_view.store, other_key)
                            .and_then(|fig| nearest_figure_point(fig, element.pos, hit_threshold));
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
                        element.pos = snap_pos(
                            &mut self.active_snap,
                            element.pos,
                            engine_view,
                            None,
                            snap_radius,
                        );
                        self.state = ShaperState::BuildShape {
                            builder: new_builder(
                                &engine_view.config.pens_config.shaper_config,
                                engine_view.store,
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
                    // Exclude the dragged stroke so a handle can't snap to its own old position.
                    // Body translation (LineMid) follows the raw cursor, snapping it would make the shape jump.
                    if handle != HandleKind::LineMid {
                        element.pos = snap_pos(
                            &mut self.active_snap,
                            element.pos,
                            engine_view,
                            Some(selected.key),
                            snap_radius,
                        );
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
            ) if !modifier_keys.contains(&ModifierKey::KeyboardCtrl)
                && !modifier_keys.contains(&ModifierKey::KeyboardAlt) =>
            {
                let key = selected.key;
                let rename_target = selected.point;

                match keyboard_key {
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
                    // Renaming wins over tool shortcuts while a point is targeted, so L/R/S/P are valid names.
                    KeyboardKey::Unicode(c) if c.is_alphanumeric() && rename_target.is_some() => {
                        if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_mut(key)
                            && let Shape::MathFigure(fig) = &mut ss.shape
                            && let Some(pt) = rename_target.and_then(|id| fig.points.get_mut(&id))
                        {
                            pt.label = Some(c.to_uppercase().to_string());
                        }
                        // One key renames once; the next letter is a normal shortcut again.
                        selected.point = None;
                        engine_view.store.update_geometry_for_stroke(key);
                        engine_view.store.regenerate_rendering_for_stroke(
                            key,
                            engine_view.camera.viewport(),
                            engine_view.camera.image_scale(),
                        );
                        widget_flags |= engine_view.store.record(Instant::now());
                        widget_flags.store_modified = true;
                        widget_flags.redraw = true;
                        EventResult {
                            handled: true,
                            propagate: EventPropagation::Stop,
                            progress: PenProgress::InProgress,
                        }
                    }
                    k => match shortcut_builder_type(k) {
                        Some(builder_type) => {
                            engine_view.config.pens_config.shaper_config.builder_type =
                                builder_type;
                            self.state = ShaperState::Idle;
                            widget_flags.refresh_ui = true;
                            widget_flags.redraw = true;
                            EventResult {
                                handled: true,
                                propagate: EventPropagation::Stop,
                                progress: PenProgress::Idle,
                            }
                        }
                        None => EventResult {
                            handled: false,
                            propagate: EventPropagation::Proceed,
                            progress: PenProgress::Idle,
                        },
                    },
                }
            }

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
                    PenEvent::Down { element, .. }
                    | PenEvent::Up { element, .. }
                    | PenEvent::Proximity { element, .. } => {
                        element.pos = snap_pos(
                            &mut self.active_snap,
                            element.pos,
                            engine_view,
                            None,
                            snap_radius,
                        );
                        widget_flags.redraw = true;
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

                        // A click without drag must not leave stacked points behind
                        let shapes: Vec<Shape> = shapes
                            .into_iter()
                            .filter(|s| !is_redundant_figure(s, engine_view.store))
                            .collect();
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
                draw_manipulating_handles(cx, &ss.shape, zoom, selected.point)?;
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
    selected_point: Option<PointId>,
) -> anyhow::Result<()> {
    let border_color = piet::Color::rgba8(28, 113, 216, 255);
    let white_fill = piet::Color::rgba8(255, 255, 255, 255);
    let line_dash_color = piet::Color::rgba8(53, 132, 228, 160);

    match shape {
        Shape::Line(line) => {
            // Draw connecting highlight
            let k_line = kurbo::Line::new(line.start.to_kurbo_point(), line.end.to_kurbo_point());
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
            cx.stroke(rect_m, &piet::Color::rgba8(46, 194, 126, 255), 2.0 / zoom);
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
            for c in fig.circles.values() {
                if let Some(center) = fig.points.get(&c.center) {
                    let ring = kurbo::Circle::new(center.pos.to_kurbo_point(), c.radius);
                    cx.stroke(ring, &line_dash_color, 1.5 / zoom);
                }
            }
            for l in fig.lines.values() {
                if let (Some(p1), Some(p2)) = (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                    let k_line = kurbo::Line::new(p1.pos.to_kurbo_point(), p2.pos.to_kurbo_point());
                    cx.stroke(k_line, &line_dash_color, 2.0 / zoom);
                }
            }
            // Every point is a drag handle (see hit_test_handle)
            for p in fig.points.values() {
                let h = kurbo::Circle::new(p.pos.to_kurbo_point(), 6.0 / zoom);
                cx.fill(
                    h,
                    if selected_point == Some(p.id) {
                        &border_color
                    } else {
                        &white_fill
                    },
                );
                cx.stroke(h, &border_color, 2.0 / zoom);
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
            cx.stroke(circle, &piet::Color::rgba8(255, 255, 255, 255), 2.0 / zoom);

            // Tick mark
            let t_len = 7.0 / zoom;
            let tick = kurbo::Line::new(
                kurbo::Point::new(pt.x - t_len, pt.y - t_len),
                kurbo::Point::new(pt.x + t_len, pt.y + t_len),
            );
            cx.stroke(tick, &piet::Color::rgba8(46, 194, 126, 255), 2.0 / zoom);
        }
        SnapPriority::KeyPoint => {
            let circle = kurbo::Circle::new(pt, 5.0 / zoom);
            cx.fill(circle, &piet::Color::rgba8(53, 132, 228, 220));
            cx.stroke(circle, &piet::Color::rgba8(255, 255, 255, 255), 1.5 / zoom);
        }
        SnapPriority::Perimeter => {
            let circle = kurbo::Circle::new(pt, 4.0 / zoom);
            cx.fill(circle, &piet::Color::rgba8(154, 153, 150, 180));
            cx.stroke(circle, &piet::Color::rgba8(255, 255, 255, 255), 1.0 / zoom);
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
) -> Option<StrokeKey> {
    store.stroke_keys_unordered().into_iter().find(|&key| {
        let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) else {
            return false;
        };
        match &ss.shape {
            Shape::Line(line) => dist_to_segment(pos, line.start, line.end) <= threshold,
            Shape::Ellipse(ellipse) => {
                let center = ellipse.affine.translation;
                let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;
                let d_center = (pos - center).length();
                d_center <= threshold || (d_center - r).abs() <= threshold
            }
            Shape::MathFigure(_) => hit_test_handle(pos, &ss.shape, threshold).is_some(),
            _ => false,
        }
    })
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
            // Points first, nearest wins, so the grabbed handle never depends on HashMap order.
            if let Some(id) = nearest_figure_point(fig, pos, threshold) {
                return Some(HandleKind::FigurePoint(id));
            }
            for (&cid, c) in &fig.circles {
                if let Some(center) = fig.points.get(&c.center)
                    && ((pos - center.pos).length() - c.radius).abs() <= threshold
                {
                    return Some(HandleKind::FigureRadius(cid));
                }
            }
            fig.lines
                .values()
                .any(|l| match (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                    (Some(p1), Some(p2)) => dist_to_segment(pos, p1.pos, p2.pos) <= threshold,
                    _ => false,
                })
                .then_some(HandleKind::LineMid)
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
                // Absolute: the center lands exactly on the (snapped) cursor, no drift
                let offset = current_pos - ellipse.affine.translation;
                ellipse.translate(offset);
            }
            (Shape::Ellipse(ellipse), HandleKind::CirclePerimeter(_)) => {
                let center = ellipse.affine.translation;
                let new_radius = (current_pos - center).length().max(2.0);
                ellipse.radii = Vector2::new(new_radius, new_radius);
            }
            (Shape::MathFigure(fig), HandleKind::FigurePoint(id)) => {
                if fig.circles.values().any(|c| c.center == id) {
                    // Dragging a circle center carries the whole figure
                    if let Some(p) = fig.points.get(&id) {
                        let offset = current_pos - p.pos;
                        fig.translate(offset);
                    }
                } else {
                    fig.drag_point(id, current_pos);
                }
            }
            (Shape::MathFigure(fig), HandleKind::FigureRadius(cid)) => {
                if let Some(c) = fig.circles.get_mut(&cid)
                    && let Some(center) = fig.points.get(&c.center).map(|p| p.pos)
                {
                    let diff = current_pos - center;
                    c.radius = diff.length().max(5.0);
                    if let Some(through) = c.through_point.and_then(|id| fig.points.get_mut(&id)) {
                        let dir = diff
                            .try_normalize()
                            .unwrap_or_else(|| Vector2::new(1.0, 0.0));
                        through.pos = center + dir * c.radius;
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

fn figure_of(store: &crate::store::StrokeStore, key: StrokeKey) -> Option<&MathFigure> {
    match store.get_stroke_ref(key) {
        Some(Stroke::ShapeStroke(ss)) => match &ss.shape {
            Shape::MathFigure(fig) => Some(fig),
            _ => None,
        },
        _ => None,
    }
}

/// The figure point nearest to `pos`, if within `threshold`.
fn nearest_figure_point(fig: &MathFigure, pos: Vector2, threshold: f64) -> Option<PointId> {
    fig.points
        .iter()
        .map(|(&id, p)| (id, (p.pos - pos).length()))
        .filter(|&(_, d)| d <= threshold)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn shortcut_builder_type(key: KeyboardKey) -> Option<ShapeBuilderType> {
    match key {
        KeyboardKey::Unicode('l' | 'L') => Some(ShapeBuilderType::MathLine),
        KeyboardKey::Unicode('r' | 'R') => Some(ShapeBuilderType::MathCircle),
        KeyboardKey::Unicode('s' | 'S') => Some(ShapeBuilderType::Rectangle),
        KeyboardKey::Unicode('p' | 'P') => Some(ShapeBuilderType::MathPoint),
        _ => None,
    }
}

/// Magnetic snap, falling back to grid snap. Updates `active_snap` for the overlay.
/// `exclude` is the stroke being edited, so its own points cannot capture the cursor.
fn snap_pos(
    active_snap: &mut Option<SnapCandidate>,
    pos: Vector2,
    engine_view: &EngineViewMut,
    exclude: Option<StrokeKey>,
    snap_radius: f64,
) -> Vector2 {
    let candidates = collect_snap_candidates(engine_view.store, exclude);
    // Hysteresis: hold the previous snap a bit past the radius so it does not flicker at the edge.
    let held = active_snap.filter(|prev| {
        (prev.point - pos).length() <= snap_radius * 1.3
            && candidates.iter().any(|c| c.point == prev.point)
    });
    *active_snap = find_magnetic_snap(pos, &candidates, snap_radius).or(held);
    match *active_snap {
        Some(snap) => snap.point,
        None => engine_view.document.snap_position(pos, engine_view.config),
    }
}

fn collect_snap_candidates(
    store: &crate::store::StrokeStore,
    exclude: Option<StrokeKey>,
) -> Vec<SnapCandidate> {
    let mut candidates = Vec::new();
    let mut lines: Vec<(Vector2, Vector2)> = Vec::new();
    let mut circles: Vec<(Vector2, f64)> = Vec::new();

    let mut push_segment = |candidates: &mut Vec<SnapCandidate>, a: Vector2, b: Vector2| {
        let m = compute_segment_midpoint(a, b);
        candidates.push(SnapCandidate::new(
            m,
            SnapKind::Midpoint { point: m, a, b },
            SnapPriority::Midpoint,
        ));
        lines.push((a, b));
    };

    for key in store.stroke_keys_unordered() {
        if Some(key) == exclude {
            continue;
        }
        if let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) {
            match &ss.shape {
                Shape::Line(line) => {
                    for p in [line.start, line.end] {
                        candidates.push(SnapCandidate::new(
                            p,
                            SnapKind::Point {
                                point: p,
                                label: None,
                            },
                            SnapPriority::KeyPoint,
                        ));
                    }
                    push_segment(&mut candidates, line.start, line.end);
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
                    // Figure lines/circles take part in midpoint and intersection snapping too
                    for l in fig.lines.values() {
                        if let (Some(a), Some(b)) = (fig.points.get(&l.p1), fig.points.get(&l.p2)) {
                            push_segment(&mut candidates, a.pos, b.pos);
                        }
                    }
                    for c in fig.circles.values() {
                        if let Some(center) = fig.points.get(&c.center)
                            && c.radius > 1.0
                        {
                            circles.push((center.pos, c.radius));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    drop(push_segment);

    // Pairwise line-line intersections
    for i in 0..lines.len() {
        for j in (i + 1)..lines.len() {
            if let Some(pt) =
                compute_line_line_intersection(lines[i].0, lines[i].1, lines[j].0, lines[j].1)
            {
                candidates.push(SnapCandidate::new(
                    pt,
                    SnapKind::Intersection { p1: pt, p2: None },
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

fn new_builder(
    shaper_config: &ShaperConfig,
    store: &crate::store::StrokeStore,
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
            free_labels(store, 1).remove(0),
        )),
        ShapeBuilderType::MathPoint => Box::new(MathPointBuilder::start_with_label(
            element,
            shaper_config.math_center_label.clone(),
        )),
        ShapeBuilderType::MathLine => {
            let mut labels = free_labels(store, 2).into_iter();
            Box::new(MathLineBuilder::start_with_config(
                element,
                labels.next(),
                labels.next(),
                shaper_config.math_auto_intersections,
            ))
        }
    }
}

/// The first `n` single-letter labels not yet used by any figure point (O is kept for centers).
fn free_labels(store: &crate::store::StrokeStore, n: usize) -> Vec<String> {
    let used: std::collections::HashSet<String> = store
        .stroke_keys_unordered()
        .into_iter()
        .filter_map(|key| figure_of(store, key))
        .flat_map(|fig| fig.points.values().filter_map(|p| p.label.clone()))
        .collect();
    ('A'..='Z')
        .filter(|&c| c != 'O')
        .map(String::from)
        .chain((1..).map(|i| format!("P{i}")))
        .filter(|l| !used.contains(l))
        .take(n)
        .collect()
}

/// A figure that only repeats what is already there: all its points coincide
/// (line/circle from a click without drag), or a lone point placed on an existing point.
fn is_redundant_figure(shape: &Shape, store: &crate::store::StrokeStore) -> bool {
    const EPS: f64 = 1.0;
    let Shape::MathFigure(fig) = shape else {
        return false;
    };
    let Some(first) = fig.points.values().next().map(|p| p.pos) else {
        return true;
    };
    if fig.points.len() > 1 {
        return fig.points.values().all(|p| (p.pos - first).length() < EPS);
    }
    store
        .stroke_keys_unordered()
        .into_iter()
        .filter_map(|key| figure_of(store, key))
        .any(|other| {
            other
                .points
                .values()
                .any(|p| (p.pos - first).length() < EPS)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figure_handles_pick_the_right_entity() {
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(10.0, 0.0), Some("B".into()));
        fig.add_segment(a, b);
        let o = fig.add_point(Vector2::new(100.0, 0.0), Some("O".into()));
        let c = fig.add_circle_radius(o, 50.0);
        let shape = Shape::MathFigure(fig);

        // Both A and B are within the threshold: the nearer one wins, independent of map order
        assert_eq!(
            hit_test_handle(Vector2::new(7.0, 0.0), &shape, 8.0),
            Some(HandleKind::FigurePoint(b))
        );
        assert_eq!(
            hit_test_handle(Vector2::new(2.0, 0.0), &shape, 8.0),
            Some(HandleKind::FigurePoint(a))
        );
        // On the perimeter of that specific circle
        assert_eq!(
            hit_test_handle(Vector2::new(150.0, 1.0), &shape, 4.0),
            Some(HandleKind::FigureRadius(c))
        );
        assert_eq!(hit_test_handle(Vector2::new(5.0, 30.0), &shape, 4.0), None);
    }

    #[test]
    fn labels_and_click_duplicates() {
        let mut store = crate::store::StrokeStore::default();
        assert_eq!(free_labels(&store, 2), vec!["A", "B"]);

        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(50.0, 0.0), Some("B".into()));
        fig.add_segment(a, b);
        store.insert_stroke(
            Stroke::ShapeStroke(ShapeStroke::new(
                Shape::MathFigure(fig),
                rnote_compose::Style::default(),
            )),
            None,
        );
        // The next line gets fresh names instead of a second A/B
        assert_eq!(free_labels(&store, 2), vec!["C", "D"]);

        // Line from a click without drag: both endpoints on one spot
        let mut click_line = MathFigure::new();
        let p = click_line.add_point(Vector2::new(10.0, 10.0), Some("C".into()));
        let q = click_line.add_point(Vector2::new(10.0, 10.0), Some("D".into()));
        click_line.add_segment(p, q);
        assert!(is_redundant_figure(&Shape::MathFigure(click_line), &store));

        // A point dropped onto existing point B
        let mut on_b = MathFigure::new();
        on_b.add_point(Vector2::new(50.0, 0.0), Some("C".into()));
        assert!(is_redundant_figure(&Shape::MathFigure(on_b), &store));

        // A real new point elsewhere is kept
        let mut free = MathFigure::new();
        free.add_point(Vector2::new(200.0, 0.0), Some("C".into()));
        assert!(!is_redundant_figure(&Shape::MathFigure(free), &store));
    }
}
