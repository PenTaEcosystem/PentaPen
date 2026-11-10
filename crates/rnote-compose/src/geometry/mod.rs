// Geometry module: Construction (Geo-Aid) and Constraint Solver (SolveSpace slvs)
/// Classical ruler-and-compass geometric constructions (Geo-Aid architecture)
pub mod construction;
/// Geometric entity representations (Points, Lines, Circles, Marks)
pub mod entity;
/// Mathematical figures with dynamic entities and constraints
pub mod figure;
/// Specialized mathematical rendering using Piet
pub mod render;
/// 2D non-linear geometric constraint solver
pub mod solver;
/// Triangles, their 4 classical centers and geometric elements
pub mod triangle;

#[cfg(test)]
mod tests;

pub use construction::*;
pub use entity::*;
pub use figure::MathFigure;
pub use render::draw_math_figure;
pub use solver::{Constraint, ConstraintSolver};
pub use triangle::{Triangle, TriangleCenterType};

use crate::shapes::Shapeable;
use crate::style::Composer;
use crate::style::Style;
use crate::style::rough::RoughOptions;
use crate::style::smooth::SmoothOptions;
use p2d::bounding_volume::{Aabb, BoundingVolume};

impl Composer<SmoothOptions> for MathFigure {
    fn composed_bounds(&self, options: &SmoothOptions) -> Aabb {
        self.bounds().loosened(options.stroke_width * 0.5 + 20.0) // padding for labels
    }

    fn draw_composed(&self, cx: &mut impl piet::RenderContext, options: &SmoothOptions) {
        if let Some(stroke_color) = options.stroke_color {
            draw_math_figure(self, cx, stroke_color.into(), options.stroke_width);
        }
    }
}

impl Composer<RoughOptions> for MathFigure {
    fn composed_bounds(&self, options: &RoughOptions) -> Aabb {
        self.bounds().loosened(options.stroke_width * 0.5 + 20.0)
    }

    fn draw_composed(&self, cx: &mut impl piet::RenderContext, options: &RoughOptions) {
        if let Some(stroke_color) = options.stroke_color {
            draw_math_figure(self, cx, stroke_color.into(), options.stroke_width);
        }
    }
}

impl Composer<Style> for MathFigure {
    fn composed_bounds(&self, options: &Style) -> Aabb {
        match options {
            Style::Smooth(opts) => self.composed_bounds(opts),
            Style::Rough(opts) => self.composed_bounds(opts),
            Style::Textured(_) => self.bounds(),
        }
    }

    fn draw_composed(&self, cx: &mut impl piet::RenderContext, options: &Style) {
        match options {
            Style::Smooth(opts) => self.draw_composed(cx, opts),
            Style::Rough(opts) => self.draw_composed(cx, opts),
            Style::Textured(_) => {}
        }
    }
}
