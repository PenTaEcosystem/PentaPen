//! Strokes appearing as if drawn by hand (the AI's figures, graphs and tables).
//!
//! The strokes are recorded finished first, so undo always sees the whole drawing; only what the
//! canvas shows is rewound and played forward frame by frame.

use super::Engine;
use crate::WidgetFlags;
use crate::store::StrokeKey;
use crate::strokes::Stroke;
use rnote_compose::{MathFigure, Shape};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug)]
enum Frame {
    /// Bring back a stroke hidden at the start
    Show(StrokeKey),
    /// Show a stroke's shape as drawn so far
    Set(StrokeKey, Shape),
}

/// Frames that still have to be shown, see [Engine::draw_animation].
#[derive(Debug, Default)]
pub struct DrawAnimation {
    frames: VecDeque<Frame>,
    /// The shape each animated stroke showed after our last frame
    shown: HashMap<StrokeKey, Shape>,
    /// Strokes changed by someone else meanwhile (undo, an edit): left alone from then on
    dropped: HashSet<StrokeKey>,
}

impl DrawAnimation {
    pub fn is_done(&self) -> bool {
        self.frames.is_empty()
    }
}

/// Whether the stroke still shows what the animation left there.
fn same_shape(a: &Shape, b: &Shape) -> bool {
    match (a, b) {
        (Shape::MathFigure(x), Shape::MathFigure(y)) => x == y,
        (Shape::Polyline(x), Shape::Polyline(y)) => {
            x.start == y.start && x.path.len() == y.path.len()
        }
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Frames for a growing polyline (curves of function graphs).
const POLYLINE_STEPS: usize = 10;

impl Engine {
    /// Rewind the just recorded strokes `keys` (in drawing order) and queue the frames that draw
    /// them again: figures element by element, curves growing, everything else one stroke at a time.
    /// `base` is the figure an in-place edit started from: its elements are not redrawn.
    pub fn draw_animation(
        &mut self,
        keys: &[StrokeKey],
        base: Option<&MathFigure>,
    ) -> (DrawAnimation, WidgetFlags) {
        let mut anim = DrawAnimation::default();
        let new_lines: usize = keys
            .iter()
            .filter_map(|&k| match self.store.get_stroke_ref(k) {
                Some(Stroke::ShapeStroke(ss)) => match &ss.shape {
                    Shape::MathFigure(f) => Some(f.lines.len()),
                    _ => None,
                },
                _ => None,
            })
            .sum::<usize>()
            .saturating_sub(base.map_or(0, |b| b.lines.len()));
        // Many lines draw faster, so a big figure does not take ages
        let line_steps = (48 / new_lines.max(1)).clamp(2, 8);

        for &key in keys {
            let shape = match self.store.get_stroke_ref(key) {
                Some(Stroke::ShapeStroke(ss)) => Some(ss.shape.clone()),
                Some(_) => None,
                None => continue,
            };
            match shape {
                Some(Shape::MathFigure(fig)) => {
                    let mut frames = fig.drawing_frames(base, line_steps).into_iter();
                    let Some(first) = frames.next() else { continue };
                    self.set_stroke_shape(key, Shape::MathFigure(first.clone()));
                    anim.shown.insert(key, Shape::MathFigure(first));
                    anim.frames
                        .extend(frames.map(|f| Frame::Set(key, Shape::MathFigure(f))));
                }
                Some(Shape::Polyline(pl)) if pl.path.len() > POLYLINE_STEPS => {
                    let mut start = pl.clone();
                    start.path.truncate(1);
                    self.set_stroke_shape(key, Shape::Polyline(start.clone()));
                    anim.shown.insert(key, Shape::Polyline(start));
                    self.store.set_trashed(key, true);
                    anim.frames.push_back(Frame::Show(key));
                    for k in 1..=POLYLINE_STEPS {
                        let mut part = pl.clone();
                        part.path.truncate(pl.path.len() * k / POLYLINE_STEPS);
                        anim.frames
                            .push_back(Frame::Set(key, Shape::Polyline(part)));
                    }
                }
                _ => {
                    self.store.set_trashed(key, true);
                    anim.frames.push_back(Frame::Show(key));
                }
            }
        }
        let widget_flags = WidgetFlags {
            redraw: true,
            ..Default::default()
        };
        (anim, widget_flags)
    }

    /// Show the next frame. Returns false once the animation is done.
    pub fn draw_animation_step(&mut self, anim: &mut DrawAnimation) -> (bool, WidgetFlags) {
        let mut widget_flags = WidgetFlags::default();
        while let Some(frame) = anim.frames.pop_front() {
            let key = match &frame {
                Frame::Show(k) | Frame::Set(k, _) => *k,
            };
            if anim.dropped.contains(&key) {
                continue;
            }
            let untouched = match (&frame, self.store.get_stroke_ref(key)) {
                (_, None) => false,
                (Frame::Show(_), Some(_)) => self.store.trashed(key) == Some(true),
                (Frame::Set(..), Some(Stroke::ShapeStroke(ss))) => anim
                    .shown
                    .get(&key)
                    .is_none_or(|s| same_shape(&ss.shape, s)),
                (Frame::Set(..), Some(_)) => false,
            };
            if !untouched {
                anim.dropped.insert(key);
                continue;
            }
            match frame {
                Frame::Show(key) => {
                    self.store.set_trashed(key, false);
                    self.store.regenerate_rendering_for_stroke(
                        key,
                        self.camera.viewport(),
                        self.camera.image_scale(),
                    );
                }
                Frame::Set(key, shape) => {
                    self.set_stroke_shape(key, shape.clone());
                    anim.shown.insert(key, shape);
                }
            }
            widget_flags.redraw = true;
            break;
        }
        (!anim.frames.is_empty(), widget_flags)
    }

    /// Show the rest at once (all strokes as recorded).
    pub fn draw_animation_finish(&mut self, anim: &mut DrawAnimation) -> WidgetFlags {
        let mut widget_flags = WidgetFlags::default();
        while !anim.is_done() {
            widget_flags |= self.draw_animation_step(anim).1;
        }
        widget_flags
    }

    fn set_stroke_shape(&mut self, key: StrokeKey, shape: Shape) {
        if let Some(Stroke::ShapeStroke(ss)) = self.store.get_stroke_mut(key) {
            ss.shape = shape;
            self.store.update_geometry_for_strokes(&[key]);
            self.store.regenerate_rendering_for_stroke(
                key,
                self.camera.viewport(),
                self.camera.image_scale(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2d::math::Vector2;

    #[test]
    fn replays_to_the_recorded_strokes_and_yields_to_undo() {
        let mut engine = Engine::default();
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(100.0, 0.0), Some("B".into()));
        let c = fig.add_point(Vector2::new(0.0, 80.0), Some("C".into()));
        fig.add_segment(a, b);
        fig.add_segment(b, c);
        let (_, key) = engine.insert_shape_key(Shape::MathFigure(fig.clone()));

        let (mut anim, _) = engine.draw_animation(&[key], None);
        let shown = |e: &Engine| match e.store.get_stroke_ref(key) {
            Some(Stroke::ShapeStroke(ss)) => match &ss.shape {
                Shape::MathFigure(f) => f.clone(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        assert!(shown(&engine).lines.values().all(|l| !l.visible));
        let mut steps = 0;
        while engine.draw_animation_step(&mut anim).0 {
            steps += 1;
        }
        assert!(steps > 2);
        assert_eq!(shown(&engine), fig);

        // An undo during the animation wins: nothing is drawn over it
        let (mut anim, _) = engine.draw_animation(&[key], None);
        let _ = engine.undo(std::time::Instant::now());
        let _ = engine.draw_animation_finish(&mut anim);
        assert!(engine.store.get_stroke_ref(key).is_none());
    }
}
