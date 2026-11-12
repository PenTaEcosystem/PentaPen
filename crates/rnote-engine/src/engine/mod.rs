// Modules
pub mod animation;
pub mod config;
pub mod export;
pub mod import;
pub mod rendering;
pub mod snapshot;
pub mod strokecontent;
pub mod visual_debug;

// Re-exports
pub use animation::Animation;
pub use config::EngineConfig;
pub use config::EngineConfigShared;
pub use export::ExportPrefs;
pub use import::ImportPrefs;
pub use snapshot::EngineSnapshot;
pub use strokecontent::{LayeredStroke, StrokeContent};

// Imports
use crate::Image;
use crate::document::Layout;
use crate::pens::PenMode;
use crate::pens::{Pen, PenStyle};
use crate::store::StrokeKey;
use crate::store::render_comp::{self, RenderCompState};
use crate::strokes::content::GeneratedContentImages;
use crate::strokes::textstroke::{TextAttribute, TextStyle};
use crate::{Camera, Document, PenHolder, StrokeStore};
use crate::{SelectionCollision, WidgetFlags};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::{mpsc, oneshot};
use p2d::bounding_volume::{Aabb, BoundingVolume};
use p2d::math::Vector2;
use rnote_compose::eventresult::EventPropagation;
use rnote_compose::ext::AabbExt;
use rnote_compose::penevent::{PenEvent, ShortcutKey};
use rnote_compose::{Color, SplitOrder};
use serde::{Deserialize, Serialize};
use snapshot::Snapshotable;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tracing::error;

/// An immutable view into the engine, excluding the penholder.
#[derive(Debug)]
pub struct EngineView<'a> {
    pub tasks_tx: EngineTaskSender,
    pub config: &'a EngineConfig,
    pub document: &'a Document,
    pub store: &'a StrokeStore,
    pub camera: &'a Camera,
    #[cfg(feature = "ui")]
    pub audioplayer: &'a Option<crate::AudioPlayer>,
    pub animation: &'a Animation,
}

/// Constructs an `EngineView` from an identifier containing an `Engine` instance.
#[macro_export]
macro_rules! engine_view {
    ($engine:ident) => {
        $crate::engine::EngineView {
            tasks_tx: $engine.tasks_tx.clone(),
            config: &$engine.config.read(),
            document: &$engine.document,
            store: &$engine.store,
            camera: &$engine.camera,
            #[cfg(feature = "ui")]
            audioplayer: &$engine.audioplayer,
            animation: &$engine.animation,
        }
    };
}

/// A mutable view into the engine, excluding the penholder.
#[derive(Debug)]
pub struct EngineViewMut<'a> {
    pub tasks_tx: EngineTaskSender,
    pub config: &'a mut EngineConfig,
    pub document: &'a mut Document,
    pub store: &'a mut StrokeStore,
    pub camera: &'a mut Camera,
    #[cfg(feature = "ui")]
    pub audioplayer: &'a mut Option<crate::AudioPlayer>,
    pub animation: &'a mut Animation,
}

/// Constructs an `EngineViewMut` from an identifier containing an `Engine` instance.
#[macro_export]
macro_rules! engine_view_mut {
    ($engine:ident) => {
        $crate::engine::EngineViewMut {
            tasks_tx: $engine.tasks_tx.clone(),
            config: &mut $engine.config.write(),
            document: &mut $engine.document,
            store: &mut $engine.store,
            camera: &mut $engine.camera,
            #[cfg(feature = "ui")]
            audioplayer: &mut $engine.audioplayer,
            animation: &mut $engine.animation,
        }
    };
}

impl EngineViewMut<'_> {
    // Converts itself to the immutable view.
    pub(crate) fn as_im<'m>(&'m self) -> EngineView<'m> {
        EngineView::<'m> {
            tasks_tx: self.tasks_tx.clone(),
            config: self.config,
            document: self.document,
            store: self.store,
            camera: self.camera,
            #[cfg(feature = "ui")]
            audioplayer: self.audioplayer,
            animation: self.animation,
        }
    }
}

#[derive(Debug, Clone)]
/// An engine task, usually coming from a spawned thread and to be processed with [Engine::handle_engine_task].
pub enum EngineTask {
    /// Replace the images for rendering of the given stroke.
    ///
    /// The state of the render component should be set **before** spawning a thread, generating images and sending this task,
    /// to avoid spawning large amounts of already outdated rendering tasks when checking the render component's state on resize/zooming, etc. .
    UpdateStrokeWithImages {
        /// The stroke key.
        key: StrokeKey,
        /// The generated images.
        images: GeneratedContentImages,
        /// The image scale-factor the render task was using while generating the images.
        image_scale: f64,
    },
    /// Appends the images to the rendering of the given stroke.
    ///
    /// The state of the render component should be set **before** spawning a thread, generating images and sending this task,
    /// to avoid spawning large amounts of already outdated rendering tasks when checking the render component's state on resize/zooming, etc. .
    AppendImagesToStroke {
        /// The stroke key
        key: StrokeKey,
        /// The generated images
        images: GeneratedContentImages,
    },
    /// Requests that the typewriter cursor should be blinked/toggled
    BlinkTypewriterCursor,
    /// Change the permanent zoom to the given value
    Zoom(f64),
    /// Indicates that the application is quitting. Sent to quit the handler which receives the tasks.
    Quit,
}

#[derive(Debug, Clone)]
pub struct EngineTaskSender(mpsc::UnboundedSender<EngineTask>);

impl EngineTaskSender {
    pub fn send(&self, task: EngineTask) {
        if let Err(e) = self.0.unbounded_send(task) {
            let err = format!("{e:?}");
            error!(
                "Failed to send engine task {:?}, Err: {err}",
                e.into_inner()
            );
        }
    }
}

#[derive(Debug)]
pub struct EngineTaskReceiver(mpsc::UnboundedReceiver<EngineTask>);

impl EngineTaskReceiver {
    pub fn recv(&mut self) -> futures::stream::Next<'_, UnboundedReceiver<EngineTask>> {
        self.0.next()
    }
}

/// The engine.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default, rename = "engine")]
pub struct Engine {
    #[serde(rename = "config")]
    pub(crate) config: EngineConfigShared,
    #[serde(rename = "document")]
    pub document: Document,
    #[serde(rename = "store")]
    pub store: StrokeStore,
    #[serde(rename = "camera")]
    pub camera: Camera,
    #[serde(rename = "penholder")]
    pub penholder: PenHolder,

    #[cfg(feature = "ui")]
    #[serde(skip)]
    audioplayer: Option<crate::AudioPlayer>,
    #[serde(skip)]
    pub animation: Animation,
    // the task sender. Must not be modified, only cloned.
    #[serde(skip)]
    tasks_tx: EngineTaskSender,
    #[serde(skip)]
    tasks_rx: Option<EngineTaskReceiver>,
    // Background rendering
    #[serde(skip)]
    background_tile_image: Option<Image>,
    #[cfg(feature = "ui")]
    #[serde(skip)]
    background_rendernodes: Vec<gtk4::gsk::RenderNode>,
    // Origin indicator rendering
    #[serde(skip)]
    origin_indicator_image: Option<Image>,
    #[cfg(feature = "ui")]
    #[serde(skip)]
    origin_indicator_rendernode: Option<gtk4::gsk::RenderNode>,
    /// Last center point dotted or clicked on canvas, for AI geometry construction
    #[serde(skip)]
    pub last_center_point: Option<(p2d::math::Vector2, Option<String>)>,
}

impl Default for Engine {
    fn default() -> Self {
        let (tasks_tx, tasks_rx) = futures::channel::mpsc::unbounded::<EngineTask>();

        Self {
            config: EngineConfigShared(Arc::new(RwLock::new(EngineConfig::default()))),
            document: Document::default(),
            store: StrokeStore::default(),
            camera: Camera::default(),
            penholder: PenHolder::default(),

            #[cfg(feature = "ui")]
            audioplayer: None,
            animation: Animation::default(),
            tasks_tx: EngineTaskSender(tasks_tx),
            tasks_rx: Some(EngineTaskReceiver(tasks_rx)),
            background_tile_image: None,
            #[cfg(feature = "ui")]
            background_rendernodes: Vec::default(),
            origin_indicator_image: None,
            #[cfg(feature = "ui")]
            origin_indicator_rendernode: None,
            last_center_point: None,
        }
    }
}

impl Engine {
    pub(crate) const STROKE_BOUNDS_INTERSECTION_TOLERANCE: f64 = 1e-3;

    /// Get last center point dotted or clicked on canvas
    pub fn last_center_point(&self) -> Option<(p2d::math::Vector2, Option<String>)> {
        self.last_center_point.clone()
    }

    /// Set last center point dotted or clicked on canvas
    pub fn set_last_center_point(&mut self, pos: p2d::math::Vector2, label: Option<String>) {
        self.last_center_point = Some((pos, label));
    }

    pub fn install_config(
        &mut self,
        config: &EngineConfigShared,
        data_dir: Option<PathBuf>,
    ) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();

        let pen_sounds = config.read().pen_sounds;

        self.config = config.clone();
        self.set_pen_sounds(pen_sounds, data_dir);

        widget_flags |= self
            .penholder
            .reinstall_pen_current_style(&mut engine_view_mut!(self));
        widget_flags |= self.doc_resize_to_fit_content();
        widget_flags.redraw = true;
        widget_flags.refresh_ui = true;
        widget_flags
    }

    pub fn engine_tasks_tx(&self) -> EngineTaskSender {
        self.tasks_tx.clone()
    }

    pub fn take_engine_tasks_rx(&mut self) -> Option<EngineTaskReceiver> {
        self.tasks_rx.take()
    }

    /// Whether pen sounds are enabled.
    pub fn pen_sounds(&self) -> bool {
        self.config.read().pen_sounds
    }

    /// Enables/disables the pen sounds.
    ///
    /// If pen sound should be enabled, the pkg data dir must be provided.
    pub fn set_pen_sounds(&mut self, pen_sounds: bool, pkg_data_dir: Option<PathBuf>) {
        self.config.write().pen_sounds = pen_sounds;

        #[cfg(feature = "ui")]
        {
            if pen_sounds {
                if let Some(pkg_data_dir) = pkg_data_dir {
                    // Only create and init a new audioplayer if it does not already exist
                    if self.audioplayer.is_none() {
                        self.audioplayer = match crate::AudioPlayer::new_init(pkg_data_dir) {
                            Ok(audioplayer) => Some(audioplayer),
                            Err(e) => {
                                error!(
                                    "Creating a new audioplayer failed while enabling pen sounds, Err: {e:?}"
                                );
                                None
                            }
                        }
                    }
                }
            } else {
                self.audioplayer.take();
            }
        }
        #[cfg(not(feature = "ui"))]
        {
            let _ = pkg_data_dir;
        }
    }

    pub fn optimize_epd(&self) -> bool {
        self.config.read().optimize_epd
    }

    /// Takes a snapshot of the current state.
    pub fn take_snapshot(&self) -> EngineSnapshot {
        let mut store_history_entry = self.store.create_history_entry();

        // Remove all trashed strokes
        let trashed_keys = store_history_entry
            .trash_components
            .iter()
            .filter_map(|(key, trash_comp)| if trash_comp.trashed { Some(key) } else { None })
            .collect::<Vec<StrokeKey>>();

        for key in trashed_keys {
            Arc::make_mut(&mut store_history_entry.stroke_components).remove(key);
        }

        EngineSnapshot {
            document: self.document.extract_snapshot_data(),
            camera: self.camera.extract_snapshot_data(),
            stroke_components: Arc::clone(&store_history_entry.stroke_components),
            chrono_components: Arc::clone(&store_history_entry.chrono_components),
            chrono_counter: store_history_entry.chrono_counter,
        }
    }

    /// Imports an engine snapshot. A save file should always be loaded with this method.
    pub fn load_snapshot(&mut self, snapshot: EngineSnapshot) -> WidgetFlags {
        self.document = snapshot.document.extract_snapshot_data();
        self.camera = snapshot.camera.extract_snapshot_data();
        let mut widget_flags = self.store.import_from_snapshot(&snapshot)
            | self.doc_resize_autoexpand()
            | self.current_pen_update_state()
            | self.background_rendering_regenerate()
            | self.update_content_rendering_current_viewport();
        widget_flags.refresh_ui = true;
        widget_flags.view_modified = true;
        widget_flags
    }

    /// Records the current store state and saves it as a history entry.
    pub fn record(&mut self, now: Instant) -> WidgetFlags {
        self.store.record(now)
    }

    /// Update the state of the latest history entry with the current document state.
    pub fn update_latest_history_entry(&mut self, now: Instant) -> WidgetFlags {
        self.store.update_latest_history_entry(now)
    }

    /// Undo the latest changes.
    pub fn undo(&mut self, now: Instant) -> WidgetFlags {
        self.store.undo(now)
            | self.doc_resize_autoexpand()
            | self.current_pen_update_state()
            | self.update_rendering_current_viewport()
    }

    /// Redo the latest changes.
    pub fn redo(&mut self, now: Instant) -> WidgetFlags {
        self.store.redo(now)
            | self.doc_resize_autoexpand()
            | self.current_pen_update_state()
            | self.update_rendering_current_viewport()
    }

    pub fn can_undo(&self) -> bool {
        self.store.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.store.can_redo()
    }

    // Clears the entire engine.
    pub fn clear(&mut self) -> WidgetFlags {
        self.store.clear() | self.current_pen_update_state() | self.return_to_origin(None)
    }

    /// Handle a received task from tasks_rx.
    /// Returns [WidgetFlags] to indicate what needs to be updated in the UI.
    ///
    /// An example of how to use it:
    /// ```rust, ignore
    ///
    /// glib::spawn_future_local(clone!(@weak canvas, @weak appwindow => async move {
    ///    let mut task_rx = canvas.engine_mut().regenerate_channel();
    ///
    ///    loop {
    ///        if let Some(task) = task_rx.next().await {
    ///            let (widget_flags, quit) = canvas.engine_mut().handle_engine_task(task);
    ///            canvas.emit_handle_widget_flags(widget_flags);
    ///
    ///            if quit {
    ///                break;
    ///            }
    ///        }
    ///    }
    /// }));
    /// ```
    pub fn handle_engine_task(&mut self, task: EngineTask) -> (WidgetFlags, bool) {
        let mut widget_flags = WidgetFlags::default();
        let mut quit = false;

        match task {
            EngineTask::UpdateStrokeWithImages {
                key,
                images,
                image_scale,
            } => {
                if let Some(state) = self.store.render_comp_state(key) {
                    match state {
                        RenderCompState::Complete | RenderCompState::ForViewport(_) => {
                            // The rendering was already regenerated in the meantime,
                            // so we just discard the render task result
                        }
                        RenderCompState::BusyRenderingInTask => {
                            if (self.camera.image_scale()
                                - render_comp::RENDER_IMAGE_SCALE_TOLERANCE
                                ..self.camera.image_scale()
                                    + render_comp::RENDER_IMAGE_SCALE_TOLERANCE)
                                .contains(&image_scale)
                            {
                                // Only when the image scale is roughly the same as when the render task was started,
                                // the new images are considered valid and can replace the old.
                                self.store.replace_rendering_with_images(key, images);
                            }
                            widget_flags.redraw = true;
                        }
                        RenderCompState::Dirty => {
                            // If the state was flagged dirty in the meantime,
                            // it is expected that retriggering rendering will be handled elsewhere
                        }
                    }
                }
            }
            EngineTask::AppendImagesToStroke { key, images } => {
                self.store.append_rendering_images(key, images);
                widget_flags.redraw = true;
            }
            EngineTask::BlinkTypewriterCursor => {
                if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
                    typewriter.toggle_cursor_visibility();
                    widget_flags.redraw = true;
                }
            }
            EngineTask::Zoom(zoom) => {
                widget_flags |= self.camera.zoom_temporarily_to(1.0) | self.camera.zoom_to(zoom);

                let all_strokes = self.store.stroke_keys_unordered();
                self.store.set_rendering_dirty_for_strokes(&all_strokes);
                widget_flags |= self.doc_resize_autoexpand()
                    | self.background_rendering_regenerate()
                    | self.update_rendering_current_viewport();
            }
            EngineTask::Quit => {
                widget_flags |= self.set_active(false);
                quit = true;
            }
        }

        (widget_flags, quit)
    }

    /// Handle a pen event.
    pub fn handle_pen_event(
        &mut self,
        event: PenEvent,
        pen_mode: Option<PenMode>,
        now: Instant,
    ) -> (EventPropagation, WidgetFlags) {
        self.penholder
            .handle_pen_event(event, pen_mode, now, &mut engine_view_mut!(self))
    }

    /// Handle a pressed shortcut key.
    pub fn handle_pressed_shortcut_key(
        &mut self,
        shortcut_key: ShortcutKey,
        now: Instant,
    ) -> (EventPropagation, WidgetFlags) {
        self.penholder
            .handle_pressed_shortcut_key(shortcut_key, now, &mut engine_view_mut!(self))
    }

    /// Change the pen style.
    pub fn change_pen_style(&mut self, new_style: PenStyle) -> WidgetFlags {
        self.penholder
            .change_style(new_style, &mut engine_view_mut!(self))
    }

    /// Change the pen style (temporary) override.
    pub fn change_pen_style_override(
        &mut self,
        new_style_override: Option<PenStyle>,
    ) -> WidgetFlags {
        self.penholder
            .change_style_override(new_style_override, &mut engine_view_mut!(self))
    }

    /// Change the pen mode. Relevant for stylus input.
    pub fn change_pen_mode(&mut self, pen_mode: PenMode) -> WidgetFlags {
        self.penholder
            .change_pen_mode(pen_mode, &mut engine_view_mut!(self))
    }

    /// Reinstall the pen in the current style.
    pub fn reinstall_pen_current_style(&mut self) -> WidgetFlags {
        self.penholder
            .reinstall_pen_current_style(&mut engine_view_mut!(self))
    }

    /// Set the engine active or inactive.
    pub fn set_active(&mut self, active: bool) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if active {
            widget_flags |= self.reinstall_pen_current_style()
                | self.background_rendering_regenerate()
                | self.update_content_rendering_current_viewport();
            widget_flags.view_modified = true;
        } else {
            widget_flags |= self.clear_rendering() | self.penholder.deinit_current_pen();
        }
        widget_flags
    }

    /// Generate bounds for each page on the document which contains content.
    pub fn pages_bounds_w_content(&self, split_order: SplitOrder) -> Vec<Aabb> {
        let doc_bounds = self.document.bounds();
        let keys = self.store.stroke_keys_as_rendered();

        let strokes_bounds = self.store.strokes_bounds(&keys);

        let pages_bounds = doc_bounds
            .split_extended_origin_aligned(self.document.config.format.size(), split_order)
            .into_iter()
            .filter(|page_bounds| {
                // Filter the pages out that don't intersect with any stroke
                strokes_bounds.iter().any(|stroke_bounds| {
                    stroke_bounds.intersects_w_tolerance(
                        page_bounds,
                        Self::STROKE_BOUNDS_INTERSECTION_TOLERANCE,
                    )
                })
            })
            .collect::<Vec<Aabb>>();

        if pages_bounds.is_empty() {
            // If no page has content, return the origin page
            vec![Aabb::new(Vector2::ZERO, self.document.config.format.size())]
        } else {
            pages_bounds
        }
    }

    /// Generates bounds which contain all pages on the doc with content, extended to fit the current format.
    pub fn bounds_w_content_extended(&self) -> Option<Aabb> {
        let pages_bounds = self.pages_bounds_w_content(SplitOrder::default());

        if pages_bounds.is_empty() {
            return None;
        }

        Some(
            pages_bounds
                .into_iter()
                .fold(Aabb::new_invalid(), |prev, next| prev.merged(&next)),
        )
    }

    /// First zoom temporarily and then permanently after a timeout.
    ///
    /// Repeated calls to this function reset the timeout.
    pub fn zoom_w_timeout(&mut self, zoom: f64) -> WidgetFlags {
        self.camera.zoom_w_timeout(zoom, self.tasks_tx.clone())
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) -> WidgetFlags {
        self.store
            .set_rendering_dirty_for_strokes(&self.store.stroke_keys_as_rendered());
        self.camera.set_scale_factor(scale_factor)
            | self.background_rendering_regenerate()
            | self.update_content_rendering_current_viewport()
    }

    /// Resizes the doc to the format and to fit all strokes.
    ///
    /// Background rendering then needs to be updated.
    pub fn doc_resize_to_fit_content(&mut self) -> WidgetFlags {
        let widget_flags = self
            .document
            .resize_to_fit_content(&self.store, &self.camera);
        widget_flags | self.update_rendering_current_viewport()
    }

    pub fn return_to_origin(&mut self, parent_width: Option<f64>) -> WidgetFlags {
        let zoom = self.camera.zoom();
        let new_offset = if let Some(parent_width) = parent_width {
            if self.document.config.format.width() * zoom <= parent_width {
                Vector2::new(
                    (self.document.config.format.width() * 0.5 * zoom) - parent_width * 0.5,
                    -Document::SHADOW_WIDTH * zoom,
                )
            } else {
                // If the zoomed format width is larger than the displayed surface, we zoom to a fixed origin
                Vector2::new(
                    -Document::SHADOW_WIDTH * zoom,
                    -Document::SHADOW_WIDTH * zoom,
                )
            }
        } else {
            Vector2::new(
                -Document::SHADOW_WIDTH * zoom,
                -Document::SHADOW_WIDTH * zoom,
            )
        };
        self.camera_set_offset_expand(new_offset)
    }

    /// Resize the doc when in autoexpanding layouts. called e.g. when finishing a new stroke.
    ///
    /// Background rendering then needs to be updated.
    pub fn doc_resize_autoexpand(&mut self) -> WidgetFlags {
        let widget_flags = self.document.resize_autoexpand(&self.store, &self.camera);
        widget_flags | self.update_rendering_current_viewport()
    }

    /// Expand the doc to the camera when in autoexpanding layouts. called e.g. when dragging with touch.
    ///
    /// Background and content rendering then need to be updated.
    pub fn doc_expand_autoexpand(&mut self) -> WidgetFlags {
        self.document.expand_autoexpand(&self.camera, &self.store)
    }

    /// Add a page to the document when in fixed size layout.
    ///
    /// Document layout must be set to fixed-size.
    pub fn doc_add_page_fixed_size(&mut self) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if self.document.add_page_fixed_size() {
            widget_flags |= self.update_rendering_current_viewport();
            widget_flags.resize = true;
        }
        widget_flags
    }

    /// Remove a page from the document when in fixed size layout.
    ///
    /// Document layout must be set to fixed-size.
    pub fn doc_remove_page_fixed_size(&mut self) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if self.document.remove_page_fixed_size() {
            self.store.set_trashed_keys(
                &self
                    .store
                    .keys_below_y(self.document.y + self.document.height),
                true,
            );
            widget_flags |= self.record(Instant::now()) | self.update_rendering_current_viewport();
            widget_flags.resize = true;
        }
        widget_flags
    }

    /// Update the viewport offset of the camera, clamped to mins and maxs values depending on the document layout.
    ///
    /// Background and content rendering then need to be updated.
    pub fn camera_set_offset(&mut self, offset: Vector2) -> WidgetFlags {
        self.camera.set_offset(offset, &self.document)
    }

    /// Update the viewport offset of the camera, clamped to mins and maxs values depending on the document layout.
    ///
    /// Expands the document when in autoexpanding layouts.
    ///
    /// Background and content rendering then need to be updated.
    pub fn camera_set_offset_expand(&mut self, offset: Vector2) -> WidgetFlags {
        let widget_flags = self.camera.set_offset(offset, &self.document);
        widget_flags | self.doc_expand_autoexpand()
    }

    /// Update the viewport size of the camera.
    ///
    /// Background and content rendering then need to be updated.
    pub fn camera_set_size(&mut self, size: Vector2) -> WidgetFlags {
        self.camera.set_size(size)
    }

    /// The minimum and maximum surface bounds (document including overshoot) in surface coordinate space.
    pub fn camera_surface_mins_maxs(&self) -> (Vector2, Vector2) {
        self.camera.surface_mins_maxs(&self.document)
    }

    /// Update the current pen with the current engine state.
    ///
    /// Needs to be called when the engine state was changed outside of pen events.
    /// ( e.g. trash all strokes, set strokes selected, etc. )
    pub fn current_pen_update_state(&mut self) -> WidgetFlags {
        self.penholder
            .current_pen_update_state(&mut engine_view_mut!(self))
    }

    /// Fetch clipboard content from the current pen.
    #[allow(clippy::type_complexity)]
    pub fn fetch_clipboard_content(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<(Vec<(Vec<u8>, String)>, WidgetFlags)>> {
        self.penholder.fetch_clipboard_content(&engine_view!(self))
    }

    /// Cut clipboard content from the current pen.
    #[allow(clippy::type_complexity)]
    pub fn cut_clipboard_content(
        &mut self,
    ) -> oneshot::Receiver<anyhow::Result<(Vec<(Vec<u8>, String)>, WidgetFlags)>> {
        self.penholder
            .cut_clipboard_content(&mut engine_view_mut!(self))
    }

    pub fn set_doc_layout(&mut self, layout: Layout) -> WidgetFlags {
        if self.document.config.layout != layout {
            self.document.config.layout = layout;
            self.doc_resize_to_fit_content()
        } else {
            self.doc_resize_autoexpand()
        }
    }

    pub fn select_all_strokes(&mut self) -> WidgetFlags {
        let widget_flags = self.change_pen_style(PenStyle::Selector);
        self.store
            .set_selected_keys(&self.store.stroke_keys_as_rendered(), true);
        widget_flags
            | self.current_pen_update_state()
            | self.doc_resize_autoexpand()
            | self.record(Instant::now())
            | self.update_rendering_current_viewport()
    }

    pub fn deselect_all_strokes(&mut self) -> WidgetFlags {
        let widget_flags = self.change_pen_style(PenStyle::Selector);
        self.store
            .set_selected_keys(&self.store.selection_keys_as_rendered(), false);
        widget_flags
            | self.current_pen_update_state()
            | self.doc_resize_autoexpand()
            | self.record(Instant::now())
            | self.update_rendering_current_viewport()
    }

    pub fn select_with_bounds(
        &mut self,
        bounds: Aabb,
        collision: SelectionCollision,
    ) -> WidgetFlags {
        let select = match collision {
            SelectionCollision::Contains => self.store.stroke_keys_as_rendered_in_bounds(bounds),
            SelectionCollision::Intersects => self
                .store
                .stroke_keys_as_rendered_intersecting_bounds(bounds),
        };
        self.store.set_selected_keys(&select, true);
        self.doc_resize_autoexpand()
            | self.record(Instant::now())
            | self.update_rendering_current_viewport()
    }

    pub fn duplicate_selection(&mut self) -> WidgetFlags {
        let new_selected = self.store.duplicate_selection();
        self.store.update_geometry_for_strokes(&new_selected);
        self.current_pen_update_state()
            | self.doc_resize_autoexpand()
            | self.record(Instant::now())
            | self.update_rendering_current_viewport()
    }

    pub fn trash_selection(&mut self) -> WidgetFlags {
        let selection_keys = self.store.selection_keys_as_rendered();
        self.store.set_trashed_keys(&selection_keys, true);
        self.current_pen_update_state()
            | self.doc_resize_autoexpand()
            | self.record(Instant::now())
            | self.update_rendering_current_viewport()
    }

    pub fn nothing_selected(&self) -> bool {
        self.store.selection_keys_unordered().is_empty()
    }

    pub fn change_selection_stroke_colors(&mut self, stroke_color: Color) -> WidgetFlags {
        self.store
            .change_stroke_colors(&self.store.selection_keys_as_rendered(), stroke_color)
            | self.record(Instant::now())
            | self.update_content_rendering_current_viewport()
    }

    pub fn change_selection_fill_colors(&mut self, fill_color: Color) -> WidgetFlags {
        self.store
            .change_fill_colors(&self.store.selection_keys_as_rendered(), fill_color)
            | self.record(Instant::now())
            | self.update_content_rendering_current_viewport()
    }

    pub fn invert_selection_colors(&mut self) -> WidgetFlags {
        self.store
            .invert_color_brightness(&self.store.selection_keys_as_rendered())
            | self.record(Instant::now())
            | self.update_content_rendering_current_viewport()
    }

    pub fn text_selection_change_style<F>(&mut self, modify_func: F) -> WidgetFlags
    where
        F: FnOnce(&mut TextStyle),
    {
        let mut widget_flags = WidgetFlags::default();
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            widget_flags |= typewriter
                .change_text_style_in_modifying_stroke(modify_func, &mut engine_view_mut!(self))
        }
        widget_flags
    }

    pub fn text_selection_remove_attributes(&mut self) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            widget_flags |=
                typewriter.remove_text_attributes_current_selection(&mut engine_view_mut!(self))
        }
        widget_flags
    }

    pub fn text_select_closest_word(&mut self) {
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            typewriter.select_closest_word(&mut engine_view_mut!(self))
        }
    }

    pub fn text_select_closest_line(&mut self) {
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            typewriter.select_closest_line(&mut engine_view_mut!(self))
        }
    }

    pub fn text_selection_toggle_attribute(
        &mut self,
        text_attribute: TextAttribute,
    ) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            widget_flags |= typewriter.toggle_text_attribute_current_selection(
                text_attribute,
                &mut engine_view_mut!(self),
            )
        }
        widget_flags
    }

    pub fn text_selection_add_attribute(&mut self, text_attribute: TextAttribute) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            widget_flags |= typewriter
                .add_text_attribute_current_selection(text_attribute, &mut engine_view_mut!(self))
        }
        widget_flags
    }

    pub fn text_change_color(&mut self, color: Color) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        if let Pen::Typewriter(typewriter) = self.penholder.current_pen_mut() {
            if typewriter.selection_range().is_some() {
                widget_flags |= typewriter.replace_text_attribute_current_selection(
                    TextAttribute::TextColor(color),
                    &mut engine_view_mut!(self),
                )
            } else {
                widget_flags |= typewriter.change_text_style_in_modifying_stroke(
                    |style| style.color = color,
                    &mut engine_view_mut!(self),
                )
            }
        }
        widget_flags
    }

    /// Handle a requested animation frame.
    ///
    /// Can request another frame using `EngineViewMut#animation.claim_frame()`.
    pub fn handle_animation_frame(&mut self) {
        self.penholder
            .handle_animation_frame(&mut engine_view_mut!(self));
    }

    pub fn current_pen_style_w_override(&self) -> PenStyle {
        self.penholder
            .current_pen_style_w_override(&engine_view!(self))
    }

    /// Insert a shape stroke into the store and regenerate its rendering.
    pub fn insert_shape(&mut self, shape: rnote_compose::Shape) -> WidgetFlags {
        let style = self
            .config
            .read()
            .pens_config
            .shaper_config
            .gen_style_for_current_options();
        let key = self.store.insert_stroke(
            crate::strokes::Stroke::ShapeStroke(crate::strokes::ShapeStroke::new(shape, style)),
            None,
        );
        self.store.update_geometry_for_strokes(&[key]);
        self.store.regenerate_rendering_for_stroke(
            key,
            self.camera.viewport(),
            self.camera.image_scale(),
        );
        let mut widget_flags = self.record(std::time::Instant::now());
        widget_flags.store_modified = true;
        widget_flags.redraw = true;
        widget_flags
    }

    /// Insert strokes as one undo step, in the shaper's current color and width.
    fn insert_generated_strokes(
        &mut self,
        make: impl FnOnce(&rnote_compose::Style, rnote_compose::Color) -> Vec<crate::strokes::Stroke>,
    ) -> WidgetFlags {
        let style = self
            .config
            .read()
            .pens_config
            .shaper_config
            .gen_style_for_current_options();
        let color = style.stroke_color().unwrap_or(rnote_compose::Color::BLACK);
        let keys: Vec<StrokeKey> = make(&style, color)
            .into_iter()
            .map(|stroke| self.store.insert_stroke(stroke, None))
            .collect();
        self.store.update_geometry_for_strokes(&keys);
        for key in keys {
            self.store.regenerate_rendering_for_stroke(
                key,
                self.camera.viewport(),
                self.camera.image_scale(),
            );
        }
        let mut widget_flags = self.document.resize_autoexpand(&self.store, &self.camera)
            | self.record(std::time::Instant::now());
        widget_flags.store_modified = true;
        widget_flags.redraw = true;
        widget_flags
    }

    /// Draw y = f(x) with axes. `center` is where the middle of the window lands on the canvas.
    pub fn insert_function_graph(
        &mut self,
        f: &crate::graph::Function,
        center: Vector2,
        unit: f64,
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) -> WidgetFlags {
        let mid = Vector2::new(0.5 * (x_range.0 + x_range.1), 0.5 * (y_range.0 + y_range.1));
        let frame = crate::graph::GraphFrame {
            origin: center - Vector2::new(mid.x * unit, -mid.y * unit),
            unit,
            x_range,
            y_range,
        };
        self.insert_generated_strokes(|style, color| {
            crate::graph::graph_strokes(f, frame, style, color)
        })
    }

    /// Draw Oxy axes for `frame` and the curves of `functions` on them, as one undo step.
    pub fn insert_graph_frame(
        &mut self,
        frame: crate::graph::GraphFrame,
        functions: &[crate::graph::Function],
    ) -> WidgetFlags {
        self.insert_generated_strokes(|style, color| {
            let mut out = crate::graph::axes_strokes(frame, style, color);
            for f in functions {
                out.extend(crate::graph::curve_strokes(f, frame, style));
            }
            out
        })
    }

    /// Draw the variation table of f with its upper left corner at `top_left`.
    pub fn insert_variation_table(
        &mut self,
        f: &crate::graph::Function,
        top_left: Vector2,
    ) -> Result<WidgetFlags, String> {
        let table = crate::graph::VariationTable::study(f)?;
        Ok(self.insert_generated_strokes(|style, color| {
            crate::graph::table_strokes(&table, top_left, style, color)
        }))
    }

    fn math_figures(&self) -> impl Iterator<Item = (StrokeKey, &rnote_compose::MathFigure)> {
        self.store
            .stroke_keys_unordered()
            .into_iter()
            .filter_map(|key| match self.store.get_stroke_ref(key) {
                Some(crate::strokes::Stroke::ShapeStroke(ss)) => match &ss.shape {
                    rnote_compose::Shape::MathFigure(fig) => Some((key, fig)),
                    _ => None,
                },
                _ => None,
            })
    }

    /// The geometry point nearest to `pos` (document coordinates) within `tolerance`:
    /// its stroke, id, label and position.
    pub fn figure_point_at(
        &self,
        pos: Vector2,
        tolerance: f64,
    ) -> Option<(
        StrokeKey,
        rnote_compose::geometry::PointId,
        Option<String>,
        Vector2,
    )> {
        self.math_figures()
            .flat_map(|(key, fig)| {
                fig.points
                    .values()
                    .map(move |p| (key, p.id, p.label.clone(), p.pos))
            })
            .map(|hit| ((hit.3 - pos).length(), hit))
            .filter(|(d, _)| *d <= tolerance)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, hit)| hit)
    }

    /// Rename and move one geometry point; dependent elements follow through the constraint solver.
    pub fn update_figure_point(
        &mut self,
        key: StrokeKey,
        id: rnote_compose::geometry::PointId,
        label: Option<String>,
        pos: Vector2,
    ) -> WidgetFlags {
        let Some(crate::strokes::Stroke::ShapeStroke(ss)) = self.store.get_stroke_mut(key) else {
            return WidgetFlags::default();
        };
        let rnote_compose::Shape::MathFigure(fig) = &mut ss.shape else {
            return WidgetFlags::default();
        };
        fig.drag_point(id, pos);
        if let Some(p) = fig.points.get_mut(&id) {
            p.label = label;
        }
        self.store.update_geometry_for_strokes(&[key]);
        self.store.regenerate_rendering_for_stroke(
            key,
            self.camera.viewport(),
            self.camera.image_scale(),
        );
        let mut widget_flags = self.record(Instant::now());
        widget_flags.store_modified = true;
        widget_flags.redraw = true;
        widget_flags
    }

    /// Whether a geometry line joins the points named `a` and `b` (any figure on the page);
    /// with `style`, those lines also get that style.
    pub fn restyle_figure_lines(
        &mut self,
        a: &str,
        b: &str,
        style: Option<rnote_compose::geometry::LineStyle>,
    ) -> (bool, WidgetFlags) {
        let norm = |s: &str| s.trim().to_uppercase();
        let wanted = [norm(a), norm(b)];
        let mut found = false;
        let mut changed = Vec::new();
        for key in self.store.stroke_keys_unordered() {
            let Some(crate::strokes::Stroke::ShapeStroke(ss)) = self.store.get_stroke_mut(key)
            else {
                continue;
            };
            let rnote_compose::Shape::MathFigure(fig) = &mut ss.shape else {
                continue;
            };
            let name = |id| {
                fig.points
                    .get(&id)
                    .and_then(|p| p.label.as_deref())
                    .map(norm)
            };
            let hits: Vec<_> = fig
                .lines
                .values()
                .filter(|l| {
                    let ends = [name(l.p1), name(l.p2)];
                    ends == [Some(wanted[0].clone()), Some(wanted[1].clone())]
                        || ends == [Some(wanted[1].clone()), Some(wanted[0].clone())]
                })
                .map(|l| l.id)
                .collect();
            found |= !hits.is_empty();
            if let Some(style) = style
                && !hits.is_empty()
            {
                for id in hits {
                    fig.lines.get_mut(&id).unwrap().style = style;
                }
                changed.push(key);
            }
        }
        let mut widget_flags = WidgetFlags::default();
        if !changed.is_empty() {
            self.store.update_geometry_for_strokes(&changed);
            for &key in &changed {
                self.store.regenerate_rendering_for_stroke(
                    key,
                    self.camera.viewport(),
                    self.camera.image_scale(),
                );
            }
            widget_flags = self.record(Instant::now());
            widget_flags.store_modified = true;
            widget_flags.redraw = true;
        }
        (found, widget_flags)
    }

    /// Named geometry points on the page, e.g. as context for the AI assistant.
    pub fn named_figure_points(&self) -> Vec<(String, Vector2)> {
        let mut points: Vec<(String, Vector2)> = self
            .math_figures()
            .flat_map(|(_, fig)| {
                fig.points
                    .values()
                    .filter_map(|p| Some((p.label.clone()?, p.pos)))
            })
            .collect();
        points.sort_by(|a, b| a.0.cmp(&b.0));
        points.dedup_by(|a, b| a.0 == b.0);
        points
    }

    /// Returns the stroke key of the shape currently clicked/selected by the user
    /// (either in Shaper manipulation mode or in Selector mode).
    pub fn selected_shape_key(&self) -> Option<StrokeKey> {
        if let Some(key) = self.penholder.selected_shaper_stroke()
            && self.store.get_stroke_ref(key).is_some()
        {
            return Some(key);
        }
        self.store
            .selection_keys_as_rendered()
            .into_iter()
            .find(|&k| {
                matches!(
                    self.store.get_stroke_ref(k),
                    Some(crate::strokes::Stroke::ShapeStroke(_))
                )
            })
    }

    /// Clear any active shape selection in Shaper or Selector.
    pub fn clear_shape_selection(&mut self) -> WidgetFlags {
        let mut flags = WidgetFlags::default();
        if self.penholder.clear_shaper_selection() {
            flags.redraw = true;
        }
        let sel = self.store.selection_keys_unordered();
        if !sel.is_empty() {
            self.store.set_selected_keys(&sel, false);
            flags.redraw = true;
            flags.store_modified = true;
        }
        flags
    }

    /// Short human-readable summary of the currently selected shape for the AI menu UI.
    pub fn selected_shape_summary(&self) -> Option<(StrokeKey, String)> {
        let key = self.selected_shape_key()?;
        let crate::strokes::Stroke::ShapeStroke(ss) = self.store.get_stroke_ref(key)? else {
            return None;
        };
        let summary = match &ss.shape {
            rnote_compose::Shape::MathFigure(fig) => {
                let mut labels: Vec<String> = fig
                    .points
                    .values()
                    .filter_map(|p| p.label.clone())
                    .collect();
                labels.sort();
                if !fig.circles.is_empty() {
                    if labels.is_empty() {
                        format!("Đường tròn ({} điểm)", fig.points.len())
                    } else {
                        format!("Đường tròn ({})", labels.join(", "))
                    }
                } else if fig.lines.len() == 3 && fig.points.len() >= 3 {
                    format!("Tam giác {}", labels.join(""))
                } else if fig.lines.len() == 4 && fig.points.len() >= 4 {
                    format!("Tứ giác {}", labels.join(""))
                } else if !labels.is_empty() {
                    format!("Hình ({})", labels.join(", "))
                } else {
                    format!("Hình ({} điểm, {} đoạn)", fig.points.len(), fig.lines.len())
                }
            }
            rnote_compose::Shape::Ellipse(e) => {
                if (e.radii.x - e.radii.y).abs() < 1.0 {
                    format!("Hình tròn (R={:.0})", e.radii.x)
                } else {
                    format!("Hình elip ({:.0}x{:.0})", e.radii.x, e.radii.y)
                }
            }
            rnote_compose::Shape::Rectangle(r) => {
                format!(
                    "Hình chữ nhật ({:.0}x{:.0})",
                    r.cuboid.half_extents.x * 2.0,
                    r.cuboid.half_extents.y * 2.0
                )
            }
            rnote_compose::Shape::Line(l) => {
                format!("Đoạn thẳng (dài {:.0})", (l.end - l.start).length())
            }
            _ => String::from("Hình đang chọn"),
        };
        Some((key, summary))
    }

    /// Convert a selected `ShapeStroke` into a `MathFigure` (preserving existing `MathFigure` or
    /// converting primitive `Ellipse`, `Rectangle`, `Line`, `Polygon` into an editable `MathFigure`).
    pub fn stroke_as_math_figure(&self, key: StrokeKey) -> Option<rnote_compose::MathFigure> {
        let crate::strokes::Stroke::ShapeStroke(ss) = self.store.get_stroke_ref(key)? else {
            return None;
        };
        let fig = match &ss.shape {
            rnote_compose::Shape::MathFigure(f) => f.clone(),
            rnote_compose::Shape::Ellipse(e) => {
                let mut f = rnote_compose::MathFigure::new();
                let r = (e.radii.x + e.radii.y) * 0.5;
                f.build_circle(e.affine.translation, r, Some("O"), Some("M"));
                f
            }
            rnote_compose::Shape::Rectangle(r) => {
                let mut f = rnote_compose::MathFigure::new();
                let center = r.affine.translation;
                let he = r.cuboid.half_extents;
                let top_left = center - he;
                f.build_rectangle(top_left, he.x * 2.0, he.y * 2.0, Some(["A", "B", "C", "D"]));
                f
            }
            rnote_compose::Shape::Line(l) => {
                let mut f = rnote_compose::MathFigure::new();
                f.build_line(l.start, l.end, Some("A"), Some("B"));
                f
            }
            _ => return None,
        };
        Some(fig)
    }

    /// Detailed geometric description and reference origin of the selected shape for the AI prompt.
    pub fn selected_shape_ai_context(&self, key: StrokeKey) -> Option<(Vector2, String)> {
        let fig = self.stroke_as_math_figure(key)?;
        let origin = if let Some(circ) = fig.circles.values().next()
            && let Some(c_pt) = fig.points.get(&circ.center)
        {
            c_pt.pos
        } else if !fig.points.is_empty() {
            let sum: Vector2 = fig.points.values().map(|p| p.pos).sum();
            sum / (fig.points.len() as f64)
        } else {
            self.camera.viewport().center()
        };

        let mut pts_desc = Vec::new();
        let mut sorted_pts: Vec<_> = fig.points.values().collect();
        sorted_pts.sort_by_key(|p| p.id);
        for p in sorted_pts {
            let rel = p.pos - origin;
            let name = p.label.as_deref().unwrap_or("(giao điểm)");
            pts_desc.push(format!("{name}({:.0},{:.0})", rel.x, rel.y));
        }

        let mut lines_desc = Vec::new();
        let mut sorted_lines: Vec<_> = fig.lines.values().collect();
        sorted_lines.sort_by_key(|l| l.id);
        for l in sorted_lines {
            let n1 = fig
                .points
                .get(&l.p1)
                .and_then(|p| p.label.as_deref())
                .unwrap_or("?");
            let n2 = fig
                .points
                .get(&l.p2)
                .and_then(|p| p.label.as_deref())
                .unwrap_or("?");
            let st = match l.style {
                rnote_compose::geometry::LineStyle::Solid => "solid",
                rnote_compose::geometry::LineStyle::Dashed => "dashed",
                rnote_compose::geometry::LineStyle::Dotted => "dotted",
            };
            lines_desc.push(format!("{n1}-{n2}({st})"));
        }

        let mut circ_desc = Vec::new();
        for c in fig.circles.values() {
            let cname = fig
                .points
                .get(&c.center)
                .and_then(|p| p.label.as_deref())
                .unwrap_or("O");
            circ_desc.push(format!("tâm {cname} bán kính {:.0}", c.radius));
        }

        let desc = format!(
            "Điểm (tọa độ tương đối so với gốc {:.0},{:.0}): [{}]; Đoạn thẳng: [{}]; Đường tròn: [{}]",
            origin.x,
            origin.y,
            pts_desc.join(", "),
            lines_desc.join(", "),
            circ_desc.join(", ")
        );
        Some((origin, desc))
    }

    /// Replace the shape of an existing stroke `key` in-place with the updated `MathFigure`,
    /// recomputing intersections and regenerating rendering so the user's selected shape is modified directly.
    pub fn update_shape_with_figure(
        &mut self,
        key: StrokeKey,
        mut fig: rnote_compose::MathFigure,
    ) -> WidgetFlags {
        if self
            .config
            .read()
            .pens_config
            .shaper_config
            .math_auto_intersections
        {
            fig.auto_detect_intersections();
            let mut segments = Vec::new();
            let mut circles = Vec::new();
            for other_key in self.store.stroke_keys_unordered() {
                if other_key == key {
                    continue;
                }
                let Some(crate::strokes::Stroke::ShapeStroke(ss)) =
                    self.store.get_stroke_ref(other_key)
                else {
                    continue;
                };
                if let rnote_compose::Shape::MathFigure(other_fig) = &ss.shape {
                    segments.extend(other_fig.lines.values().filter_map(|l| {
                        Some((
                            other_fig.points.get(&l.p1)?.pos,
                            other_fig.points.get(&l.p2)?.pos,
                        ))
                    }));
                    circles.extend(
                        other_fig
                            .circles
                            .values()
                            .filter_map(|c| Some((other_fig.points.get(&c.center)?.pos, c.radius))),
                    );
                }
            }
            fig.add_intersections_with(&segments, &circles);
        }

        if let Some(crate::strokes::Stroke::ShapeStroke(ss)) = self.store.get_stroke_mut(key) {
            ss.shape = rnote_compose::Shape::MathFigure(fig);
        } else {
            return self.insert_shape(rnote_compose::Shape::MathFigure(fig));
        }

        self.store.update_geometry_for_strokes(&[key]);
        self.store.regenerate_rendering_for_stroke(
            key,
            self.camera.viewport(),
            self.camera.image_scale(),
        );
        let mut widget_flags = self.record(Instant::now());
        widget_flags |= self.document.resize_autoexpand(&self.store, &self.camera);
        widget_flags.store_modified = true;
        widget_flags.redraw = true;
        widget_flags
    }
}
