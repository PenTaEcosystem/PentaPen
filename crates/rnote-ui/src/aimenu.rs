// Imports
use crate::appwindow::RnAppWindow;
use gtk4::{
    Button, CompositeTemplate, Entry, Label, MenuButton, PasswordEntry, Popover, ScrolledWindow,
    Widget, glib, pango, prelude::*, subclass::prelude::*,
};
use p2d::math::Vector2;
use rnote_compose::Shape;
use rnote_compose::geometry::triangle::TriangleCenterType;
use rnote_compose::geometry::{LineStyle, MathFigure, PointId};
use std::cell::RefCell;
use std::collections::HashMap;

/// Connection to an OpenAI-compatible chat completions API (OpenAI, a proxy, a custom server).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct AiConfig {
    /// e.g. `https://api.openai.com/v1`; `/chat/completions` is appended.
    pub base_url: String,
    pub model: String,
    pub api_key: String,
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
 "remove_circles": ["O"]}
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
- "style" (lines and circles) is "solid", "dashed" or "dotted"; use dashed for auxiliary lines.
- To change an existing line (e.g. "make AB dashed"), list it in "lines" with the new style: lines between existing points are restyled, not redrawn.
- Intersections are added automatically."#;

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
    if s.contains("dash") || s.contains("đứt") {
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

    let mut ids = HashMap::new();
    for (&id, pt) in &fig.points {
        if let Some(ref lbl) = pt.label {
            ids.insert(key(lbl), id);
        }
    }
    let mut warnings = Vec::new();

    // 1. Removals
    for rm_line in &geom.remove_lines {
        let k1 = key(&rm_line.p1);
        let k2 = key(&rm_line.p2);
        if let (Some(&p1), Some(&p2)) = (ids.get(&k1), ids.get(&k2)) {
            fig.lines
                .retain(|_, l| !((l.p1 == p1 && l.p2 == p2) || (l.p1 == p2 && l.p2 == p1)));
        }
    }
    for rm_circ in &geom.remove_circles {
        let kc = key(rm_circ);
        if let Some(&c_id) = ids.get(&kc) {
            fig.circles.retain(|_, c| c.center != c_id);
        }
    }
    for rm_pt in &geom.remove_points {
        let kp = key(rm_pt);
        if let Some(pid) = ids.remove(&kp) {
            fig.points.remove(&pid);
            fig.lines.retain(|_, l| l.p1 != pid && l.p2 != pid);
            fig.circles
                .retain(|_, c| c.center != pid && c.through_point != Some(pid));
        }
    }

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

/// POST one chat completion with curl (bundled with Windows 10+ and Linux).
/// The API key goes through stdin so it never shows up in the process list.
fn chat_completion(cfg: &AiConfig, user: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let url = format!(
        "{}/chat/completions",
        cfg.base_url.trim().trim_end_matches('/')
    );
    let body = serde_json::json!({
        "model": cfg.model.trim(),
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user},
        ],
    });
    let body_path = std::env::temp_dir().join(format!("pentapen-ai-{}.json", std::process::id()));
    std::fs::write(&body_path, body.to_string()).map_err(|e| e.to_string())?;

    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "--max-time", "180", "-X", "POST", &url])
        .args(["-H", "Content-Type: application/json", "-H", "@-"])
        .arg("--data-binary")
        .arg(format!("@{}", body_path.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let _ = std::fs::remove_file(&body_path);
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
    if let Some(msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
        return Err(format!("API báo lỗi: {msg}"));
    }
    v.pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .ok_or_else(|| "API không trả về nội dung (choices[0].message.content).".to_string())
}

mod imp {
    use super::*;

    #[derive(Default, Debug, CompositeTemplate)]
    #[template(resource = "/com/github/flxzt/rnote/ui/aimenu.ui")]
    pub(crate) struct RnAiMenu {
        #[template_child]
        pub(crate) menubutton: TemplateChild<MenuButton>,
        #[template_child]
        pub(crate) popover: TemplateChild<Popover>,
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
        pub(crate) prompt_entry: TemplateChild<Entry>,
        #[template_child]
        pub(crate) send_button: TemplateChild<Button>,
        pub(crate) appwindow: RefCell<Option<glib::WeakRef<RnAppWindow>>>,
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
            self.menubutton.get().set_popover(Some(&self.popover.get()));
        }

        fn dispose(&self) {
            self.dispose_template();
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for RnAiMenu {}
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
        self.imp().popover.popup();
        self.imp().prompt_entry.grab_focus();
    }

    pub(crate) fn close_menu(&self) {
        self.imp().popover.popdown();
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

    pub(crate) fn init(&self, appwindow: &RnAppWindow) {
        let imp = self.imp();
        imp.appwindow.borrow_mut().replace(appwindow.downgrade());

        let config = AiConfig::load();
        imp.base_url_entry.set_text(&config.base_url);
        imp.model_entry.set_text(&config.model);
        imp.api_key_entry.set_text(&config.api_key);

        self.append_ai_message(
            "<b>Trợ lý Hình học AI</b>\n\
            • Nhấn chọn một hình bất kỳ trên bảng rồi nhập lệnh (vd: <i>kẻ đường cao AH</i>, <i>vẽ đường tròn nội tiếp</i>, <i>đổi cạnh BC thành nét đứt</i>, <i>tăng bán kính lên 120</i>) để AI chỉnh sửa trực tiếp trên hình đó.\n\
            • Hoặc nhập lệnh vẽ hình mới (vd: <i>vẽ tam giác ABC vuông tại A, AB = 4cm, AC = 3cm</i>).\n\
            Mở mục <b>Cấu hình API</b> phía trên để thiết lập kết nối.",
        );

        let this = self.clone();
        imp.save_key_button.connect_clicked(move |_| {
            let imp = this.imp();
            let cfg = AiConfig {
                base_url: imp.base_url_entry.text().trim().to_string(),
                model: imp.model_entry.text().trim().to_string(),
                api_key: imp.api_key_entry.text().trim().to_string(),
            };
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

        let this = self.clone();
        imp.popover
            .connect_show(move |_| this.update_center_status());
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
        imp.prompt_entry
            .connect_activate(move |_| this.send_prompt_entry());
    }

    fn send_prompt_entry(&self) {
        let entry = &self.imp().prompt_entry;
        let text = entry.text().to_string();
        if !text.trim().is_empty() {
            entry.set_text("");
            self.handle_user_prompt(&text);
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
        let (selected_target, anchor, existing) = {
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
            (sel, anchor, engine.named_figure_points())
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

            // Case 1: Editing a clicked/selected shape in-place!
            if let Some((sel_key, _, _, mut target_fig)) = selected_target {
                let warnings = apply_geometry_to_figure(&mut target_fig, &geom, &existing);
                if let Some(appwindow) = this.appwindow()
                    && let Some(canvas) = appwindow.active_tab_canvas()
                {
                    let flags = canvas
                        .engine_mut()
                        .update_shape_with_figure(sel_key, target_fig);
                    appwindow.handle_widget_flags(flags, &canvas);
                    this.update_center_status();
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
            if fig.points.is_empty() && restyled > 0 {
                this.append_ai_message(&glib::markup_escape_text(&explanation));
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
                let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);
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
}
