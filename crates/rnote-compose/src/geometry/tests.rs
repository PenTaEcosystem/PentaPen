// Tests for Triangle, Construction, and ConstraintSolver
use super::construction::*;
use super::figure::MathFigure;
use super::solver::Constraint;
use super::triangle::{Triangle, TriangleCenterType};
use p2d::math::Vector2;

#[test]
fn test_triangle_centroid() {
    let a = Vector2::new(0.0, 0.0);
    let b = Vector2::new(6.0, 0.0);
    let c = Vector2::new(0.0, 6.0);
    let tri = Triangle::new(a, b, c);

    let g = tri.centroid();
    assert!((g.x - 2.0).abs() < 1e-9);
    assert!((g.y - 2.0).abs() < 1e-9);
}

#[test]
fn test_triangle_circumcenter_and_orthocenter_right_triangle() {
    // Right triangle at A(0,0), B(6,0), C(0,8)
    let a = Vector2::new(0.0, 0.0);
    let b = Vector2::new(6.0, 0.0);
    let c = Vector2::new(0.0, 8.0);
    let tri = Triangle::new(a, b, c);

    // Circumcenter must be midpoint of hypotenuse BC: (3, 4)
    let o = tri.circumcenter().unwrap();
    assert!((o.x - 3.0).abs() < 1e-9);
    assert!((o.y - 4.0).abs() < 1e-9);

    // Orthocenter must be at vertex A (0,0)
    let h = tri.orthocenter().unwrap();
    assert!((h.x - 0.0).abs() < 1e-9);
    assert!((h.y - 0.0).abs() < 1e-9);

    // Euler relation: H = 3G - 2O
    let g = tri.centroid();
    let h_euler = 3.0 * g - 2.0 * o;
    assert!((h.x - h_euler.x).abs() < 1e-9);
    assert!((h.y - h_euler.y).abs() < 1e-9);
}

#[test]
fn test_triangle_incenter_3_4_5() {
    // 3-4-5 right triangle at A(0,0), B(4,0), C(0,3)
    let a = Vector2::new(0.0, 0.0);
    let b = Vector2::new(4.0, 0.0);
    let c = Vector2::new(0.0, 3.0);
    let tri = Triangle::new(a, b, c);

    // Inradius r = Area / p = 6 / 6 = 1.0
    let r = tri.inradius().unwrap();
    assert!((r - 1.0).abs() < 1e-9);

    // Incenter should be at (1, 1)
    let i = tri.incenter().unwrap();
    assert!((i.x - 1.0).abs() < 1e-9);
    assert!((i.y - 1.0).abs() < 1e-9);
}

#[test]
fn test_tangents_from_point_to_circle() {
    let center = Vector2::new(0.0, 0.0);
    let radius = 3.0;
    let p = Vector2::new(0.0, 5.0);

    let tangents = tangents_from_point_to_circle(p, center, radius);
    assert_eq!(tangents.len(), 2);

    for t in tangents {
        // Distance from center to tangent point must be radius
        assert!((t.length() - radius).abs() < 1e-9);
        // Tangent line PT must be perpendicular to radius CT: (P - T) . (T - C) = 0
        let pt = p - t;
        let ct = t - center;
        assert!(pt.dot(ct).abs() < 1e-8);
    }
}

#[test]
fn test_circle_circle_intersection() {
    let c1 = Vector2::new(0.0, 0.0);
    let r1 = 5.0;
    let c2 = Vector2::new(6.0, 0.0);
    let r2 = 5.0;

    let pts = circle_circle_intersection(c1, r1, c2, r2);
    assert_eq!(pts.len(), 2);

    for p in pts {
        assert!(((p - c1).length() - r1).abs() < 1e-9);
        assert!(((p - c2).length() - r2).abs() < 1e-9);
        assert!((p.x - 3.0).abs() < 1e-9); // Symmetry along x = 3
    }
}

#[test]
fn test_constraint_solver_equal_length_and_perpendicular() {
    // Triangle with vertices P1(0,0), P2(10,0), P3(2, 8)
    // Constrain:
    // 1. P1 is fixed at (0,0)
    // 2. P2 is fixed at (10,0)
    // 3. Segment P1-P3 is perpendicular to P1-P2
    // 4. Equal length |P1 - P3| = |P1 - P2| = 10
    let mut figure = MathFigure::new();
    let p1 = figure.add_point(Vector2::new(0.0, 0.0), Some("A".to_string()));
    let p2 = figure.add_point(Vector2::new(10.0, 0.0), Some("B".to_string()));
    let p3 = figure.add_point(Vector2::new(3.0, 7.0), Some("C".to_string()));

    figure.points.get_mut(&p1).unwrap().fixed = true;
    figure.points.get_mut(&p2).unwrap().fixed = true;

    let c_perp_id = figure.next_constraint_id();
    figure.add_constraint(Constraint::Perpendicular {
        id: c_perp_id,
        p1,
        p2,
        p3: p1,
        p4: p3,
    });

    let c_eq_id = figure.next_constraint_id();
    figure.add_constraint(Constraint::EqualLength {
        id: c_eq_id,
        p1,
        p2,
        p3: p1,
        p4: p3,
    });

    // Initial position of P3 is (3, 7). Drag toward (0, 12).
    let solved = figure.drag_point(p3, Vector2::new(0.0, 12.0));
    assert!(solved);

    let pos3 = figure.points[&p3].pos;
    // P3 must converge to (0, 10) (or (0, -10))
    assert!(pos3.x.abs() < 1e-3, "P3.x should be ~0, got {}", pos3.x);
    assert!(
        (pos3.y.abs() - 10.0).abs() < 1e-3,
        "P3.y should be ~10, got {}",
        pos3.y
    );
}

#[test]
fn test_math_figure_triangle_with_4_centers() {
    let mut figure = MathFigure::new();
    let a = Vector2::new(0.0, 0.0);
    let b = Vector2::new(60.0, 0.0);
    let c = Vector2::new(20.0, 50.0);

    let (pa, pb, pc, _, _, _) = figure.build_triangle(a, b, c, Some(["A", "B", "C"]));
    let g = figure.add_triangle_center(pa, pb, pc, TriangleCenterType::Centroid, true);
    let o = figure.add_triangle_center(pa, pb, pc, TriangleCenterType::Circumcenter, true);
    let h = figure.add_triangle_center(pa, pb, pc, TriangleCenterType::Orthocenter, true);
    let _i = figure.add_triangle_center(pa, pb, pc, TriangleCenterType::Incenter, true);

    assert!(g.is_some());
    assert!(o.is_some());
    assert!(h.is_some());
    // 3 vertices + 4 centers + 3 medians feet + 2 altitude feet + 2 bisector feet = 14
    assert_eq!(figure.points.len(), 14);
}

#[test]
fn test_math_circle_with_center_and_radius() {
    let mut figure = MathFigure::new();
    let center = Vector2::new(100.0, 100.0);
    let radius = 50.0;

    let (c_id, b_id, circ_id, _line_id) = figure.build_circle(center, radius, Some("O"), Some("M"));

    assert_eq!(figure.points[&c_id].label.as_deref(), Some("O"));
    assert_eq!(figure.points[&b_id].label.as_deref(), Some("M"));
    assert_eq!(figure.circles[&circ_id].center, c_id);
    assert!((figure.circles[&circ_id].radius - radius).abs() < 1e-9);
}

#[test]
fn test_auto_intersections_line_circle_and_line_line() {
    let mut figure = MathFigure::new();
    // Circle at (0, 0), radius 5
    figure.build_circle(Vector2::new(0.0, 0.0), 5.0, Some("O"), Some("M"));

    // Horizontal line passing through y = 3 from x = -10 to x = 10
    // Intersects circle at (-4, 3) and (4, 3) because (-4)^2 + 3^2 = 25
    figure.build_line(
        Vector2::new(-10.0, 3.0),
        Vector2::new(10.0, 3.0),
        Some("A"),
        Some("B"),
    );

    // Vertical line along x = 0 from y = -10 to y = 10
    // Intersects horizontal line at (0, 3) and circle at (0, 5) and (0, -5)
    figure.build_line(
        Vector2::new(0.0, -10.0),
        Vector2::new(0.0, 10.0),
        Some("C"),
        Some("D"),
    );

    let new_pts = figure.auto_detect_intersections();
    assert!(!new_pts.is_empty());

    // Check that intersection (-4, 3) or (4, 3) was detected
    let has_inter_4_3 = figure
        .points
        .values()
        .any(|pt| (pt.pos - Vector2::new(4.0, 3.0)).length() < 1e-3);
    let has_inter_neg4_3 = figure
        .points
        .values()
        .any(|pt| (pt.pos - Vector2::new(-4.0, 3.0)).length() < 1e-3);
    let has_inter_0_3 = figure
        .points
        .values()
        .any(|pt| (pt.pos - Vector2::new(0.0, 3.0)).length() < 1e-3);

    assert!(
        has_inter_4_3,
        "Should detect (4, 3) line-circle intersection"
    );
    assert!(
        has_inter_neg4_3,
        "Should detect (-4, 3) line-circle intersection"
    );
    assert!(has_inter_0_3, "Should detect (0, 3) line-line intersection");
}

#[test]
fn test_auto_detect_triangles_and_centers() {
    let mut figure = MathFigure::new();
    let p1 = figure.add_point(Vector2::new(0.0, 0.0), Some("A".to_string()));
    let p2 = figure.add_point(Vector2::new(60.0, 0.0), Some("B".to_string()));
    let p3 = figure.add_point(Vector2::new(20.0, 50.0), Some("C".to_string()));

    // Connect segments to form triangle
    figure.add_segment(p1, p2);
    figure.add_segment(p2, p3);
    figure.add_segment(p3, p1);

    figure.auto_detect_triangles_and_centers(true);

    // Centroid G, Circumcenter O, Orthocenter H, Incenter I must be present
    let labels: Vec<String> = figure
        .points
        .values()
        .filter_map(|p| p.label.clone())
        .collect();
    assert!(
        labels.contains(&"G".to_string()),
        "Centroid G should be detected"
    );
    assert!(
        labels.contains(&"O".to_string()),
        "Circumcenter O should be detected"
    );
    assert!(
        labels.contains(&"H".to_string()),
        "Orthocenter H should be detected"
    );
    assert!(
        labels.contains(&"I".to_string()),
        "Incenter I should be detected"
    );
}
