// Imports
use super::PenBehaviour;
use super::PenStyle;
use crate::engine::{EngineView, EngineViewMut, StrokeContent};
use futures::channel::oneshot;
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
use rnote_compose::constraints::ConstraintRatio;
use rnote_compose::eventresult::{EventPropagation, EventResult};
use rnote_compose::ext::Vector2Ext;
use rnote_compose::geometry::{CircleId, LineId, LineStyle, MathFigure, PointId};
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
    /// Selected sub-segment (region between intersections/vertices) when clicked on a segment
    pub segment: Option<LineId>,
    /// Where the shape was last clicked: D restyles the line or circle there.
    pub click: Vector2,
}

#[derive(Debug)]
enum ShaperState {
    Idle,
    BuildShape {
        builder: Box<dyn Buildable<Emit = Shape>>,
        /// Shape whose point the press started on: selected instead when the press was only a click.
        pick: Option<SelectedShapeData>,
        start: Vector2,
        moved: bool,
        /// The figure that was selected when drawing started: what is drawn joins it
        into: Option<StrokeKey>,
    },
    Manipulating {
        selected: SelectedShapeData,
        active_handle: Option<HandleKind>,
        last_drag_pos: Option<Vector2>,
    },
}

/// Something the shaper needs typed in: the UI asks (see [WidgetFlags::shaper_prompt]) and calls back
/// into the engine (`Engine::add_point_on_line`, `Engine::add_circle_with_radius`).
#[derive(Debug, Clone)]
pub enum ShaperPrompt {
    /// The point tool tapped a line: name the new point on it
    PointOnLine {
        key: StrokeKey,
        line: LineId,
        pos: Vector2,
    },
    /// The circle tool tapped without dragging: type the radius around `center`
    /// (an existing figure point when `on` is set)
    CircleRadius {
        center: Vector2,
        on: Option<(StrokeKey, PointId)>,
    },
}

#[derive(Debug)]
pub struct Shaper {
    state: ShaperState,
    hovered_stroke: Option<StrokeKey>,
    active_snap: Option<SnapCandidate>,
    prompt: Option<ShaperPrompt>,
}

impl Default for Shaper {
    fn default() -> Self {
        Self {
            state: ShaperState::Idle,
            hovered_stroke: None,
            active_snap: None,
            prompt: None,
        }
    }
}

impl Shaper {
    /// Returns the stroke key currently selected for manipulation in Shaper, if any.
    pub fn selected_stroke_key(&self) -> Option<StrokeKey> {
        match &self.state {
            ShaperState::Manipulating { selected, .. } => Some(selected.key),
            _ => None,
        }
    }

    /// The input the last tap asked for, if any (taken once).
    pub fn take_prompt(&mut self) -> Option<ShaperPrompt> {
        self.prompt.take()
    }

    /// Point tool on a line (not on a point): ask for the name of a new point on that line.
    fn point_on_line_prompt(
        &mut self,
        pos: Vector2,
        engine_view: &EngineViewMut,
        line_threshold: f64,
        point_threshold: f64,
    ) -> bool {
        if engine_view.config.pens_config.shaper_config.builder_type != ShapeBuilderType::MathPoint
            || point_hit(pos, engine_view.store, point_threshold).is_some()
        {
            return false;
        }
        let Some(key) = hit_test_stroke(pos, engine_view.store, line_threshold) else {
            return false;
        };
        let Some(line) = figure_of(engine_view.store, key)
            .and_then(|fig| nearest_figure_segment(fig, pos, line_threshold))
        else {
            return false;
        };
        self.prompt = Some(ShaperPrompt::PointOnLine { key, line, pos });
        self.state = ShaperState::Idle;
        self.active_snap = None;
        true
    }

    /// Clear any active shape selection in Shaper.
    pub fn clear_selection(&mut self) {
        if matches!(self.state, ShaperState::Manipulating { .. }) {
            self.state = ShaperState::Idle;
            self.active_snap = None;
        }
    }

    /// Enter the state a press decided on.
    fn apply_press(
        &mut self,
        action: PressAction,
        mut element: Element,
        now: Instant,
        engine_view: &mut EngineViewMut,
        snap_radius: f64,
    ) {
        // Drawing while a figure is selected adds to that figure
        let into = match &self.state {
            ShaperState::Manipulating { selected, .. }
                if figure_of(engine_view.store, selected.key).is_some() =>
            {
                Some(selected.key)
            }
            _ => None,
        };
        self.state = match action {
            PressAction::Drag(handle, selected) => ShaperState::Manipulating {
                selected,
                active_handle: Some(handle),
                last_drag_pos: Some(element.pos),
            },
            PressAction::Select(selected) => {
                self.active_snap = None;
                ShaperState::Manipulating {
                    selected,
                    active_handle: None,
                    last_drag_pos: Some(element.pos),
                }
            }
            PressAction::Build { from, pick } => {
                engine_view
                    .config
                    .pens_config
                    .shaper_config
                    .new_style_seeds();
                element.pos = match from {
                    Some(p) => p,
                    None => snap_pos(
                        &mut self.active_snap,
                        element.pos,
                        engine_view,
                        None,
                        snap_radius,
                    ),
                };
                ShaperState::BuildShape {
                    builder: new_builder(
                        &engine_view.config.pens_config.shaper_config,
                        engine_view.store,
                        element,
                        now,
                        from.is_some(),
                    ),
                    pick,
                    start: element.pos,
                    moved: false,
                    into,
                }
            }
        };
    }
}

/// What a pen press does.
enum PressAction {
    /// Grab a handle of the selected shape
    Drag(HandleKind, SelectedShapeData),
    /// Select a shape (pressed on its body)
    Select(SelectedShapeData),
    /// Start a new shape, from an existing point when pressed on one
    Build {
        from: Option<Vector2>,
        pick: Option<SelectedShapeData>,
    },
}

/// Points win over bodies everywhere, so a point is easy to hit even next to a selected line.
/// Pressing an unselected point starts drawing from it; pressing the selected point drags it.
fn press_action(
    pos: Vector2,
    store: &crate::store::StrokeStore,
    selected: Option<&SelectedShapeData>,
    line_threshold: f64,
    point_threshold: f64,
) -> PressAction {
    let ph = point_hit(pos, store, point_threshold);
    if let Some(sel) = selected
        && let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(sel.key)
    {
        let drag = |h: HandleKind| {
            let point = match h {
                HandleKind::FigurePoint(id) => Some(id),
                _ => None,
            };
            let segment = point
                .is_none()
                .then(|| figure_of(store, sel.key))
                .flatten()
                .and_then(|fig| nearest_figure_segment(fig, pos, line_threshold));
            PressAction::Drag(
                h,
                SelectedShapeData {
                    key: sel.key,
                    point,
                    segment,
                    click: pos,
                },
            )
        };
        match ph {
            Some((key, Some(id), _)) if key == sel.key && sel.point == Some(id) => {
                return drag(HandleKind::FigurePoint(id));
            }
            // Plain line ends and ellipse centers of the selected shape stay drag handles
            Some((key, None, _)) if key == sel.key => {
                if let Some(h) = hit_test_handle(pos, &ss.shape, point_threshold) {
                    return drag(h);
                }
            }
            None => {
                if let Some(h) = hit_test_handle(pos, &ss.shape, line_threshold) {
                    return drag(h);
                }
            }
            _ => {}
        }
    }
    if let Some((key, point, at)) = ph {
        return PressAction::Build {
            from: Some(at),
            pick: Some(SelectedShapeData {
                key,
                point,
                segment: None,
                click: pos,
            }),
        };
    }
    match hit_test_stroke(pos, store, line_threshold) {
        Some(key) => PressAction::Select(SelectedShapeData {
            key,
            point: None,
            segment: figure_of(store, key)
                .and_then(|fig| nearest_figure_segment(fig, pos, line_threshold)),
            click: pos,
        }),
        None => PressAction::Build {
            from: None,
            pick: None,
        },
    }
}

/// The key point (figure point, line end, ellipse center) nearest to `pos` on the page, within `threshold`.
fn point_hit(
    pos: Vector2,
    store: &crate::store::StrokeStore,
    threshold: f64,
) -> Option<(StrokeKey, Option<PointId>, Vector2)> {
    let mut best: Option<(StrokeKey, Option<PointId>, Vector2)> = None;
    let consider =
        |key, id, p: Vector2, best: &mut Option<(StrokeKey, Option<PointId>, Vector2)>| {
            let d = (p - pos).length();
            if d <= threshold && best.is_none_or(|(_, _, b)| d < (b - pos).length()) {
                *best = Some((key, id, p));
            }
        };
    for key in store.stroke_keys_unordered() {
        let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) else {
            continue;
        };
        match &ss.shape {
            Shape::Line(line) => {
                consider(key, None, line.start, &mut best);
                consider(key, None, line.end, &mut best);
            }
            Shape::Ellipse(e) => consider(key, None, e.affine.translation, &mut best),
            Shape::MathFigure(fig) => {
                for (&id, p) in &fig.points {
                    consider(key, Some(id), p.pos, &mut best);
                }
            }
            _ => {}
        }
    }
    best
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

    /// Ctrl+C on a selected figure copies it, like the selector does.
    fn fetch_clipboard_content(
        &self,
        engine_view: &EngineView,
    ) -> oneshot::Receiver<anyhow::Result<(Vec<(Vec<u8>, String)>, WidgetFlags)>> {
        let content = match &self.state {
            ShaperState::Manipulating { selected, .. } => Some(
                engine_view
                    .store
                    .fetch_stroke_content(&[selected.key]),
            ),
            _ => None,
        };
        clipboard_content(content, WidgetFlags::default())
    }

    /// Ctrl+X on a selected figure: copy it, then remove it (undo brings it back).
    fn cut_clipboard_content(
        &mut self,
        engine_view: &mut EngineViewMut,
    ) -> oneshot::Receiver<anyhow::Result<(Vec<(Vec<u8>, String)>, WidgetFlags)>> {
        let mut widget_flags = WidgetFlags::default();
        let content = match &self.state {
            ShaperState::Manipulating { selected, .. } => {
                let c = engine_view.store.cut_stroke_content(&[selected.key]);
                self.state = ShaperState::Idle;
                self.hovered_stroke = None;
                self.active_snap = None;
                widget_flags |= engine_view.store.record(Instant::now());
                widget_flags.store_modified = true;
                widget_flags.redraw = true;
                Some(c)
            }
            _ => None,
        };
        clipboard_content(content, widget_flags)
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
        // Points get a wider catch radius than lines: they are tiny targets, especially with a pen
        let point_threshold = 24.0 / zoom;
        let snap_radius = 20.0 / zoom;

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
            (ShaperState::Idle, PenEvent::Down { element, .. }) => {
                if self.point_on_line_prompt(
                    element.pos,
                    engine_view,
                    hit_threshold,
                    point_threshold,
                ) {
                    widget_flags.shaper_prompt = true;
                    widget_flags.redraw = true;
                    return (
                        EventResult {
                            handled: true,
                            propagate: EventPropagation::Stop,
                            progress: PenProgress::Finished,
                        },
                        widget_flags,
                    );
                }
                let action = press_action(
                    element.pos,
                    engine_view.store,
                    None,
                    hit_threshold,
                    point_threshold,
                );
                self.apply_press(action, element, now, engine_view, snap_radius);
                widget_flags.redraw = true;
                EventResult {
                    handled: true,
                    propagate: EventPropagation::Stop,
                    progress: PenProgress::InProgress,
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
                    let sel = selected.clone();
                    if self.point_on_line_prompt(
                        element.pos,
                        engine_view,
                        hit_threshold,
                        point_threshold,
                    ) {
                        widget_flags.shaper_prompt = true;
                        widget_flags.redraw = true;
                        return (
                            EventResult {
                                handled: true,
                                propagate: EventPropagation::Stop,
                                progress: PenProgress::Finished,
                            },
                            widget_flags,
                        );
                    }
                    // A press only picks what to do; motion afterwards drags or draws.
                    let action = press_action(
                        element.pos,
                        engine_view.store,
                        Some(&sel),
                        hit_threshold,
                        point_threshold,
                    );
                    self.apply_press(action, element, now, engine_view, snap_radius);
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
                    // Delete the selected shape; undo brings it back.
                    KeyboardKey::Delete | KeyboardKey::BackSpace => {
                        engine_view.store.set_trashed(key, true);
                        self.state = ShaperState::Idle;
                        self.hovered_stroke = None;
                        self.active_snap = None;
                        widget_flags |= engine_view.store.record(Instant::now());
                        widget_flags.store_modified = true;
                        widget_flags.redraw = true;
                        EventResult {
                            handled: true,
                            propagate: EventPropagation::Stop,
                            progress: PenProgress::Finished,
                        }
                    }
                    // D: solid <-> dashed for the clicked line or circle
                    KeyboardKey::Unicode('d' | 'D') => {
                        let click = selected.click;
                        if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_mut(key)
                            && let Shape::MathFigure(fig) = &mut ss.shape
                        {
                            toggle_dashed(fig, click, hit_threshold);
                        }
                        engine_view.store.update_geometry_for_strokes(&[key]);
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

            (
                ShaperState::BuildShape {
                    builder,
                    pick,
                    start,
                    moved,
                    into,
                },
                mut event,
            ) => {
                let mut constraints = engine_view
                    .config
                    .pens_config
                    .shaper_config
                    .constraints
                    .clone();
                let shift_held = match &event {
                    PenEvent::Down { modifier_keys, .. }
                    | PenEvent::Up { modifier_keys, .. }
                    | PenEvent::Proximity { modifier_keys, .. }
                    | PenEvent::KeyPressed { modifier_keys, .. } => {
                        modifier_keys.contains(&ModifierKey::KeyboardShift)
                    }
                    PenEvent::Text { .. } | PenEvent::Cancel => false,
                };
                constraints.enabled = match &event {
                    PenEvent::Down { modifier_keys, .. }
                    | PenEvent::Up { modifier_keys, .. }
                    | PenEvent::Proximity { modifier_keys, .. }
                    | PenEvent::KeyPressed { modifier_keys, .. } => {
                        shift_held
                            || (constraints.enabled
                                ^ modifier_keys.contains(&ModifierKey::KeyboardCtrl))
                    }
                    PenEvent::Text { .. } | PenEvent::Cancel => false,
                };
                if shift_held {
                    constraints.ratios.insert(ConstraintRatio::OneToOne);
                }

                // Magnetic Snapping during building
                match &mut event {
                    PenEvent::Down { element, .. }
                    | PenEvent::Up { element, .. }
                    | PenEvent::Proximity { element, .. } => {
                        *moved |= (element.pos - *start).length() > 8.0 / zoom;
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

                        // Circle tool tapped without dragging: ask for the radius instead of guessing one
                        let circle_tap = !*moved
                            && engine_view.config.pens_config.shaper_config.builder_type
                                == ShapeBuilderType::MathCircle
                            && matches!(event, PenEvent::Up { .. });
                        if circle_tap {
                            let on = pick.take().and_then(|p| Some((p.key, p.point?)));
                            self.prompt = Some(ShaperPrompt::CircleRadius { center: *start, on });
                            widget_flags.shaper_prompt = true;
                        }
                        // A click on an existing point selects it instead of drawing.
                        let clicked_pick = pick.take().filter(|_| !*moved);
                        // A click without drag must not leave stacked points behind
                        let shapes: Vec<Shape> = shapes
                            .into_iter()
                            .filter(|_| clicked_pick.is_none() && !circle_tap)
                            .filter(|s| !is_redundant_figure(s, engine_view.store))
                            .collect();
                        let shapes_emitted = !shapes.is_empty();
                        let shaper_config = &engine_view.config.pens_config.shaper_config;
                        let dash = math_line_style(shaper_config.smooth_options.line_style);
                        let show_points = shaper_config.math_show_points;

                        for mut shape in shapes {
                            // Promote plain Line to MathFigure when auto-intersections are enabled
                            // so crossings with existing lines/circles pop up and split into selectable regions
                            if let Shape::Line(ref line) = shape
                                && engine_view
                                    .config
                                    .pens_config
                                    .shaper_config
                                    .math_auto_intersections
                                && (line.end - line.start).length() >= 2.0
                            {
                                let mut labels = free_labels(engine_view.store, 2).into_iter();
                                let mut fig = MathFigure::new();
                                fig.build_line(
                                    line.start,
                                    line.end,
                                    labels.next().as_deref(),
                                    labels.next().as_deref(),
                                );
                                shape = Shape::MathFigure(fig);
                            }

                            if let Shape::MathFigure(ref mut fig) = shape {
                                prepare_new_figure(fig, engine_view.store, dash, show_points);
                            }

                            if let Shape::MathFigure(ref mut fig) = shape {
                                if engine_view
                                    .config
                                    .pens_config
                                    .shaper_config
                                    .math_auto_intersections
                                {
                                    fig.auto_detect_intersections();
                                    let (segments, circles) = page_geometry(engine_view.store);
                                    let cross_pts = fig.add_intersections_with(&segments, &circles);
                                    if !cross_pts.is_empty() {
                                        let cross_coords: Vec<Vector2> = cross_pts
                                            .iter()
                                            .filter_map(|id| fig.points.get(id).map(|p| p.pos))
                                            .collect();
                                        propagate_intersections_to_existing_strokes(
                                            &cross_coords,
                                            engine_view,
                                        );
                                    }
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
                            if let (Some(target), Shape::MathFigure(new_fig)) = (*into, &shape)
                                && let Some(Stroke::ShapeStroke(ss)) =
                                    engine_view.store.get_stroke_mut(target)
                                && let Shape::MathFigure(fig) = &mut ss.shape
                            {
                                fig.absorb(new_fig);
                                if engine_view
                                    .config
                                    .pens_config
                                    .shaper_config
                                    .math_auto_intersections
                                {
                                    fig.auto_detect_intersections();
                                }
                                engine_view.store.update_geometry_for_strokes(&[target]);
                                engine_view.store.regenerate_rendering_for_stroke(
                                    target,
                                    engine_view.camera.viewport(),
                                    engine_view.camera.image_scale(),
                                );
                                continue;
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

                        // A figure drawn into stays selected, so drawing can go on.
                        // Otherwise no auto-selection after drawing: the next press on any point draws from it.
                        let keep = into
                            .filter(|_| shapes_emitted && clicked_pick.is_none())
                            .filter(|&k| figure_of(engine_view.store, k).is_some())
                            .map(|key| SelectedShapeData {
                                key,
                                point: None,
                                segment: None,
                                click: *start,
                            });
                        self.state = match clicked_pick.or(keep) {
                            Some(selected) => ShaperState::Manipulating {
                                selected,
                                active_handle: None,
                                last_drag_pos: None,
                            },
                            None => ShaperState::Idle,
                        };
                        self.active_snap = None;

                        if shapes_emitted
                            && engine_view.config.pens_config.shaper_config.builder_type
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
                draw_manipulating_handles(cx, &ss.shape, zoom, selected.point, selected.segment)?;
            }
        }

        // 3. Current builder shape + live intersection pop-ups while drawing a line
        if let ShaperState::BuildShape { builder, .. } = &self.state {
            builder.draw_styled(cx, &style, zoom);
            if let Some((p1, p2)) = builder.active_segment()
                && (p2 - p1).length() >= 2.0
            {
                let (segments, circles) = page_geometry(engine_view.store);
                let mut preview = MathFigure::new();
                preview.build_line(p1, p2, None, None);
                let inter_ids = preview.add_intersections_with(&segments, &circles);
                for pid in inter_ids {
                    if let Some(pt) = preview.points.get(&pid) {
                        let p = pt.pos.to_kurbo_point();
                        let halo = kurbo::Circle::new(p, 9.5 / zoom);
                        cx.fill(halo, &piet::Color::rgba8(224, 27, 36, 75));
                        let ring = kurbo::Circle::new(p, 6.2 / zoom);
                        cx.fill(ring, &piet::Color::rgba8(255, 255, 255, 250));
                        cx.stroke(ring, &piet::Color::rgba8(224, 27, 36, 240), 2.2 / zoom);
                        let dot = kurbo::Circle::new(p, 3.0 / zoom);
                        cx.fill(dot, &piet::Color::rgba8(224, 27, 36, 255));
                    }
                }
            }
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
    selected_segment: Option<LineId>,
) -> anyhow::Result<()> {
    let border_color = piet::Color::rgba8(28, 113, 216, 255);
    let white_fill = piet::Color::rgba8(255, 255, 255, 255);
    let line_dash_color = piet::Color::rgba8(53, 132, 228, 160);
    let active_seg_color = piet::Color::rgba8(230, 97, 0, 230);

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
            let rx = ellipse.radii.x;
            let ry = ellipse.radii.y;

            // Highlight ring
            cx.stroke(ellipse.outline_path(), &line_dash_color, 1.5 / zoom);

            // Center handle O
            let circle_o = kurbo::Circle::new(center.to_kurbo_point(), 6.0 / zoom);
            cx.fill(circle_o, &white_fill);
            cx.stroke(circle_o, &border_color, 2.0 / zoom);
            let dot_o = kurbo::Circle::new(center.to_kurbo_point(), 2.0 / zoom);
            cx.fill(dot_o, &border_color);

            // Radius guide line to right handle
            let right_p = center + Vector2::new(rx, 0.0);
            cx.stroke(
                kurbo::Line::new(center.to_kurbo_point(), right_p.to_kurbo_point()),
                &line_dash_color,
                1.5 / zoom,
            );

            // 4 Perimeter handles
            let cardinal = [
                center + Vector2::new(rx, 0.0),
                center - Vector2::new(rx, 0.0),
                center + Vector2::new(0.0, ry),
                center - Vector2::new(0.0, ry),
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
                    if selected_segment == Some(l.id) {
                        cx.stroke(k_line, &active_seg_color, 4.2 / zoom);
                    } else {
                        cx.stroke(k_line, &line_dash_color, 2.0 / zoom);
                    }
                }
            }
            // Every point is a drag handle (see hit_test_handle)
            let seg_endpoints = selected_segment
                .and_then(|lid| fig.lines.get(&lid))
                .map(|l| (l.p1, l.p2));
            for p in fig.points.values() {
                let is_active = selected_point == Some(p.id)
                    || seg_endpoints.is_some_and(|(a, b)| p.id == a || p.id == b);
                let radius = if p.label.is_none() || is_active {
                    7.0 / zoom
                } else {
                    6.0 / zoom
                };
                let h = kurbo::Circle::new(p.pos.to_kurbo_point(), radius);
                cx.fill(
                    h,
                    if selected_point == Some(p.id) {
                        &border_color
                    } else if seg_endpoints.is_some_and(|(a, b)| p.id == a || p.id == b) {
                        &active_seg_color
                    } else {
                        &white_fill
                    },
                );
                cx.stroke(h, &border_color, 2.2 / zoom);
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
    let mut best_point_hit: Option<(StrokeKey, f64)> = None;
    let mut best_body_hit: Option<(StrokeKey, f64)> = None;

    for key in store.stroke_keys_unordered() {
        let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) else {
            continue;
        };
        match &ss.shape {
            Shape::Line(line) => {
                let d_end = (pos - line.start).length().min((pos - line.end).length());
                if d_end <= threshold {
                    if best_point_hit.is_none_or(|(_, best)| d_end < best) {
                        best_point_hit = Some((key, d_end));
                    }
                } else {
                    let d_seg = dist_to_segment(pos, line.start, line.end);
                    if d_seg <= threshold && best_body_hit.is_none_or(|(_, best)| d_seg < best) {
                        best_body_hit = Some((key, d_seg));
                    }
                }
            }
            Shape::Ellipse(ellipse) => {
                let center = ellipse.affine.translation;
                let r = (ellipse.radii.x + ellipse.radii.y) * 0.5;
                let d_center = (pos - center).length();
                if d_center <= threshold {
                    if best_point_hit.is_none_or(|(_, best)| d_center < best) {
                        best_point_hit = Some((key, d_center));
                    }
                } else {
                    let d_ring = (d_center - r).abs();
                    if d_ring <= threshold && best_body_hit.is_none_or(|(_, best)| d_ring < best) {
                        best_body_hit = Some((key, d_ring));
                    }
                }
            }
            Shape::MathFigure(fig) => {
                if let Some(pid) = nearest_figure_point(fig, pos, threshold)
                    && let Some(pt) = fig.points.get(&pid)
                {
                    let d = (pos - pt.pos).length();
                    if best_point_hit.is_none_or(|(_, best)| d < best) {
                        best_point_hit = Some((key, d));
                    }
                } else if hit_test_handle(pos, &ss.shape, threshold).is_some() {
                    let d_seg = fig
                        .lines
                        .values()
                        .filter_map(|l| {
                            Some(dist_to_segment(
                                pos,
                                fig.points.get(&l.p1)?.pos,
                                fig.points.get(&l.p2)?.pos,
                            ))
                        })
                        .chain(fig.circles.values().filter_map(|c| {
                            Some(((pos - fig.points.get(&c.center)?.pos).length() - c.radius).abs())
                        }))
                        .fold(threshold, f64::min);
                    if best_body_hit.is_none_or(|(_, best)| d_seg < best) {
                        best_body_hit = Some((key, d_seg));
                    }
                }
            }
            _ => {}
        }
    }

    best_point_hit
        .map(|(k, _)| k)
        .or_else(|| best_body_hit.map(|(k, _)| k))
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
            let rx = ellipse.radii.x;
            let ry = ellipse.radii.y;
            let r = (rx + ry) * 0.5;
            if (pos - center).length() <= threshold {
                return Some(HandleKind::CircleCenter);
            }
            let cardinal = [
                center + Vector2::new(rx, 0.0),
                center - Vector2::new(rx, 0.0),
                center + Vector2::new(0.0, ry),
                center - Vector2::new(0.0, ry),
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
    let mut synced_old_pos: Option<Vector2> = None;
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
                    synced_old_pos = fig.points.get(&id).map(|p| p.pos);
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

    let mut updated_keys = vec![key];
    // If an intersection/shared point was dragged, also move any coincident point on other figures
    // so all lines meeting at an intersection adjust together smoothly.
    if let Some(old_pos) = synced_old_pos {
        for other_key in engine_view.store.stroke_keys_unordered() {
            if other_key == key {
                continue;
            }
            let mut moved = false;
            if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_mut(other_key)
                && let Shape::MathFigure(other_fig) = &mut ss.shape
                && let Some(other_pt_id) = other_fig.find_point_near(old_pos, 2.5)
                && !other_fig.circles.values().any(|c| c.center == other_pt_id)
            {
                other_fig.drag_point(other_pt_id, current_pos);
                moved = true;
            }
            if moved {
                updated_keys.push(other_key);
            }
        }
    }

    engine_view.store.update_geometry_for_strokes(&updated_keys);
    for k in updated_keys {
        engine_view.store.regenerate_rendering_for_stroke(
            k,
            engine_view.camera.viewport(),
            engine_view.camera.image_scale(),
        );
    }
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

/// Cycle solid -> dashed -> dotted -> dash-dot for the line or circle nearest to `pos` (within `threshold`),
/// or for all lines of the figure when the click was not on one (e.g. on a point).
fn toggle_dashed(fig: &mut MathFigure, pos: Vector2, threshold: f64) {
    let flip = |s: LineStyle| match s {
        LineStyle::Solid => LineStyle::Dashed,
        LineStyle::Dashed => LineStyle::Dotted,
        LineStyle::Dotted => LineStyle::DashDot,
        LineStyle::DashDot => LineStyle::Solid,
    };
    let at = |id: PointId| fig.points.get(&id).map(|p| p.pos);
    let line = fig
        .lines
        .values()
        .filter_map(|l| Some((l.id, dist_to_segment(pos, at(l.p1)?, at(l.p2)?))))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let circle = fig
        .circles
        .values()
        .filter_map(|c| Some((c.id, ((pos - at(c.center)?).length() - c.radius).abs())))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match (line, circle) {
        (Some((l, dl)), c) if dl <= threshold && c.is_none_or(|(_, dc)| dl <= dc) => {
            let line = fig.lines.get_mut(&l).unwrap();
            line.style = flip(line.style);
        }
        (_, Some((c, dc))) if dc <= threshold => {
            let circle = fig.circles.get_mut(&c).unwrap();
            circle.style = flip(circle.style);
        }
        _ => {
            let style = fig.lines.values().next().map(|l| flip(l.style));
            for l in fig.lines.values_mut() {
                l.style = style.unwrap_or(LineStyle::Dashed);
            }
        }
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

/// The figure sub-segment (region between intersections/vertices) nearest to `pos`, if within `threshold`.
fn nearest_figure_segment(fig: &MathFigure, pos: Vector2, threshold: f64) -> Option<LineId> {
    fig.lines
        .values()
        .filter(|l| l.visible)
        .filter_map(|l| {
            let p1 = fig.points.get(&l.p1)?.pos;
            let p2 = fig.points.get(&l.p2)?.pos;
            let d = dist_to_segment(pos, p1, p2);
            (d <= threshold).then_some((l.id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Propagate newly discovered intersection coordinates onto any crossed existing strokes on the page,
/// splitting their crossed segments so both lines share the intersection point and sub-segments.
fn propagate_intersections_to_existing_strokes(
    intersections: &[Vector2],
    engine_view: &mut EngineViewMut,
) {
    if intersections.is_empty() {
        return;
    }
    let keys = engine_view.store.stroke_keys_unordered();
    let mut modified_keys = Vec::new();

    for key in keys {
        let mut changed = false;
        if let Some(Stroke::ShapeStroke(ss)) = engine_view.store.get_stroke_mut(key) {
            // Upgrade plain Shape::Line to Shape::MathFigure if an intersection lies on it
            if let Shape::Line(line) = &ss.shape {
                let hits_line = intersections
                    .iter()
                    .any(|&pt| dist_to_segment(pt, line.start, line.end) <= 1.5);
                if hits_line {
                    let mut fig = MathFigure::new();
                    fig.build_line(line.start, line.end, None, None);
                    ss.shape = Shape::MathFigure(fig);
                    changed = true;
                }
            }

            if let Shape::MathFigure(fig) = &mut ss.shape {
                for &pt in intersections {
                    let on_seg = fig.lines.values().any(|l| {
                        let Some(p1) = fig.points.get(&l.p1) else {
                            return false;
                        };
                        let Some(p2) = fig.points.get(&l.p2) else {
                            return false;
                        };
                        dist_to_segment(pt, p1.pos, p2.pos) <= 1.5
                    });
                    let on_circ = fig.circles.values().any(|c| {
                        let Some(center) = fig.points.get(&c.center) else {
                            return false;
                        };
                        ((pt - center.pos).length() - c.radius).abs() <= 1.5
                    });

                    if on_seg || on_circ {
                        let pt_id = fig
                            .find_point_near(pt, 0.5)
                            .unwrap_or_else(|| fig.add_point(pt, None));
                        fig.split_segments_at_point(pt_id, 1.5);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            modified_keys.push(key);
        }
    }

    if !modified_keys.is_empty() {
        engine_view
            .store
            .update_geometry_for_strokes(&modified_keys);
        for key in modified_keys {
            engine_view.store.regenerate_rendering_for_stroke(
                key,
                engine_view.camera.viewport(),
                engine_view.camera.image_scale(),
            );
        }
    }
}

/// The clipboard formats of copied strokes (Rnote strokes, Svg, Png), made off the main thread.
#[allow(clippy::type_complexity)]
fn clipboard_content(
    content: Option<StrokeContent>,
    widget_flags: WidgetFlags,
) -> oneshot::Receiver<anyhow::Result<(Vec<(Vec<u8>, String)>, WidgetFlags)>> {
    let (sender, receiver) = oneshot::channel();
    rayon::spawn(move || {
        let result = move || {
            let mut out = Vec::new();
            if let Some(content) = content {
                let svg =
                    content.gen_svg(false, false, false, StrokeContent::CLIPBOARD_EXPORT_MARGIN)?;
                out.push((
                    serde_json::to_string(&content)?.into_bytes(),
                    StrokeContent::MIME_TYPE.to_string(),
                ));
                if let Some(svg) = svg {
                    let png = svg
                        .gen_image(crate::Engine::STROKE_EXPORT_IMAGE_SCALE)?
                        .into_encoded_bytes(image::ImageFormat::Png, None)?;
                    out.push((svg.to_document().into_bytes(), crate::Svg::MIME_TYPE.to_string()));
                    out.push((png, String::from("image/png")));
                }
            }
            Ok((out, widget_flags))
        };
        if sender.send(result()).is_err() {
            tracing::error!("Sending shaper clipboard content failed, receiver already dropped.");
        }
    });
    receiver
}

fn shortcut_builder_type(key: KeyboardKey) -> Option<ShapeBuilderType> {
    match key {
        KeyboardKey::Unicode('l' | 'L') => Some(ShapeBuilderType::MathLine),
        KeyboardKey::Unicode('r' | 'R' | 'c' | 'C') => Some(ShapeBuilderType::MathCircle),
        KeyboardKey::Unicode('t' | 'T') => Some(ShapeBuilderType::MathTriangle),
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
    // Starts on an existing point: that point keeps its name, the new start gets none
    from_existing: bool,
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
        // Vertices stay unnamed: name the ones you need with the point tool or the point editor
        ShapeBuilderType::MathTriangle => Box::new(MathTriangleBuilder::start_with_kind(
            element,
            shaper_config.math_shape_kind,
        )),
        ShapeBuilderType::MathCircle => {
            // Center: the configured name unless a point already has it. Through point: next free name.
            let configured = &shaper_config.math_center_label;
            let mut free = free_labels(store, 3)
                .into_iter()
                .filter(|l| l != configured);
            let center = if from_existing {
                String::new()
            } else if label_in_use(store, configured) {
                free.next().unwrap_or_default()
            } else {
                configured.clone()
            };
            Box::new(MathCircleBuilder::start_with_labels(
                element,
                center,
                free.next().unwrap_or_default(),
            ))
        }
        ShapeBuilderType::MathPoint => Box::new(MathPointBuilder::start_with_label(
            element,
            shaper_config.math_center_label.clone(),
        )),
        ShapeBuilderType::MathLine => {
            let mut labels = free_labels(store, 2).into_iter();
            let start_label = if from_existing { None } else { labels.next() };
            Box::new(MathLineBuilder::start_with_config(
                element,
                start_label,
                labels.next(),
                shaper_config.math_auto_intersections,
            ))
        }
    }
}

/// Segments and circles of all shapes on the page, for crossing points with a new figure.
fn page_geometry(
    store: &crate::store::StrokeStore,
) -> (Vec<(Vector2, Vector2)>, Vec<(Vector2, f64)>) {
    let mut segments = Vec::new();
    let mut circles = Vec::new();
    for key in store.stroke_keys_unordered() {
        let Some(Stroke::ShapeStroke(ss)) = store.get_stroke_ref(key) else {
            continue;
        };
        match &ss.shape {
            Shape::Line(line) => segments.push((line.start, line.end)),
            // Only round ellipses: an ellipse treated as a circle would produce phantom crossings
            Shape::Ellipse(e)
                if (e.radii.x - e.radii.y).abs() <= 0.01 * e.radii.x.max(e.radii.y) =>
            {
                circles.push((e.affine.translation, e.radii.x))
            }
            Shape::MathFigure(fig) => {
                segments.extend(fig.lines.values().filter_map(|l| {
                    Some((fig.points.get(&l.p1)?.pos, fig.points.get(&l.p2)?.pos))
                }));
                circles.extend(
                    fig.circles
                        .values()
                        .filter_map(|c| Some((fig.points.get(&c.center)?.pos, c.radius))),
                );
            }
            _ => {}
        }
    }
    (segments, circles)
}

fn label_in_use(store: &crate::store::StrokeStore, label: &str) -> bool {
    store
        .stroke_keys_unordered()
        .into_iter()
        .filter_map(|key| figure_of(store, key))
        .any(|fig| {
            fig.points
                .values()
                .any(|p| p.label.as_deref() == Some(label))
        })
}

/// The first `n` single-letter labels not yet used by any figure point (O is kept for centers).
pub(crate) fn free_labels(store: &crate::store::StrokeStore, n: usize) -> Vec<String> {
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

/// The figure line style for a shaper line style: dotted and dash-dot stay, any other dash is dashed.
pub(crate) fn math_line_style(style: rnote_compose::style::smooth::LineStyle) -> LineStyle {
    use rnote_compose::style::smooth::LineStyle as S;
    match style {
        S::Solid => LineStyle::Solid,
        S::Dotted => LineStyle::Dotted,
        S::DashedNarrow | S::DashedEquidistant | S::DashedWide => LineStyle::Dashed,
        S::DashDot => LineStyle::DashDot,
    }
}

/// Style a just-drawn figure and join it to the page: lines/circles take the chosen line style,
/// points are hidden in "no points" mode, and a point drawn onto an existing point is merged
/// into it (hidden, unnamed), so a line drawn from A starts at A instead of a second label there.
fn prepare_new_figure(
    fig: &mut MathFigure,
    store: &crate::store::StrokeStore,
    dash: LineStyle,
    show_points: bool,
) {
    if dash != LineStyle::Solid {
        fig.lines.values_mut().for_each(|l| l.style = dash);
        fig.circles.values_mut().for_each(|c| c.style = dash);
    }
    let existing: Vec<Vector2> = store
        .stroke_keys_unordered()
        .into_iter()
        .filter_map(|key| figure_of(store, key))
        .flat_map(|f| f.points.values().map(|p| p.pos).collect::<Vec<_>>())
        .collect();
    let mut merged = false;
    for p in fig.points.values_mut() {
        if !show_points || existing.iter().any(|e| (*e - p.pos).length() < 0.5) {
            merged |= p.label.is_some() && show_points;
            p.label = None;
            p.visible = false;
        }
    }
    // Names shift down so a line from A to a new point is "AC", not "AD"
    if merged {
        let mut ids: Vec<PointId> = fig
            .points
            .values()
            .filter(|p| p.label.as_deref().is_some_and(|l| l != "O"))
            .map(|p| p.id)
            .collect();
        ids.sort();
        for (id, label) in ids.iter().zip(free_labels(store, ids.len())) {
            fig.points.get_mut(id).unwrap().label = Some(label);
        }
    }
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

    #[test]
    fn point_on_line_and_circle_from_radius() {
        use rnote_compose::geometry::Constraint;
        let mut engine = crate::Engine::default();
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), None);
        let b = fig.add_point(Vector2::new(100.0, 0.0), None);
        let line = fig.add_segment(a, b);
        let _ = engine.insert_shape(Shape::MathFigure(fig));
        let key = engine.store.stroke_keys_unordered()[0];

        // Tapped a bit off the line: the point lands on it, splits it and is tied to it
        let _ = engine.add_point_on_line(key, line, Vector2::new(30.0, 7.0), Some("M".into()));
        let fig = figure_of(&engine.store, key).unwrap();
        let m = fig.find_point_by_label("M").unwrap();
        assert_eq!(fig.points[&m].pos, Vector2::new(30.0, 0.0));
        assert_eq!(fig.lines.len(), 2);
        assert!(
            fig.constraints
                .iter()
                .any(|c| matches!(c, Constraint::PointOnLine { p, .. } if *p == m))
        );

        // Around an existing point: same figure. On empty board: a new figure with a named center
        let _ = engine.add_circle_with_radius(Vector2::new(30.0, 0.0), Some((key, m)), None, 50.0);
        assert_eq!(figure_of(&engine.store, key).unwrap().circles.len(), 1);
        let _ =
            engine.add_circle_with_radius(Vector2::new(500.0, 500.0), None, Some("O".into()), 40.0);
        assert_eq!(engine.store.stroke_keys_unordered().len(), 2);
        assert_eq!(engine.suggest_point_label(), "A");
    }

    #[test]
    fn press_on_point_draws_from_it_and_selected_point_drags() {
        let mut store = crate::store::StrokeStore::default();
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(100.0, 0.0), Some("B".into()));
        fig.add_segment(a, b);
        let key = store.insert_stroke(
            Stroke::ShapeStroke(ShapeStroke::new(
                Shape::MathFigure(fig),
                rnote_compose::Style::default(),
            )),
            None,
        );

        // Near A (even a bit off): start a new line exactly at A, a mere click selects A
        match press_action(Vector2::new(10.0, 6.0), &store, None, 16.0, 24.0) {
            PressAction::Build {
                from: Some(from),
                pick: Some(pick),
            } => {
                assert_eq!(from, Vector2::new(0.0, 0.0));
                assert_eq!((pick.key, pick.point), (key, Some(a)));
            }
            _ => panic!("expected drawing from A"),
        }
        // On the body: select
        assert!(matches!(
            press_action(Vector2::new(50.0, 3.0), &store, None, 16.0, 24.0),
            PressAction::Select(_)
        ));

        let selected_a = SelectedShapeData {
            key,
            point: Some(a),
            segment: None,
            click: Vector2::new(0.0, 0.0),
        };
        // The selected point drags
        assert!(matches!(
            press_action(Vector2::new(2.0, 0.0), &store, Some(&selected_a), 16.0, 24.0),
            PressAction::Drag(HandleKind::FigurePoint(id), _) if id == a
        ));
        // Another point of the selected figure still draws, it is not swallowed by the body
        assert!(matches!(
            press_action(Vector2::new(96.0, 5.0), &store, Some(&selected_a), 16.0, 24.0),
            PressAction::Build { from: Some(p), .. } if p == Vector2::new(100.0, 0.0)
        ));
        // Selected body: translate
        assert!(matches!(
            press_action(
                Vector2::new(50.0, 3.0),
                &store,
                Some(&selected_a),
                16.0,
                24.0
            ),
            PressAction::Drag(HandleKind::LineMid, _)
        ));

        // A line drawn from A merges into A and takes the next free name
        let mut line = MathFigure::new();
        line.build_line(
            Vector2::new(0.0, 0.0),
            Vector2::new(0.0, 80.0),
            Some("C"),
            Some("D"),
        );
        prepare_new_figure(&mut line, &store, LineStyle::Dashed, true);
        let mut pts: Vec<_> = line.points.values().collect();
        pts.sort_by_key(|p| p.id);
        assert!(!pts[0].visible && pts[0].label.is_none());
        assert_eq!(pts[1].label.as_deref(), Some("C"));
        assert!(line.lines.values().all(|l| l.style == LineStyle::Dashed));

        // "No points" mode hides every point
        let mut bare = MathFigure::new();
        bare.build_line(
            Vector2::new(300.0, 0.0),
            Vector2::new(400.0, 0.0),
            Some("E"),
            Some("F"),
        );
        prepare_new_figure(&mut bare, &store, LineStyle::Solid, false);
        assert!(
            bare.points
                .values()
                .all(|p| !p.visible && p.label.is_none())
        );
    }
}
