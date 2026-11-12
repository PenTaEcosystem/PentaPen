// Imports
use crate::appwindow::RnAppWindow;
use gtk4::{
    Button, CompositeTemplate, Entry, Label, PasswordEntry, ScrolledWindow, TextView, Widget, gdk,
    glib, pango, prelude::*, subclass::prelude::*,
};
use p2d::math::Vector2;
use rnote_compose::Shape;
use rnote_compose::geometry::triangle::TriangleCenterType;
use rnote_compose::geometry::{LineStyle, MathFigure, PointId};
use rnote_engine::graph::{Function, GraphFrame};
use rnote_engine::store::StrokeKey;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Connection to an OpenAI-compatible chat completions API (Gemini, OpenAI, a proxy, a custom server).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AiConfig {
    /// One of the [PROVIDERS] ids; empty in configs saved before providers existed
    pub provider: String,
    /// e.g. `https://api.openai.com/v1`; `/chat/completions` is appended.
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    /// Off by default: the AI's drawings appear stroke by stroke unless this is set
    pub instant_draw: bool,
}

/// A service with an OpenAI-compatible API: id, base URL, suggested models, where to get a key (markup).
struct Provider {
    id: &'static str,
    base_url: &'static str,
    models: &'static [&'static str],
    key_hint: &'static str,
}

/// In the order of the dropdown in aimenu.ui.
const PROVIDERS: [Provider; 4] = [
    Provider {
        id: "gemini",
        // Gemini's OpenAI-compatible endpoint: same requests, Bearer key
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        models: &[
            "gemini-2.5-flash",
            "gemini-2.5-pro",
            "gemini-2.5-flash-lite",
            "gemini-2.0-flash",
        ],
        key_hint: "Lấy API key miễn phí tại <a href=\"https://aistudio.google.com/apikey\">aistudio.google.com/apikey</a> (đăng nhập Google, bấm Create API key).",
    },
    Provider {
        id: "openai",
        base_url: "https://api.openai.com/v1",
        models: &["gpt-4o-mini", "gpt-4o", "gpt-4.1", "gpt-4.1-mini"],
        key_hint: "API key tại <a href=\"https://platform.openai.com/api-keys\">platform.openai.com/api-keys</a>.",
    },
    Provider {
        id: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        models: &[
            "google/gemini-2.5-flash",
            "google/gemini-2.5-pro",
            "openai/gpt-4o-mini",
        ],
        key_hint: "Một key dùng được nhiều hãng: <a href=\"https://openrouter.ai/keys\">openrouter.ai/keys</a>.",
    },
    Provider {
        id: "custom",
        base_url: "",
        models: &[],
        key_hint: "Máy chủ bất kỳ có API /chat/completions kiểu OpenAI, vd. Ollama: http://localhost:11434/v1 (không cần key).",
    },
];

/// The provider of a config; old configs are recognized by their URL.
fn provider_index(cfg: &AiConfig) -> usize {
    if let Some(i) = PROVIDERS.iter().position(|p| p.id == cfg.provider) {
        return i;
    }
    let url = cfg.base_url.trim();
    if url.is_empty() {
        return 0;
    }
    PROVIDERS
        .iter()
        .position(|p| !p.base_url.is_empty() && url.starts_with(p.base_url))
        .unwrap_or(PROVIDERS.len() - 1)
}

impl AiConfig {
    pub fn config_path() -> std::path::PathBuf {
        glib::user_config_dir().join("pentapen_ai_config.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::config_path())
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, data)
    }

    fn is_complete(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.model.trim().is_empty()
    }
}

const SYSTEM_PROMPT: &str = r#"You draw and edit plane-geometry figures on the canvas of the drawing app PentaPen.
Reply with ONLY one JSON object (no markdown, no prose outside it):
{"explanation": "<short answer in Vietnamese>",
 "replace": false,
 "points": [{"name": "A", "x": 0.0, "y": 0.0}],
 "lines": [{"p1": "A", "p2": "B", "style": "solid"}],
 "circles": [{"center": "O", "radius": 80.0}, {"center": "O", "through": "A"}],
 "right_angles": [{"vertex": "H", "arm1": "A", "arm2": "B"}],
 "constructions": [{"op": "altitude", "of": ["A", "B", "C"], "name": "H"}],
 "rename": [{"from": "A", "to": "M"}],
 "remove_points": ["D"],
 "remove_lines": [{"p1": "A", "p2": "B"}],
 "remove_circles": ["O"],
 "remove_all": false}
Rules:
- Coordinates are canvas pixels, x to the right, y DOWNWARDS. Place the figure around the given anchor, typical size 150-300 px.
- Give coordinates only for free points (e.g. the vertices of a triangle). Every derived point must come from a construction so it is exact:
  midpoint of [A,B]; altitude, median, bisector of [A,B,C] (from A to side BC, the foot is named `name`);
  perpendicular of [P,A,B] (foot of the perpendicular from P to line AB);
  centroid, orthocenter, circumcenter, incenter of [A,B,C]; circumcircle, incircle of [A,B,C] (center named `name`);
  tangents of [P,O] with "names": ["B","C"] (tangents from P to the circle centered at O, put that circle in "circles").
- Constructions run in order and may use points created by earlier ones. Draw sides yourself with "lines".
- When "Selected shape" is provided in the user prompt, your edits apply DIRECTLY to that selected shape in-place:
  - Keep `"replace": false` when adding constructions (altitude, median, bisector, circumcircle, incircle, tangents, etc.), adding lines/circles/points, restyling lines/circles, moving specific points (by listing them in `"points"` with new canvas coordinates), resizing an existing circle (by listing its `"center"` and new `"radius"` in `"circles"`), renaming points (`"rename"`), or removing elements (`"remove_points"`, `"remove_lines"`, `"remove_circles"`).
  - Set `"replace": true` ONLY when the user asks to replace or redraw the entire selected shape into a completely different shape (e.g. "đổi thành hình vuông", "vẽ lại thành tam giác đều").
- Respect requested lengths and angles. If the user gives lengths in cm, 1 cm = 40 px.
- Deleting (xoá, bỏ, gỡ): list what to delete in "remove_points" (a point with its lines and circles), "remove_lines" (by end point names) and "remove_circles" (by center name); draw nothing else. Without a selected shape this works on every figure on the canvas. "remove_all": true deletes ALL shapes on the canvas (xoá hết, xoá tất cả); use it only when the user asks to delete everything.
- "style" (lines and circles) is "solid", "dashed", "dotted" or "dashdot"; use dashed for auxiliary lines.
- To change an existing line (e.g. "make AB dashed"), list it in "lines" with the new style: lines between existing points are restyled, not redrawn.
- Intersections are added automatically.
Coordinate plane (hệ trục Oxy), function graphs (đồ thị hàm số), variation tables (bảng biến thiên):
- "axes": {"x_min": -5, "x_max": 5, "y_min": -5, "y_max": 5, "unit": 40} draws Oxy with O at the anchor.
  With "axes" (or "graphs"), ALL "points" coordinates are in axis units with y UPWARDS: A(1; 2) is {"name": "A", "x": 1, "y": 2}.
- "graphs": [{"f": "x^3-3*x+1", "table": true}] draws y = f(x) on the axes; "table": true also draws its variation table.
  Write f with + - * / ^ ( ), sqrt(), abs(), sin cos tan ln log exp, pi; the variable is x.
  Choose the window so extrema, intercepts and asymptotes are visible.
- "tables": [{"f": "(2*x+1)/(x-1)"}] draws only the variation table of f.
- Points, lines and constructions still work on the axes (e.g. A, B on the plane, segment AB, midpoint)."#;

#[derive(serde::Deserialize, Debug, Default)]
#[serde(default)]
struct GeometryJson {
    explanation: Option<String>,
    replace: bool,
    points: Vec<JsonPoint>,
    lines: Vec<JsonLine>,
    circles: Vec<JsonCircle>,
    right_angles: Vec<JsonRightAngle>,
    constructions: Vec<JsonConstruction>,
    rename: Vec<JsonRename>,
    remove_points: Vec<String>,
    remove_lines: Vec<JsonLineRef>,
    remove_circles: Vec<String>,
    /// Delete every shape on the page
    remove_all: bool,
    axes: Option<JsonAxes>,
    graphs: Vec<JsonGraph>,
    tables: Vec<JsonGraph>,
}

#[derive(serde::Deserialize, Debug, Default, Clone)]
#[serde(default)]
struct JsonAxes {
    x_min: Option<f64>,
    x_max: Option<f64>,
    y_min: Option<f64>,
    y_max: Option<f64>,
    unit: Option<f64>,
}

#[derive(serde::Deserialize, Debug)]
struct JsonGraph {
    f: String,
    #[serde(default)]
    table: bool,
}

/// The Oxy frame the reply asks for, with O at `anchor`; None for a plain geometry reply.
fn graph_frame(geom: &GeometryJson, anchor: Vector2) -> Option<GraphFrame> {
    if geom.axes.is_none() && geom.graphs.is_empty() {
        return None;
    }
    let a = geom.axes.clone().unwrap_or_default();
    let range = |lo: Option<f64>, hi: Option<f64>| {
        let lo = lo.unwrap_or(-5.0).clamp(-50.0, 49.0);
        (lo, hi.unwrap_or(5.0).clamp(lo + 1.0, 50.0))
    };
    Some(GraphFrame {
        origin: anchor,
        unit: a.unit.unwrap_or(40.0).clamp(10.0, 200.0),
        x_range: range(a.x_min, a.x_max),
        y_range: range(a.y_min, a.y_max),
    })
}

#[derive(serde::Deserialize, Debug)]
struct JsonRename {
    from: String,
    to: String,
}

#[derive(serde::Deserialize, Debug)]
struct JsonLineRef {
    p1: String,
    p2: String,
}

#[derive(serde::Deserialize, Debug)]
struct JsonPoint {
    name: String,
    x: f64,
    y: f64,
}

#[derive(serde::Deserialize, Debug)]
struct JsonLine {
    p1: String,
    p2: String,
    #[serde(default)]
    style: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct JsonCircle {
    center: String,
    #[serde(default)]
    radius: Option<f64>,
    #[serde(default)]
    through: Option<String>,
    #[serde(default)]
    style: Option<String>,
}

/// "dashed" / "dash" / "nét đứt" ... -> a line style; None when not given or not understood.
fn line_style(style: Option<&str>) -> Option<LineStyle> {
    let s = style?.to_lowercase();
    if s.contains("dashdot") || s.contains("dash-dot") || s.contains("chấm gạch") {
        Some(LineStyle::DashDot)
    } else if s.contains("dash") || s.contains("đứt") {
        Some(LineStyle::Dashed)
    } else if s.contains("dot") || s.contains("chấm") {
        Some(LineStyle::Dotted)
    } else if s.contains("solid") || s.contains("liền") {
        Some(LineStyle::Solid)
    } else {
        None
    }
}

#[derive(serde::Deserialize, Debug)]
struct JsonRightAngle {
    vertex: String,
    arm1: String,
    arm2: String,
}

#[derive(serde::Deserialize, Debug)]
struct JsonConstruction {
    op: String,
    #[serde(default)]
    of: Vec<String>,
    #[serde(default)]
    name: Option<String>,
    /// Names of several created points, e.g. the two tangent points
    #[serde(default)]
    names: Vec<String>,
}

/// The JSON object in a model reply, tolerating markdown fences or text around it.
fn parse_geometry(reply: &str) -> Result<GeometryJson, String> {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return Err("AI không trả về JSON hình học.".to_string());
    };
    serde_json::from_str(&reply[start..=end])
        .map_err(|e| format!("JSON hình học không hợp lệ: {e}"))
}

/// Build the figure the model described. Names not defined in the reply are looked up among
/// `existing` canvas points. Returns the figure and warnings for steps that could not be built.
fn build_figure(geom: &GeometryJson, existing: &[(String, Vector2)]) -> (MathFigure, Vec<String>) {
    let mut fig = MathFigure::new();
    let warnings = apply_geometry_to_figure(&mut fig, geom, existing);
    (fig, warnings)
}

/// Whether the reply asks to delete anything.
fn wants_removal(geom: &GeometryJson) -> bool {
    geom.remove_all
        || !geom.remove_points.is_empty()
        || !geom.remove_lines.is_empty()
        || !geom.remove_circles.is_empty()
}

/// Delete the points (with their lines and circles), lines (by end point names) and circles
/// (by center name) the reply lists. Returns whether anything was deleted.
fn remove_elements(fig: &mut MathFigure, geom: &GeometryJson) -> bool {
    fn key(name: &str) -> String {
        name.trim().to_uppercase()
    }
    let id_of = |fig: &MathFigure, name: &str| {
        fig.points
            .values()
            .find(|p| p.label.as_deref().map(key) == Some(key(name)))
            .map(|p| p.id)
    };
    let before = (fig.points.len(), fig.lines.len(), fig.circles.len());
    for rm in &geom.remove_lines {
        if let (Some(p1), Some(p2)) = (id_of(fig, &rm.p1), id_of(fig, &rm.p2)) {
            fig.lines
                .retain(|_, l| !((l.p1 == p1 && l.p2 == p2) || (l.p1 == p2 && l.p2 == p1)));
        }
    }
    for rm in &geom.remove_circles {
        if let Some(c) = id_of(fig, rm) {
            fig.circles.retain(|_, x| x.center != c);
        }
    }
    for rm in &geom.remove_points {
        if let Some(pid) = id_of(fig, rm) {
            fig.points.remove(&pid);
            fig.lines.retain(|_, l| l.p1 != pid && l.p2 != pid);
            fig.circles
                .retain(|_, c| c.center != pid && c.through_point != Some(pid));
            fig.right_angles
                .retain(|m| m.vertex != pid && m.arm1 != pid && m.arm2 != pid);
        }
    }
    // Unnamed helper points (intersections, line ends) left without any line or circle go too
    let used: std::collections::HashSet<PointId> = fig
        .lines
        .values()
        .flat_map(|l| [l.p1, l.p2])
        .chain(fig.circles.values().flat_map(|c| [Some(c.center), c.through_point]).flatten())
        .collect();
    if (fig.lines.len(), fig.circles.len()) != (before.1, before.2) || fig.points.len() != before.0
    {
        fig.points
            .retain(|id, p| p.label.is_some() || used.contains(id));
        fig.right_angles.retain(|m| {
            [m.vertex, m.arm1, m.arm2]
                .iter()
                .all(|id| fig.points.contains_key(id))
        });
    }
    (fig.points.len(), fig.lines.len(), fig.circles.len()) != before
}

/// Apply `GeometryJson` operations directly onto an existing (or new) `MathFigure`.
/// This allows AI to edit a user's clicked/selected figure in-place (moving points, resizing circles,
/// adding constructions/lines/circles, changing styles, renaming, or removing elements).
fn apply_geometry_to_figure(
    fig: &mut MathFigure,
    geom: &GeometryJson,
    existing: &[(String, Vector2)],
) -> Vec<String> {
    fn key(name: &str) -> String {
        name.trim().to_uppercase()
    }
    fn resolve(
        fig: &mut MathFigure,
        ids: &mut HashMap<String, PointId>,
        existing: &[(String, Vector2)],
        name: &str,
    ) -> Option<PointId> {
        if let Some(&id) = ids.get(&key(name)) {
            return Some(id);
        }
        if let Some(id) = fig.find_point_by_label(name) {
            ids.insert(key(name), id);
            return Some(id);
        }
        let (label, pos) = existing.iter().find(|(l, _)| key(l) == key(name))?;
        let id = fig.add_point(*pos, Some(label.clone()));
        ids.insert(key(name), id);
        Some(id)
    }

    /// Add or update one circle; false when its points do not exist (yet).
    fn add_circle(
        fig: &mut MathFigure,
        ids: &mut HashMap<String, PointId>,
        existing: &[(String, Vector2)],
        c: &JsonCircle,
    ) -> bool {
        let Some(o) = resolve(fig, ids, existing, &c.center) else {
            return false;
        };
        let through = match c.through.as_deref() {
            Some(t) => match resolve(fig, ids, existing, t) {
                Some(t) => Some(t),
                None => return false,
            },
            None => None,
        };
        let Some(r) = through
            .map(|t| (fig.points[&t].pos - fig.points[&o].pos).length())
            .or(c.radius)
            .filter(|r| *r > 0.0)
        else {
            return false;
        };
        // If there is already a circle with center `o` in the figure and an explicit radius/style was given,
        // update it in-place when `c.radius` is specified and only 1 circle is centered at `o`.
        let existing_at_o: Vec<_> = fig
            .circles
            .values()
            .filter(|x| x.center == o)
            .map(|x| x.id)
            .collect();
        if existing_at_o.len() == 1 && through.is_none() && c.radius.is_some() {
            let cid = existing_at_o[0];
            let through_pt = fig.circles.get(&cid).and_then(|circ| circ.through_point);
            let center_pos = fig.points[&o].pos;
            if let Some(circ) = fig.circles.get_mut(&cid) {
                circ.radius = r;
                if let Some(style) = line_style(c.style.as_deref()) {
                    circ.style = style;
                }
            }
            if let Some(tp_id) = through_pt
                && let Some(tp) = fig.points.get_mut(&tp_id)
            {
                let dir = (tp.pos - center_pos)
                    .try_normalize()
                    .unwrap_or_else(|| Vector2::new(1.0, 0.0));
                tp.pos = center_pos + dir * r;
            }
            return true;
        }

        // Already drawn, e.g. by a circumcircle construction
        if let Some(cid) = fig
            .circles
            .values()
            .find(|x| x.center == o && (x.radius - r).abs() < 0.5)
            .map(|x| x.id)
        {
            if let Some(style) = line_style(c.style.as_deref())
                && let Some(circ) = fig.circles.get_mut(&cid)
            {
                circ.style = style;
            }
        } else {
            let id = match through {
                Some(t) => fig.add_circle_through(o, t),
                None => fig.add_circle_radius(o, r),
            };
            if let Some(style) = line_style(c.style.as_deref()) {
                fig.circles.get_mut(&id).unwrap().style = style;
            }
        }
        true
    }

    if geom.replace {
        *fig = MathFigure::new();
    }

    // 1. Removals
    remove_elements(fig, geom);

    let mut ids = HashMap::new();
    for (&id, pt) in &fig.points {
        if let Some(ref lbl) = pt.label {
            ids.insert(key(lbl), id);
        }
    }
    let mut warnings = Vec::new();

    // 2. Renames
    for rn in &geom.rename {
        let kf = key(&rn.from);
        if let Some(pid) = ids.remove(&kf) {
            if let Some(pt) = fig.points.get_mut(&pid) {
                pt.label = Some(rn.to.trim().to_string());
            }
            ids.insert(key(&rn.to), pid);
        }
    }

    // 3. Points (update existing point position in-place or add new point)
    for p in &geom.points {
        let kp = key(&p.name);
        let new_pos = Vector2::new(p.x, p.y);
        if let Some(&existing_id) = ids.get(&kp) {
            if let Some(pt) = fig.points.get_mut(&existing_id) {
                pt.pos = new_pos;
            }
            // Update radii of any circles centered at or passing through this point
            let updates: Vec<_> = fig
                .circles
                .values()
                .filter_map(|c| {
                    let tp = c.through_point?;
                    if c.center == existing_id || tp == existing_id {
                        let c_pos = fig.points.get(&c.center)?.pos;
                        let t_pos = fig.points.get(&tp)?.pos;
                        Some((c.id, (t_pos - c_pos).length()))
                    } else {
                        None
                    }
                })
                .collect();
            for (cid, new_r) in updates {
                if let Some(c) = fig.circles.get_mut(&cid) {
                    c.radius = new_r;
                }
            }
        } else {
            let id = fig.add_point(new_pos, Some(p.name.trim().to_string()));
            ids.insert(kp, id);
        }
    }
    // Circles on known points come before constructions (tangents need their circle);
    // circles on construction points (e.g. a circumcenter) are retried after them.
    let pending_circles: Vec<&JsonCircle> = geom
        .circles
        .iter()
        .filter(|c| !add_circle(fig, &mut ids, existing, c))
        .collect();
    for c in &geom.constructions {
        let Some(pts) =
            c.of.iter()
                .map(|n| resolve(fig, &mut ids, existing, n))
                .collect::<Option<Vec<PointId>>>()
        else {
            warnings.push(format!("{}: thiếu điểm {}", c.op, c.of.join(",")));
            continue;
        };
        let name = c.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let created = match (c.op.to_lowercase().as_str(), pts.as_slice()) {
            ("midpoint", &[a, b]) => {
                let m = (fig.points[&a].pos + fig.points[&b].pos) * 0.5;
                Some(fig.add_point(m, Some(name.unwrap_or("M").to_string())))
            }
            ("altitude" | "perpendicular", &[a, b, c]) => {
                fig.add_altitude(a, b, c, name).map(|(p, _)| p)
            }
            ("median", &[a, b, c]) => fig.add_median(a, b, c, name).map(|(p, _)| p),
            ("bisector", &[a, b, c]) => fig.add_angle_bisector(a, b, c, name).map(|(p, _)| p),
            ("centroid", &[a, b, c]) => {
                fig.add_triangle_center(a, b, c, TriangleCenterType::Centroid, false)
            }
            ("orthocenter", &[a, b, c]) => {
                fig.add_triangle_center(a, b, c, TriangleCenterType::Orthocenter, false)
            }
            ("circumcenter" | "circumcircle", &[a, b, c]) => {
                fig.add_triangle_center(a, b, c, TriangleCenterType::Circumcenter, false)
            }
            ("incenter" | "incircle", &[a, b, c]) => {
                fig.add_triangle_center(a, b, c, TriangleCenterType::Incenter, false)
            }
            ("tangents", &[p, o]) => {
                match fig
                    .circles
                    .values()
                    .find(|circ| circ.center == o)
                    .map(|circ| circ.id)
                {
                    Some(circle) => {
                        let names = if c.names.is_empty() {
                            name.map(|n| n.split(',').map(str::to_string).collect())
                                .unwrap_or_default()
                        } else {
                            c.names.clone()
                        };
                        let lines = fig.add_tangents_from_point(p, circle);
                        for (line, n) in lines.iter().zip(&names) {
                            let t = fig.lines[line].p2;
                            fig.points.get_mut(&t).unwrap().label = Some(n.trim().to_string());
                            ids.insert(key(n), t);
                        }
                    }
                    None => warnings.push(format!("tangents: không có đường tròn tâm {}", c.of[1])),
                }
                None
            }
            _ => {
                warnings.push(format!(
                    "Không dựng được bước '{}' ({})",
                    c.op,
                    c.of.join(",")
                ));
                None
            }
        };
        if let (Some(id), Some(n)) = (created, name) {
            if let Some(p) = fig.points.get_mut(&id) {
                p.label = Some(n.to_string());
            }
            ids.insert(key(n), id);
        }
    }
    for c in pending_circles {
        if !add_circle(fig, &mut ids, existing, c) {
            warnings.push(format!("Đường tròn tâm {}: thiếu dữ liệu", c.center));
        }
    }
    for l in &geom.lines {
        match (
            resolve(fig, &mut ids, existing, &l.p1),
            resolve(fig, &mut ids, existing, &l.p2),
        ) {
            // Already drawn: update style if requested
            (Some(a), Some(b)) => {
                if let Some(existing_line_id) = fig
                    .lines
                    .values()
                    .find(|x| (x.p1, x.p2) == (a, b) || (x.p1, x.p2) == (b, a))
                    .map(|x| x.id)
                {
                    if let Some(style) = line_style(l.style.as_deref())
                        && let Some(line) = fig.lines.get_mut(&existing_line_id)
                    {
                        line.style = style;
                    }
                } else {
                    let id = fig.add_segment(a, b);
                    if let Some(style) = line_style(l.style.as_deref()) {
                        fig.lines.get_mut(&id).unwrap().style = style;
                    }
                }
            }
            _ => warnings.push(format!("Đoạn {}{}: thiếu điểm", l.p1, l.p2)),
        }
    }
    for ra in &geom.right_angles {
        let pts = [&ra.vertex, &ra.arm1, &ra.arm2].map(|n| resolve(fig, &mut ids, existing, n));
        if let [Some(v), Some(a1), Some(a2)] = pts {
            fig.add_right_angle(v, a1, a2, 8.0);
        }
    }
    fig.auto_detect_intersections();
    warnings
}

/// One request to the API with curl (bundled with Windows 10+ and Linux): POST `body` or GET.
/// The API key goes through stdin so it never shows up in the process list.
fn call_api(
    cfg: &AiConfig,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let url = format!("{}/{path}", cfg.base_url.trim().trim_end_matches('/'));
    let body_path = std::env::temp_dir().join(format!("pentapen-ai-{}.json", std::process::id()));
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--max-time", "180", &url])
        .args(["-H", "Content-Type: application/json", "-H", "@-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(body) = body {
        std::fs::write(&body_path, body.to_string()).map_err(|e| e.to_string())?;
        cmd.args(["-X", "POST", "--data-binary"])
            .arg(format!("@{}", body_path.display()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    let result = (|| {
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Không chạy được curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let auth = if cfg.api_key.trim().is_empty() {
                String::new()
            } else {
                format!("Authorization: Bearer {}\n", cfg.api_key.trim())
            };
            stdin
                .write_all(auth.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        child.wait_with_output().map_err(|e| e.to_string())
    })();
    if body.is_some() {
        let _ = std::fs::remove_file(&body_path);
    }
    let out = result?;
    if !out.status.success() {
        return Err(format!(
            "Không kết nối được {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let text = String::from_utf8_lossy(&out.stdout);
        format!(
            "API trả về dữ liệu lạ: {}",
            text.chars().take(300).collect::<String>()
        )
    })?;
    // Gemini sometimes wraps the error in a list
    if let Some(msg) = ["/error/message", "/0/error/message"]
        .iter()
        .find_map(|p| v.pointer(p).and_then(|m| m.as_str()))
    {
        return Err(format!("API báo lỗi: {msg}"));
    }
    Ok(v)
}

/// One chat completion: the model's reply text.
fn chat_completion(cfg: &AiConfig, user: &str) -> Result<String, String> {
    let mut body = serde_json::json!({
        "model": cfg.model.trim(),
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user},
        ],
    });
    // JSON mode where the service is known to support it (custom servers may reject the field)
    if matches!(PROVIDERS[provider_index(cfg)].id, "gemini" | "openai") {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }
    call_api(cfg, "chat/completions", Some(&body))?
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .ok_or_else(|| "API không trả về nội dung (choices[0].message.content).".to_string())
}

/// The models the key can use (checks the key and the URL at the same time).
fn list_models(cfg: &AiConfig) -> Result<Vec<String>, String> {
    let v = call_api(cfg, "models", None)?;
    let mut models: Vec<String> = v
        .pointer("/data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "API không trả về danh sách model.".to_string())?
        .iter()
        .filter_map(|m| m.get("id")?.as_str())
        // Gemini lists "models/gemini-2.5-flash" but expects "gemini-2.5-flash"
        .map(|id| id.trim_start_matches("models/").to_string())
        .collect();
    models.sort();
    Ok(models)
}

mod imp {
    use super::*;

    #[derive(Default, Debug, CompositeTemplate)]
    #[template(resource = "/com/github/flxzt/rnote/ui/aimenu.ui")]
    pub(crate) struct RnAiMenu {
        #[template_child]
        pub(crate) panel: TemplateChild<gtk4::Box>,
        #[template_child]
        pub(crate) header: TemplateChild<gtk4::Box>,
        #[template_child]
        pub(crate) close_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) bubble: TemplateChild<Button>,
        #[template_child]
        pub(crate) provider_dropdown: TemplateChild<gtk4::DropDown>,
        #[template_child]
        pub(crate) key_hint_label: TemplateChild<Label>,
        #[template_child]
        pub(crate) model_menubutton: TemplateChild<gtk4::MenuButton>,
        #[template_child]
        pub(crate) animate_check: TemplateChild<gtk4::CheckButton>,
        #[template_child]
        pub(crate) test_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) base_url_entry: TemplateChild<Entry>,
        #[template_child]
        pub(crate) model_entry: TemplateChild<Entry>,
        #[template_child]
        pub(crate) api_key_entry: TemplateChild<PasswordEntry>,
        #[template_child]
        pub(crate) save_key_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) center_status_label: TemplateChild<Label>,
        #[template_child]
        pub(crate) clear_center_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) pick_center_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) center_current_view_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) chat_scroller: TemplateChild<ScrolledWindow>,
        #[template_child]
        pub(crate) chat_box: TemplateChild<gtk4::Box>,
        #[template_child]
        pub(crate) prompt_view: TemplateChild<TextView>,
        #[template_child]
        pub(crate) pick_point_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) send_button: TemplateChild<Button>,
        pub(crate) appwindow: RefCell<Option<glib::WeakRef<RnAppWindow>>>,
        /// Bumped by every point pick, so a stale timeout does not cancel a newer pick
        pub(crate) pick_generation: Cell<u32>,
        /// The ring that follows the finger while a point is being picked
        pub(crate) pick_marker: RefCell<Option<gtk4::Box>>,
        /// The drawing being played (see `play_drawing`), finished at once when the user acts
        pub(crate) drawing:
            Rc<RefCell<Option<(crate::RnCanvas, rnote_engine::engine::DrawAnimation)>>>,
        /// Margins (end, bottom) and pointer position in the parent when a drag started
        pub(crate) drag_origin: Cell<(i32, i32, f64, f64)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RnAiMenu {
        const NAME: &'static str = "RnAiMenu";
        type Type = super::RnAiMenu;
        type ParentType = Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RnAiMenu {
        fn constructed(&self) {
            self.parent_constructed();
            self.setup_drag(&self.bubble.get().upcast());
            self.setup_drag(&self.header.get().upcast());
            let obj = self.obj();
            let w = obj.downgrade();
            self.bubble.get().connect_clicked(move |_| {
                if let Some(o) = w.upgrade() {
                    if o.imp().panel.is_visible() {
                        o.close_menu();
                    } else {
                        o.open_menu();
                    }
                }
            });
            let w = obj.downgrade();
            self.close_button.get().connect_clicked(move |_| {
                if let Some(o) = w.upgrade() {
                    o.close_menu();
                }
            });
        }

        fn dispose(&self) {
            self.dispose_template();
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for RnAiMenu {}

    impl RnAiMenu {
        /// Drag the assistant around the board (assistive-touch style) by its bubble or panel header.
        /// A short press still clicks: the drag only claims the gesture after the pointer moved a few pixels.
        /// Positions are measured in the parent: the widget moves under the pointer, so its own
        /// coordinates would feed the movement back and make it jitter in place.
        fn setup_drag(&self, handle: &Widget) {
            let drag = gtk4::GestureDrag::new();
            drag.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let claimed = std::rc::Rc::new(Cell::new(false));
            let pointer_in_parent =
                |o: &super::RnAiMenu, g: &gtk4::GestureDrag, dx: f64, dy: f64| {
                    let (x, y) = g.start_point()?;
                    let handle = g.widget()?;
                    let parent = o.parent()?;
                    let p = gtk4::graphene::Point::new((x + dx) as f32, (y + dy) as f32);
                    handle
                        .compute_point(&parent, &p)
                        .map(|p| (p.x() as f64, p.y() as f64))
                };
            let w = self.obj().downgrade();
            let c = claimed.clone();
            drag.connect_drag_begin(move |g, _, _| {
                c.set(false);
                if let Some(o) = w.upgrade()
                    && let Some((px, py)) = pointer_in_parent(&o, g, 0.0, 0.0)
                {
                    o.imp()
                        .drag_origin
                        .set((o.margin_end(), o.margin_bottom(), px, py));
                }
            });
            let w = self.obj().downgrade();
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(o) = w.upgrade() else { return };
                if !claimed.get() {
                    if dx.hypot(dy) < 6.0 {
                        return;
                    }
                    claimed.set(true);
                    g.set_state(gtk4::EventSequenceState::Claimed);
                }
                let Some((px, py)) = pointer_in_parent(&o, g, dx, dy) else {
                    return;
                };
                let (end, bottom, x0, y0) = o.imp().drag_origin.get();
                o.set_margins_clamped(end - (px - x0) as i32, bottom - (py - y0) as i32);
            });
            handle.add_controller(drag);
        }
    }
}

glib::wrapper! {
    pub(crate) struct RnAiMenu(ObjectSubclass<imp::RnAiMenu>)
        @extends Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl Default for RnAiMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl RnAiMenu {
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    pub(crate) fn open_menu(&self) {
        let imp = self.imp();
        imp.panel.set_visible(true);
        self.update_center_status();
        imp.prompt_view.grab_focus();
        // The panel makes the widget bigger: keep all of it inside the window once it is laid out
        let w = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(o) = w.upgrade() {
                o.set_margins_clamped(o.margin_end(), o.margin_bottom());
            }
        });
    }

    /// Move the assistant, keeping it fully inside its parent.
    fn set_margins_clamped(&self, end: i32, bottom: i32) {
        let (pw, ph) = self
            .parent()
            .map(|p| (p.width(), p.height()))
            .unwrap_or((i32::MAX, i32::MAX));
        let (_, w, _, _) = self.measure(gtk4::Orientation::Horizontal, -1);
        let (_, h, _, _) = self.measure(gtk4::Orientation::Vertical, w);
        self.set_margin_end(end.clamp(0, (pw - w).max(0)));
        self.set_margin_bottom(bottom.clamp(0, (ph - h).max(0)));
    }

    /// The board was tapped while the pick button waited: put the nearest point's name
    /// (or the coordinates) into the prompt and reopen the chat.
    pub(crate) fn point_picked(&self, pos: Vector2) {
        let imp = self.imp();
        imp.pick_generation
            .set(imp.pick_generation.get().wrapping_add(1));
        self.hide_pick_marker();
        let Some(canvas) = self.appwindow().and_then(|w| w.active_tab_canvas()) else {
            return;
        };
        let reach = 30.0 / canvas.engine_ref().camera.total_zoom().max(0.01);
        let nearest = canvas
            .engine_ref()
            .named_figure_points()
            .into_iter()
            .map(|(name, p)| ((p - pos).length(), name))
            .filter(|(d, _)| *d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        // Nothing is added to the board: the prompt gets the point's name or its position
        let text = match nearest {
            Some((_, name)) => format!("điểm {name} "),
            None => format!("tại toạ độ ({:.0}, {:.0}) ", pos.x, pos.y),
        };
        self.flash_marker(&canvas, pos);
        imp.prompt_view.buffer().insert_at_cursor(&text);
        self.open_menu();
    }

    /// A marker ring at the board position `pos`, placed in the toolbar overlay.
    fn marker_at(&self, canvas: &crate::RnCanvas, pos: Vector2) -> Option<gtk4::Box> {
        const SIZE: i32 = 30;
        let appwindow = self.appwindow()?;
        let overlay = appwindow.overlays().imp().toolbar_overlay.get();
        let p = canvas.engine_ref().camera.transform().transform_point2(pos);
        let p = canvas.compute_point(
            &overlay,
            &gtk4::graphene::Point::new(p.x as f32, p.y as f32),
        )?;
        let marker = self.imp().pick_marker.borrow().clone().unwrap_or_else(|| {
            gtk4::Box::builder()
                .width_request(SIZE)
                .height_request(SIZE)
                .halign(gtk4::Align::Start)
                .valign(gtk4::Align::Start)
                .can_target(false)
                .css_classes(["pick_marker"])
                .build()
        });
        marker.set_margin_start(p.x() as i32 - SIZE / 2);
        marker.set_margin_top(p.y() as i32 - SIZE / 2);
        if marker.parent().is_none() {
            overlay.add_overlay(&marker);
        }
        Some(marker)
    }

    /// While picking: the ring follows the finger or pen, the point is taken where it is let go.
    pub(crate) fn move_pick_marker(&self, pos: Vector2) {
        let Some(canvas) = self.appwindow().and_then(|w| w.active_tab_canvas()) else {
            return;
        };
        // Pressing keeps the pick alive, however long the finger takes to get there
        self.imp()
            .pick_generation
            .set(self.imp().pick_generation.get().wrapping_add(1));
        let marker = self.marker_at(&canvas, pos);
        self.imp().pick_marker.replace(marker);
    }

    pub(crate) fn hide_pick_marker(&self) {
        if let Some(marker) = self.imp().pick_marker.take()
            && let Some(appwindow) = self.appwindow()
        {
            appwindow
                .overlays()
                .imp()
                .toolbar_overlay
                .get()
                .remove_overlay(&marker);
        }
    }

    /// A ring that stays for a moment where the point was picked.
    fn flash_marker(&self, canvas: &crate::RnCanvas, pos: Vector2) {
        let Some(marker) = self.marker_at(canvas, pos) else {
            return;
        };
        let Some(appwindow) = self.appwindow() else {
            return;
        };
        let overlay = appwindow.overlays().imp().toolbar_overlay.get();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            overlay.remove_overlay(&marker);
        });
    }

    /// Wait for one press on the board (see `point_picked`), with a red cursor: the ring follows
    /// the finger until it is let go. Gives up after 5 s without a press, picking nothing.
    fn start_point_pick(&self) {
        const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
        let Some(appwindow) = self.appwindow() else {
            return;
        };
        let Some(canvas) = appwindow.active_tab_canvas() else {
            return;
        };
        let imp = self.imp();
        let generation = imp.pick_generation.get().wrapping_add(1);
        imp.pick_generation.set(generation);
        canvas.set_picking_ai_point(true);
        self.close_menu();
        appwindow.overlays().dispatch_toast_text(
            "Bấm vào vị trí bất kỳ trên bảng trong 5 giây (giữ và kéo để chỉnh, Esc để huỷ).",
            Some(WAIT),
        );
        let this = self.clone();
        glib::timeout_add_local_once(WAIT, move || {
            if this.imp().pick_generation.get() != generation || !canvas.is_picking_ai_point() {
                return;
            }
            canvas.set_picking_ai_point(false);
            this.hide_pick_marker();
            this.append_ai_message("Hết 5 giây, chưa chọn điểm nào.");
            this.open_menu();
        });
    }

    pub(crate) fn close_menu(&self) {
        self.imp().panel.set_visible(false);
    }

    fn appwindow(&self) -> Option<RnAppWindow> {
        self.imp()
            .appwindow
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
    }

    pub(crate) fn update_center_status(&self) {
        let imp = self.imp();
        if let Some(canvas) = self.appwindow().and_then(|w| w.active_tab_canvas()) {
            let engine = canvas.engine_ref();
            if let Some((_, summary)) = engine.selected_shape_summary() {
                imp.center_status_label.set_markup(&format!(
                    "Đang chọn: <b>{}</b> — AI sẽ chỉnh sửa trực tiếp trên hình này",
                    glib::markup_escape_text(&summary),
                ));
                return;
            }
            if let Some((pos, lbl)) = engine.last_center_point() {
                let name = lbl.unwrap_or_else(|| "O".to_string());
                imp.center_status_label.set_markup(&format!(
                    "Tâm đang chọn: <b>{}</b> ({:.0}, {:.0})",
                    glib::markup_escape_text(&name),
                    pos.x,
                    pos.y
                ));
                return;
            }
        }
        imp.center_status_label
            .set_markup("Chưa chọn hình hoặc tâm (Nhấn vào hình trên bảng để sửa trực tiếp)");
    }

    fn config_from_ui(&self) -> AiConfig {
        let imp = self.imp();
        AiConfig {
            provider: PROVIDERS
                [(imp.provider_dropdown.selected() as usize).min(PROVIDERS.len() - 1)]
            .id
            .to_string(),
            base_url: imp.base_url_entry.text().trim().to_string(),
            model: imp.model_entry.text().trim().to_string(),
            api_key: imp.api_key_entry.text().trim().to_string(),
            instant_draw: !imp.animate_check.is_active(),
        }
    }

    /// Models offered under the model field's arrow.
    fn set_model_choices(&self, models: &[String]) {
        let imp = self.imp();
        let list = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let scroller = ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .max_content_height(320)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .build();
        let popover = gtk4::Popover::builder().child(&scroller).build();
        for m in models {
            let b = Button::with_label(m);
            b.add_css_class("flat");
            let (entry, popover, m) = (imp.model_entry.get(), popover.clone(), m.clone());
            b.connect_clicked(move |_| {
                entry.set_text(&m);
                popover.popdown();
            });
            list.append(&b);
        }
        imp.model_menubutton.set_popover(Some(&popover));
        imp.model_menubutton.set_sensitive(!models.is_empty());
    }

    /// Provider presets fill the URL and a model; "Thử kết nối" checks the key and lists the models.
    fn setup_providers(&self, initial: usize) {
        let imp = self.imp();
        let show = |this: &Self, i: usize, fill: bool| {
            let imp = this.imp();
            let p = &PROVIDERS[i];
            imp.key_hint_label.set_markup(p.key_hint);
            imp.base_url_entry.set_editable(p.id == "custom");
            if fill && !p.base_url.is_empty() {
                imp.base_url_entry.set_text(p.base_url);
                let model = imp.model_entry.text();
                if !p.models.iter().any(|m| *m == model.as_str()) {
                    imp.model_entry.set_text(p.models[0]);
                }
            }
            let models: Vec<String> = p.models.iter().map(|m| m.to_string()).collect();
            this.set_model_choices(&models);
        };
        imp.provider_dropdown.set_selected(initial as u32);
        // An empty config gets the first preset (Gemini) filled in
        show(self, initial, imp.base_url_entry.text().is_empty());
        let this = self.clone();
        imp.provider_dropdown.connect_selected_notify(move |d| {
            show(
                &this,
                (d.selected() as usize).min(PROVIDERS.len() - 1),
                true,
            );
        });

        let this = self.clone();
        imp.test_button.connect_clicked(move |b| {
            let cfg = this.config_from_ui();
            if cfg.base_url.is_empty() {
                this.append_ai_message("Chưa có Base URL.");
                return;
            }
            b.set_sensitive(false);
            let (this, b) = (this.clone(), b.clone());
            glib::spawn_future_local(async move {
                let model = cfg.model.clone();
                let result = blocking::unblock(move || list_models(&cfg)).await;
                b.set_sensitive(true);
                match result {
                    Ok(models) => {
                        let found = models.iter().any(|m| *m == model);
                        this.append_ai_message(&format!(
                            "Kết nối được. Key dùng được <b>{}</b> model{}.",
                            models.len(),
                            if model.is_empty() || found {
                                String::new()
                            } else {
                                format!(
                                    " <i>Không thấy model \"{}\" trong danh sách, chọn lại ở mũi tên cạnh ô Model.</i>",
                                    glib::markup_escape_text(&model)
                                )
                            }
                        ));
                        if !models.is_empty() {
                            this.set_model_choices(&models);
                        }
                    }
                    Err(e) => this.append_ai_message(&glib::markup_escape_text(&e)),
                }
            });
        });
    }

    pub(crate) fn init(&self, appwindow: &RnAppWindow) {
        let imp = self.imp();
        imp.appwindow.borrow_mut().replace(appwindow.downgrade());

        let config = AiConfig::load();
        imp.base_url_entry.set_text(&config.base_url);
        imp.model_entry.set_text(&config.model);
        imp.api_key_entry.set_text(&config.api_key);
        imp.animate_check.set_active(!config.instant_draw);
        self.setup_providers(provider_index(&config));

        self.append_ai_message(
            "<b>Trợ lý Hình học AI</b>\n\
            • Nhấn chọn một hình bất kỳ trên bảng rồi nhập lệnh (vd: <i>kẻ đường cao AH</i>, <i>vẽ đường tròn nội tiếp</i>, <i>đổi cạnh BC thành nét đứt</i>, <i>tăng bán kính lên 120</i>) để AI chỉnh sửa trực tiếp trên hình đó.\n\
            • Hoặc nhập lệnh vẽ hình mới (vd: <i>vẽ tam giác ABC vuông tại A, AB = 4cm, AC = 3cm</i>).\n\
            Mở mục <b>Cấu hình API</b> phía trên để thiết lập kết nối.",
        );

        let this = self.clone();
        imp.save_key_button.connect_clicked(move |_| {
            let cfg = this.config_from_ui();
            match cfg.save() {
                Ok(()) => this.append_ai_message(&format!(
                    "Đã lưu cấu hình: <b>{}</b> · mô hình <b>{}</b>",
                    glib::markup_escape_text(&cfg.base_url),
                    glib::markup_escape_text(&cfg.model)
                )),
                Err(e) => this.append_ai_message(&format!(
                    "Không lưu được cấu hình: {}",
                    glib::markup_escape_text(&e.to_string())
                )),
            }
        });

        // Takes effect right away, without "Lưu"
        imp.animate_check.connect_toggled(|c| {
            let mut cfg = AiConfig::load();
            cfg.instant_draw = !c.is_active();
            let _ = cfg.save();
        });

        self.update_center_status();

        let this = self.clone();
        imp.pick_center_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                canvas.set_picking_center(true);
                this.close_menu();
                appwindow.overlays().dispatch_toast_text(
                    "Hãy nhấp chuột hoặc chạm bút lên bảng vẽ để đặt điểm tâm (Esc để huỷ)",
                    None,
                );
            }
        });

        let this = self.clone();
        imp.center_current_view_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                let pos = canvas.engine_ref().camera.viewport_center();
                let mut fig = MathFigure::new();
                fig.add_point(pos, Some("O".to_string()));
                canvas
                    .engine_mut()
                    .set_last_center_point(pos, Some("O".to_string()));
                let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);
                this.update_center_status();
            }
        });

        let this = self.clone();
        imp.clear_center_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                let flags = canvas.engine_mut().clear_shape_selection();
                canvas.engine_mut().last_center_point = None;
                appwindow.handle_widget_flags(flags, &canvas);
                this.update_center_status();
            }
        });

        let this = self.clone();
        imp.send_button
            .connect_clicked(move |_| this.send_prompt_entry());
        let this = self.clone();
        imp.pick_point_button
            .connect_clicked(move |_| this.start_point_pick());

        // Enter sends, Shift+Enter inserts a line break
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let this = self.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                this.send_prompt_entry();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        imp.prompt_view.add_controller(keys);
    }

    fn send_prompt_entry(&self) {
        let buffer = self.imp().prompt_view.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        if !text.trim().is_empty() {
            buffer.set_text("");
            self.handle_user_prompt(text.trim());
        }
    }

    fn append_user_message(&self, text: &str) {
        let label = Label::builder()
            .label(text)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(true)
            .halign(gtk4::Align::End)
            .margin_start(40)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        label.add_css_class("card");
        self.imp().chat_box.append(&label);
        self.scroll_to_bottom();
    }

    pub(crate) fn append_ai_message(&self, markup: &str) {
        let label = Label::builder()
            .use_markup(true)
            .label(markup)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(true)
            .halign(gtk4::Align::Start)
            .margin_start(4)
            .margin_end(30)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        label.add_css_class("card");
        self.imp().chat_box.append(&label);
        self.scroll_to_bottom();
    }

    fn scroll_to_bottom(&self) {
        let adj = self.imp().chat_scroller.vadjustment();
        glib::idle_add_local_once(move || {
            adj.set_value(adj.upper() - adj.page_size());
        });
    }

    /// Axes, graphs and variation tables of a reply. Point coordinates are turned from axis
    /// units into canvas pixels so the figure lands on the drawn plane. Returns whether
    /// anything was drawn, and the formulas that could not be read.
    fn draw_graphs(
        &self,
        geom: &mut GeometryJson,
        anchor: Vector2,
        drawn: &mut Vec<StrokeKey>,
    ) -> (bool, Vec<String>) {
        let mut warnings = Vec::new();
        let Some(appwindow) = self.appwindow() else {
            return (false, warnings);
        };
        let Some(canvas) = appwindow.active_tab_canvas() else {
            return (false, warnings);
        };
        let mut parse = |g: &JsonGraph| match Function::parse(&g.f) {
            Ok(f) => Some(f),
            Err(e) => {
                warnings.push(format!("{}: {e}", g.f));
                None
            }
        };
        let graphs: Vec<(Function, bool)> = geom
            .graphs
            .iter()
            .filter_map(|g| parse(g).map(|f| (f, g.table)))
            .collect();
        let only_tables: Vec<Function> = geom.tables.iter().filter_map(parse).collect();
        let frame = graph_frame(geom, anchor);
        let mut drew = false;
        // Tables go right of the plane (or at the anchor without one), one under the other
        let mut table_at = anchor;
        if let Some(frame) = frame {
            let fns: Vec<Function> = graphs.iter().map(|(f, _)| f.clone()).collect();
            let (flags, keys) = canvas.engine_mut().insert_graph_frame(frame, &fns);
            appwindow.handle_widget_flags(flags, &canvas);
            drawn.extend(keys);
            for p in &mut geom.points {
                let c = frame.to_canvas(p.x, p.y);
                (p.x, p.y) = (c.x, c.y);
            }
            table_at = frame.to_canvas(frame.x_range.1, frame.y_range.1) + Vector2::new(60.0, 0.0);
            drew = true;
        }
        let tables = graphs
            .iter()
            .filter(|(_, t)| *t)
            .map(|(f, _)| f)
            .chain(only_tables.iter());
        for f in tables {
            match canvas.engine_mut().insert_variation_table(f, table_at) {
                Ok((flags, keys)) => {
                    appwindow.handle_widget_flags(flags, &canvas);
                    drawn.extend(keys);
                    table_at.y += 240.0;
                    drew = true;
                }
                Err(e) => warnings.push(format!("{}: {e}", f.text)),
            }
        }
        (drew, warnings)
    }

    /// Replay what the AI just drew (already recorded for undo) stroke by stroke, unless
    /// "Vẽ từng nét" is off.
    fn play_drawing(&self, keys: Vec<StrokeKey>, base: Option<MathFigure>) {
        let Some(appwindow) = self.appwindow() else {
            return;
        };
        let Some(canvas) = appwindow.active_tab_canvas() else {
            return;
        };
        if keys.is_empty() || AiConfig::load().instant_draw {
            return;
        }
        self.finish_drawing();
        let (anim, flags) = canvas.engine_mut().draw_animation(&keys, base.as_ref());
        appwindow.handle_widget_flags(flags, &canvas);
        let slot = self.imp().drawing.clone();
        slot.replace(Some((canvas.clone(), anim)));
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(std::time::Duration::from_millis(40)).await;
                let mut slot = slot.borrow_mut();
                // Finished early (the user started doing something), or a newer drawing took over
                let Some((c, anim)) = slot.as_mut() else { break };
                if *c != canvas {
                    break;
                }
                let (more, flags) = canvas.engine_mut().draw_animation_step(anim);
                if !more {
                    *slot = None;
                }
                drop(slot);
                appwindow.handle_widget_flags(flags, &canvas);
                if !more {
                    break;
                }
            }
        });
    }

    /// Show the rest of a drawing being played at once, so nothing the user does (drawing,
    /// moving a point, copying, saving) sees a half drawn figure.
    pub(crate) fn finish_drawing(&self) {
        let Some((canvas, mut anim)) = self.imp().drawing.take() else {
            return;
        };
        let flags = canvas.engine_mut().draw_animation_finish(&mut anim);
        if let Some(appwindow) = self.appwindow() {
            appwindow.handle_widget_flags(flags, &canvas);
        }
    }

    fn handle_user_prompt(&self, prompt: &str) {
        self.append_user_message(prompt);

        let Some(canvas) = self.appwindow().and_then(|w| w.active_tab_canvas()) else {
            self.append_ai_message("Không tìm thấy trang vẽ đang mở.");
            return;
        };
        let cfg = AiConfig::load();
        if !cfg.is_complete() {
            self.append_ai_message(
                "Chưa cấu hình API. Mở mục <b>Cấu hình API</b>, nhập Base URL, Model, API key rồi bấm <b>Lưu</b>.",
            );
            return;
        }

        // Check if the user has clicked/selected a shape on the canvas to edit in-place
        let (selected_target, anchor, existing, (lines, circles)) = {
            let engine = canvas.engine_ref();
            let sel = engine.selected_shape_key().and_then(|key| {
                let (origin, ctx) = engine.selected_shape_ai_context(key)?;
                let fig = engine.stroke_as_math_figure(key)?;
                Some((key, origin, ctx, fig))
            });
            let anchor = sel
                .as_ref()
                .map(|(_, origin, _, _)| *origin)
                .or_else(|| engine.last_center_point().map(|(p, _)| p))
                .unwrap_or_else(|| engine.camera.viewport_center());
            (
                sel,
                anchor,
                engine.named_figure_points(),
                engine.named_figure_lines_and_circles(),
            )
        };
        let existing_desc = if existing.is_empty() {
            "none".to_string()
        } else {
            existing
                .iter()
                .map(|(n, p)| format!("{n}({:.0}, {:.0})", p.x, p.y))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let existing_desc = if lines.is_empty() && circles.is_empty() {
            existing_desc
        } else {
            format!(
                "{existing_desc}. Existing lines: {}. Existing circles: {}",
                if lines.is_empty() { "none".to_string() } else { lines.join(", ") },
                if circles.is_empty() { "none".to_string() } else { circles.join(", ") },
            )
        };
        let user = if let Some((_, _, ref shape_ctx, _)) = selected_target {
            format!(
                "Anchor: ({:.0}, {:.0}).\nSelected shape (modify this shape in-place): {shape_ctx}.\nExisting canvas points: {existing_desc}.\n\nRequest: {prompt}",
                anchor.x, anchor.y
            )
        } else {
            format!(
                "Anchor: ({:.0}, {:.0}).\nExisting points: {existing_desc}.\n\nRequest: {prompt}",
                anchor.x, anchor.y
            )
        };

        self.append_ai_message(&format!(
            "<b>{}</b> đang xử lý...",
            glib::markup_escape_text(&cfg.model)
        ));
        let this = self.clone();
        glib::spawn_future_local(async move {
            let reply = blocking::unblock(move || chat_completion(&cfg, &user)).await;
            let mut geom = match reply.and_then(|r| parse_geometry(&r)) {
                Ok(geom) => geom,
                Err(e) => {
                    this.append_ai_message(&glib::markup_escape_text(&e));
                    return;
                }
            };

            let explanation = geom.explanation.clone().unwrap_or_default();
            // "Delete everything": the page is cleared first, a selection no longer matters
            let mut removed = 0;
            let selected_target = if geom.remove_all {
                if let Some(appwindow) = this.appwindow()
                    && let Some(canvas) = appwindow.active_tab_canvas()
                {
                    this.finish_drawing();
                    let (n, flags) = canvas.engine_mut().remove_all_shapes();
                    appwindow.handle_widget_flags(flags, &canvas);
                    removed += n;
                }
                None
            } else {
                selected_target
            };
            // Everything the reply adds, replayed stroke by stroke at the end
            let mut drawn = Vec::new();
            let (drew_graph, graph_warnings) = this.draw_graphs(&mut geom, anchor, &mut drawn);

            // Case 1: Editing a clicked/selected shape in-place!
            if let Some((sel_key, _, _, mut target_fig)) = selected_target {
                let before = target_fig.clone();
                let warnings = apply_geometry_to_figure(&mut target_fig, &geom, &existing);
                if let Some(appwindow) = this.appwindow()
                    && let Some(canvas) = appwindow.active_tab_canvas()
                {
                    this.finish_drawing();
                    if target_fig.points.is_empty() {
                        // Everything of it was deleted
                        let flags = canvas.engine_mut().remove_stroke(sel_key);
                        appwindow.handle_widget_flags(flags, &canvas);
                        this.update_center_status();
                        this.play_drawing(drawn, None);
                    } else {
                        let flags = canvas
                            .engine_mut()
                            .update_shape_with_figure(sel_key, target_fig);
                        appwindow.handle_widget_flags(flags, &canvas);
                        this.update_center_status();
                        drawn.push(sel_key);
                        this.play_drawing(drawn, Some(before));
                    }
                }
                let mut msg = if explanation.is_empty() {
                    "Đã cập nhật hình đang chọn.".to_string()
                } else {
                    glib::markup_escape_text(&explanation).to_string()
                };
                if !warnings.is_empty() {
                    msg.push_str(&format!(
                        "\n<i>Bỏ qua: {}</i>",
                        glib::markup_escape_text(&warnings.join("; "))
                    ));
                }
                this.append_ai_message(&msg);
                return;
            }

            // Case 2: No shape selected -> restyle existing lines or insert a new figure
            let defined: std::collections::HashSet<String> = geom
                .points
                .iter()
                .map(|p| p.name.clone())
                .chain(geom.constructions.iter().flat_map(|c| {
                    c.name
                        .iter()
                        .flat_map(|n| n.split(','))
                        .map(str::to_string)
                        .chain(c.names.clone())
                }))
                .map(|n| n.trim().to_uppercase())
                .collect();
            let mut restyled = 0;
            // Deleting with nothing selected: on every figure on the page
            if !geom.remove_all
                && wants_removal(&geom)
                && let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                this.finish_drawing();
                let (n, flags) = canvas
                    .engine_mut()
                    .edit_all_figures(|fig| remove_elements(fig, &geom));
                appwindow.handle_widget_flags(flags, &canvas);
                removed += n;
            }
            if let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                geom.lines.retain(|l| {
                    if [&l.p1, &l.p2]
                        .iter()
                        .any(|n| defined.contains(&n.trim().to_uppercase()))
                    {
                        return true;
                    }
                    let (exists, flags) = canvas.engine_mut().restyle_figure_lines(
                        &l.p1,
                        &l.p2,
                        line_style(l.style.as_deref()),
                    );
                    appwindow.handle_widget_flags(flags, &canvas);
                    restyled += usize::from(exists && l.style.is_some());
                    !exists
                });
            }
            let (fig, warnings) = build_figure(&geom, &existing);
            if fig.points.is_empty() && !drew_graph && wants_removal(&geom) {
                let msg = if removed == 0 {
                    "Không tìm thấy phần cần xoá trên bảng (kiểm tra lại tên điểm)."
                } else if explanation.is_empty() {
                    "Đã xoá. Bấm Hoàn tác (Ctrl+Z) nếu muốn lấy lại."
                } else {
                    &explanation
                };
                this.append_ai_message(&glib::markup_escape_text(msg));
                return;
            }
            if fig.points.is_empty() && restyled > 0 {
                this.append_ai_message(&glib::markup_escape_text(&explanation));
                return;
            }
            if fig.points.is_empty() && drew_graph {
                this.play_drawing(drawn, None);
                let mut msg = glib::markup_escape_text(&explanation).to_string();
                if !graph_warnings.is_empty() {
                    msg.push_str(&format!(
                        "
<i>Bỏ qua: {}</i>",
                        glib::markup_escape_text(&graph_warnings.join("; "))
                    ));
                }
                this.append_ai_message(&msg);
                return;
            }
            if fig.points.is_empty() {
                this.append_ai_message(&glib::markup_escape_text(if explanation.is_empty() {
                    "AI không trả về hình nào để vẽ."
                } else {
                    &explanation
                }));
                return;
            }
            if let Some(appwindow) = this.appwindow()
                && let Some(canvas) = appwindow.active_tab_canvas()
            {
                let (flags, key) = canvas.engine_mut().insert_shape_key(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);
                drawn.push(key);
                this.play_drawing(drawn, None);
            }
            let mut msg = glib::markup_escape_text(&explanation).to_string();
            if !warnings.is_empty() {
                msg.push_str(&format!(
                    "\n<i>Bỏ qua: {}</i>",
                    glib::markup_escape_text(&warnings.join("; "))
                ));
            }
            this.append_ai_message(&msg);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletes_what_the_reply_lists() {
        let mut fig = MathFigure::new();
        let a = fig.add_point(Vector2::new(0.0, 0.0), Some("A".into()));
        let b = fig.add_point(Vector2::new(100.0, 0.0), Some("B".into()));
        let c = fig.add_point(Vector2::new(0.0, 100.0), Some("C".into()));
        let o = fig.add_point(Vector2::new(50.0, 50.0), Some("O".into()));
        fig.add_segment(a, b);
        fig.add_segment(b, c);
        fig.add_segment(c, a);
        fig.add_circle_through(o, a);

        let geom = parse_geometry(
            r#"{"remove_circles": ["o"], "remove_lines": [{"p1": "B", "p2": "A"}]}"#,
        )
        .unwrap();
        assert!(wants_removal(&geom));
        assert!(remove_elements(&mut fig, &geom));
        assert!(fig.circles.is_empty());
        assert_eq!(fig.lines.len(), 2);
        // Named points stay unless asked for
        assert_eq!(fig.points.len(), 4);

        // Removing C takes its lines along; asking again changes nothing
        let geom = parse_geometry(r#"{"remove_points": ["C"]}"#).unwrap();
        assert!(remove_elements(&mut fig, &geom));
        assert!(fig.lines.is_empty() && fig.find_point_by_label("C").is_none());
        assert!(!remove_elements(&mut fig, &geom));

        let all = parse_geometry(r#"{"explanation": "Đã xoá", "remove_all": true}"#).unwrap();
        assert!(all.remove_all && wants_removal(&all));
    }

    #[test]
    fn builds_constructions_from_reply() {
        let reply = r#"```json
        {"explanation": "Tam giác vuông tại A, đường cao AH",
         "points": [{"name": "A", "x": 0, "y": 0}, {"name": "C", "x": 0, "y": 120}],
         "lines": [{"p1": "A", "p2": "B"}, {"p1": "B", "p2": "C"}, {"p1": "C", "p2": "A"},
                   {"p1": "A", "p2": "H"}],
         "circles": [{"center": "O", "through": "A"}],
         "constructions": [{"op": "altitude", "of": ["A", "B", "C"], "name": "H"},
                           {"op": "midpoint", "of": ["B", "C"], "name": "M"},
                           {"op": "circumcircle", "of": ["A", "B", "C"], "name": "O"},
                           {"op": "spiral", "of": ["A"]}]}
        ```"#;
        let geom = parse_geometry(reply).unwrap();
        // B is not in the reply: it comes from the canvas
        let existing = vec![("B".to_string(), Vector2::new(160.0, 0.0))];
        let (fig, warnings) = build_figure(&geom, &existing);

        let by_name = |n: &str| {
            fig.points
                .values()
                .find(|p| p.label.as_deref() == Some(n))
                .map(|p| p.pos)
                .unwrap()
        };
        assert_eq!(by_name("M"), Vector2::new(80.0, 60.0));
        // Foot of the altitude from A onto BC (3-4-5 triangle scaled by 40): (57.6, 76.8)
        let h = by_name("H");
        assert!((h.x - 57.6).abs() < 1e-6 && (h.y - 76.8).abs() < 1e-6);
        // Lines/circles may name construction points; repeats of what a construction drew are skipped
        assert_eq!(fig.lines.len(), 4); // 3 sides + altitude (the explicit AH is the same segment)
        assert_eq!(fig.circles.len(), 1); // circumcircle, the explicit circle (O) through A repeats it
        assert_eq!(by_name("O"), Vector2::new(80.0, 60.0));
        assert_eq!(
            warnings,
            vec!["Không dựng được bước 'spiral' (A)".to_string()]
        );
        assert!(parse_geometry("no json here").is_err());
    }

    #[test]
    fn tangents_find_the_circle_and_name_their_points() {
        // Shape of a real reply: circle in "circles", tangents in "constructions", sides in "lines"
        let reply = r#"{"explanation": "OA = 2R",
         "points": [{"name": "A", "x": 500, "y": 200}, {"name": "O", "x": 500, "y": 431.458}],
         "circles": [{"center": "O", "radius": 115.729}],
         "constructions": [{"op": "tangents", "of": ["A", "O"], "names": ["B", "C1"]}],
         "lines": [{"p1": "A", "p2": "B"}, {"p1": "A", "p2": "C1"}]}"#;
        let (fig, warnings) = build_figure(&parse_geometry(reply).unwrap(), &[]);
        assert!(warnings.is_empty(), "{warnings:?}");

        let pos = |n: &str| {
            fig.points
                .values()
                .find(|p| p.label.as_deref() == Some(n))
                .map(|p| p.pos)
                .unwrap_or_else(|| panic!("missing point {n}"))
        };
        let (a, o) = (pos("A"), pos("O"));
        for t in [pos("B"), pos("C1")] {
            assert!(((t - o).length() - 115.729).abs() < 1e-6); // on the circle
            assert!((t - o).dot(t - a).abs() < 1e-6); // radius is perpendicular to the tangent
        }
        assert_eq!(fig.lines.len(), 2); // AB, AC1 are the tangent segments, not drawn twice
    }

    #[test]
    fn line_and_circle_styles() {
        assert_eq!(line_style(Some("dashed")), Some(LineStyle::Dashed));
        assert_eq!(line_style(Some("nét đứt")), Some(LineStyle::Dashed));
        assert_eq!(line_style(Some("dotted")), Some(LineStyle::Dotted));
        assert_eq!(line_style(Some("solid")), Some(LineStyle::Solid));
        assert_eq!(line_style(None), None);

        let reply = r#"{"points": [{"name": "A", "x": 0, "y": 0}, {"name": "B", "x": 100, "y": 0}],
         "lines": [{"p1": "A", "p2": "B", "style": "dash"}],
         "circles": [{"center": "A", "radius": 50, "style": "dashed"}]}"#;
        let (fig, _) = build_figure(&parse_geometry(reply).unwrap(), &[]);
        assert!(fig.lines.values().all(|l| l.style == LineStyle::Dashed));
        assert!(fig.circles.values().all(|c| c.style == LineStyle::Dashed));
    }

    #[test]
    fn edits_selected_figure_in_place() {
        let mut fig = MathFigure::new();
        fig.build_triangle(
            Vector2::new(0.0, 0.0),
            Vector2::new(160.0, 0.0),
            Vector2::new(0.0, 120.0),
            Some(["A", "B", "C"]),
        );

        let reply = r#"{"explanation": "Kẻ đường cao AH và đổi cạnh BC thành nét đứt",
         "constructions": [{"op": "altitude", "of": ["A", "B", "C"], "name": "H"}],
         "lines": [{"p1": "B", "p2": "C", "style": "dashed"}]}"#;
        let geom = parse_geometry(reply).unwrap();
        let warnings = apply_geometry_to_figure(&mut fig, &geom, &[]);
        assert!(warnings.is_empty());
        assert!(fig.find_point_by_label("H").is_some());
        let bc = fig
            .lines
            .values()
            .find(|l| {
                let b = fig.find_point_by_label("B").unwrap();
                let c = fig.find_point_by_label("C").unwrap();
                (l.p1 == b && l.p2 == c) || (l.p1 == c && l.p2 == b)
            })
            .unwrap();
        assert_eq!(bc.style, LineStyle::Dashed);
    }

    #[test]
    fn coordinate_replies_use_axis_units() {
        let reply = r#"{"explanation": "Đồ thị y = x^2 - 2x",
         "axes": {"x_min": -2, "x_max": 4, "unit": 50},
         "graphs": [{"f": "x^2-2*x", "table": true}],
         "points": [{"name": "A", "x": 1, "y": -1}]}"#;
        let geom = parse_geometry(reply).unwrap();
        let frame = graph_frame(&geom, Vector2::new(400.0, 300.0)).unwrap();
        assert_eq!(
            (frame.unit, frame.x_range, frame.y_range),
            (50.0, (-2.0, 4.0), (-5.0, 5.0))
        );
        // A(1; -1) is right of and below O on the canvas
        assert_eq!(frame.to_canvas(1.0, -1.0), Vector2::new(450.0, 350.0));
        assert!(geom.graphs[0].table);
        assert_eq!(Function::parse(&geom.graphs[0].f).unwrap().eval(3.0), 3.0);
        // A plain geometry reply draws no plane
        let plain = parse_geometry(r#"{"points": [{"name": "A", "x": 0, "y": 0}]}"#).unwrap();
        assert!(graph_frame(&plain, Vector2::new(0.0, 0.0)).is_none());
    }
}
