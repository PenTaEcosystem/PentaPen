// Geometric entities (Points, Lines, Circles, Marks)
use p2d::math::Vector2;
use serde::{Deserialize, Serialize};

/// Unique identifier for a geometric point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PointId(pub usize);

/// Unique identifier for a geometric line or segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LineId(pub usize);

/// Unique identifier for a geometric circle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CircleId(pub usize);

/// Unique identifier for a geometric constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConstraintId(pub usize);

/// Visual style of a line or circle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LineStyle {
    /// Solid line (main geometric figure)
    #[default]
    Solid,
    /// Dashed line (auxiliary construction)
    Dashed,
    /// Dotted line
    Dotted,
    /// Dash-dot line (axes of symmetry, hidden edges)
    DashDot,
}

/// Type of line representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LineType {
    /// Finite segment connecting p1 and p2
    #[default]
    Segment,
    /// Infinite line passing through p1 and p2
    Infinite,
    /// Ray starting from p1 and passing through p2
    Ray,
}

/// A geometric 2D point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeomPoint {
    /// Unique point ID
    pub id: PointId,
    /// 2D position vector
    pub pos: Vector2,
    /// Optional label (e.g. "A", "B", "C", "G", "H", "O", "I")
    pub label: Option<String>,
    /// Whether this point is rendered
    pub visible: bool,
    /// Whether this point's position is fixed/pinned
    pub fixed: bool,
}

impl GeomPoint {
    /// Create a new geometric point.
    pub fn new(id: PointId, pos: Vector2, label: Option<String>) -> Self {
        Self {
            id,
            pos,
            label,
            visible: true,
            fixed: false,
        }
    }
}

/// A geometric line, ray or segment connecting or passing through two points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeomLine {
    /// Unique line ID
    pub id: LineId,
    /// Start or defining point 1
    pub p1: PointId,
    /// End or defining point 2
    pub p2: PointId,
    /// Segment, Infinite line, or Ray
    pub line_type: LineType,
    /// Visual style (Solid, Dashed, Dotted)
    pub style: LineStyle,
    /// Visual congruence tick marks (0: none, 1: single, 2: double, 3: triple)
    pub tick_marks: u8,
    /// Whether this line is visible
    pub visible: bool,
}

impl GeomLine {
    /// Create a new line segment.
    pub fn new_segment(id: LineId, p1: PointId, p2: PointId) -> Self {
        Self {
            id,
            p1,
            p2,
            line_type: LineType::Segment,
            style: LineStyle::Solid,
            tick_marks: 0,
            visible: true,
        }
    }

    /// Create an auxiliary dashed line.
    pub fn new_auxiliary(id: LineId, p1: PointId, p2: PointId, line_type: LineType) -> Self {
        Self {
            id,
            p1,
            p2,
            line_type,
            style: LineStyle::Dashed,
            tick_marks: 0,
            visible: true,
        }
    }
}

/// A geometric circle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeomCircle {
    /// Unique circle ID
    pub id: CircleId,
    /// Center point ID
    pub center: PointId,
    /// Radius of the circle
    pub radius: f64,
    /// Optional point on the circumference that defines the radius
    pub through_point: Option<PointId>,
    /// Visual style
    pub style: LineStyle,
    /// Whether this circle is visible
    pub visible: bool,
}

impl GeomCircle {
    /// Create a circle with fixed radius.
    pub fn new_with_radius(id: CircleId, center: PointId, radius: f64) -> Self {
        Self {
            id,
            center,
            radius,
            through_point: None,
            style: LineStyle::Solid,
            visible: true,
        }
    }

    /// Create a circle defined by center and a point on the perimeter.
    pub fn new_through_point(
        id: CircleId,
        center: PointId,
        through: PointId,
        initial_radius: f64,
    ) -> Self {
        Self {
            id,
            center,
            radius: initial_radius,
            through_point: Some(through),
            style: LineStyle::Solid,
            visible: true,
        }
    }
}

/// Right angle indicator at a vertex between two arms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RightAngleMark {
    /// Corner vertex point ID
    pub vertex: PointId,
    /// First arm point ID
    pub arm1: PointId,
    /// Second arm point ID
    pub arm2: PointId,
    /// Size of the right-angle square glyph
    pub size: f64,
}
