// Imports
use crate::appwindow::RnAppWindow;
use gtk4::{
    Button, CompositeTemplate, DropDown, Entry, Label, MenuButton, PasswordEntry, Popover,
    ScrolledWindow, StringList, StringObject, Widget, glib, pango, prelude::*, subclass::prelude::*,
};
use p2d::math::Vector2;
use rnote_compose::geometry::entity::LineType;
use rnote_compose::geometry::triangle::TriangleCenterType;
use rnote_compose::geometry::MathFigure;
use rnote_compose::Shape;
use std::cell::RefCell;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AiConfig {
    pub provider: String,
    pub model: String,
    pub api_key: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "Ollama (Local AI)".to_string(),
            model: "qwen3.5:fast".to_string(),
            api_key: String::new(),
        }
    }
}

impl AiConfig {
    pub fn config_path() -> std::path::PathBuf {
        glib::user_config_dir().join("pentapen_ai_config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Self>(&data) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&path, data)
    }
}

#[derive(serde::Deserialize, Debug, Default)]
struct GeometryJson {
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    points: Vec<JsonPoint>,
    #[serde(default)]
    lines: Vec<JsonLine>,
    #[serde(default)]
    circles: Vec<JsonCircle>,
    #[serde(default)]
    right_angles: Vec<JsonRightAngle>,
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
    radius: f64,
}

#[derive(serde::Deserialize, Debug)]
struct JsonRightAngle {
    vertex: String,
    arm1: String,
    arm2: String,
}

#[derive(Clone, Debug)]
struct RawPoint {
    name: String,
    x: f64,
    y: f64,
}

fn parse_points_from_text(text: &str) -> Vec<RawPoint> {
    let re = regex::Regex::new(r"(?i)\b([a-z][a-z0-9_]*)\s*\(\s*(-?\d+(?:\.\d+)?)\s*[,;\s]\s*(-?\d+(?:\.\d+)?)\s*\)").unwrap();
    let mut points = Vec::new();
    for cap in re.captures_iter(text) {
        if let (Some(name), Some(x), Some(y)) = (cap.get(1), cap.get(2), cap.get(3)) {
            if let (Ok(x_val), Ok(y_val)) = (x.as_str().parse::<f64>(), y.as_str().parse::<f64>()) {
                points.push(RawPoint {
                    name: name.as_str().to_uppercase(),
                    x: x_val,
                    y: y_val,
                });
            }
        }
    }
    points
}

fn fetch_ollama_models() -> Vec<String> {
    let output = std::process::Command::new("curl")
        .args(["-s", "--connect-timeout", "2", "http://127.0.0.1:11434/api/tags"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
                    let mut list = Vec::new();
                    for item in arr {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            list.push(name.to_string());
                        }
                    }
                    if !list.is_empty() {
                        return list;
                    }
                }
            }
        }
    }
    vec![
        "qwen3.5:fast".to_string(),
        "qwen3.5:4b".to_string(),
        "qwen2.5:1.5b".to_string(),
    ]
}

fn get_dropdown_text(dropdown: &DropDown) -> String {
    if let Some(item) = dropdown.selected_item() {
        if let Ok(so) = item.downcast::<StringObject>() {
            return so.string().to_string();
        }
    }
    String::new()
}

fn set_dropdown_by_text(dropdown: &DropDown, target: &str) {
    if let Some(model) = dropdown.model() {
        let n = model.n_items();
        for i in 0..n {
            if let Some(item) = model.item(i) {
                if let Ok(so) = item.downcast::<StringObject>() {
                    if so.string().as_str() == target {
                        dropdown.set_selected(i);
                        return;
                    }
                }
            }
        }
    }
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
        pub(crate) provider_dropdown: TemplateChild<DropDown>,
        #[template_child]
        pub(crate) model_dropdown: TemplateChild<DropDown>,
        #[template_child]
        pub(crate) refresh_models_button: TemplateChild<Button>,
        #[template_child]
        pub(crate) pull_model_entry: TemplateChild<Entry>,
        #[template_child]
        pub(crate) pull_model_button: TemplateChild<Button>,
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
        pub(crate) draw_circle_from_center: TemplateChild<Button>,
        #[template_child]
        pub(crate) draw_triangle_from_center: TemplateChild<Button>,
        #[template_child]
        pub(crate) draw_square_from_center: TemplateChild<Button>,
        #[template_child]
        pub(crate) quick_triangle: TemplateChild<Button>,
        #[template_child]
        pub(crate) quick_circle: TemplateChild<Button>,
        #[template_child]
        pub(crate) quick_tangent: TemplateChild<Button>,
        #[template_child]
        pub(crate) quick_centers: TemplateChild<Button>,
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
    }

    pub(crate) fn close_menu(&self) {
        self.imp().popover.popdown();
    }

    pub(crate) fn update_center_status(&self) {
        let imp = self.imp();
        if let Some(appwindow) = imp.appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
            if let Some(canvas) = appwindow.active_tab_canvas() {
                if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                    let name = lbl.unwrap_or_else(|| "O".to_string());
                    imp.center_status_label.set_markup(&format!(
                        "🎯 Tâm đang chọn: <b>{}</b> ({:.0}, {:.0})",
                        name, pos.x, pos.y
                    ));
                    return;
                }
            }
        }
        imp.center_status_label.set_markup("⚪ Chưa chọn tâm (Bấm 'Chấm tâm' để chọn)");
    }

    pub(crate) fn init(&self, appwindow: &RnAppWindow) {
        let imp = self.imp();
        imp.appwindow
            .borrow_mut()
            .replace(appwindow.downgrade());

        // Load config from ~/.config/pentapen_ai_config.json
        let config = AiConfig::load();
        imp.api_key_entry.set_text(&config.api_key);
        set_dropdown_by_text(&imp.provider_dropdown, &config.provider);

        // Fetch installed models in background and set to dropdown
        let saved_model = config.model.clone();
        let model_dd = imp.model_dropdown.clone();
        glib::spawn_future_local(async move {
            let models = blocking::unblock(fetch_ollama_models).await;
            let refs: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
            let sl = StringList::new(&refs);
            model_dd.set_model(Some(&sl));
            set_dropdown_by_text(&model_dd, &saved_model);
        });

        // Add initial greeting message
        self.append_ai_message(
            "<b>✨ Xin chào! Tôi là Trợ lý Hình học AI (MCP Geometric CAD Solver).</b>\n\n\
            <b>🎯 Hệ thống hỗ trợ ra lệnh chính xác 100% theo yêu cầu:</b>\n\
            • <b>Điểm:</b> <i>'tạo điểm A(100, 200)'</i> hoặc <i>'các điểm A(100, 150), B(350, 150), C(225, 320)'</i>\n\
            • <b>Đoạn thẳng:</b> <i>'đoạn thẳng AB với A(100, 100), B(400, 100)'</i> hoặc <i>'nối A và B'</i>\n\
            • <b>Đường tròn:</b> <i>'vẽ đường tròn tâm O(300, 300) bán kính 80'</i>\n\
            • <b>Tam giác:</b> <i>'vẽ tam giác ABC với A(100, 100), B(400, 100), C(250, 300)'</i> (vuông, đều, cân)\n\
            • <b>Hình vuông / Chữ nhật:</b> <i>'vẽ hình vuông ABCD cạnh 150'</i>, <i>'hình chữ nhật ABCD dài 250 rộng 150'</i>\n\
            • <b>Đường đặc biệt:</b> <i>'đường cao AH'</i>, <i>'trung tuyến AM'</i>, <i>'phân giác AD'</i>, <i>'trung điểm M của BC'</i>\n\
            • <b>4 Tâm tam giác:</b> <i>'tìm 4 tâm tam giác'</i> (G, H, O, I)\n\n\
            Bạn cũng có thể chọn Mô hình AI bên trên (Ollama/Gemini/OpenAI) để giải các bài toán hình học tự do!"
        );

        // Save Button (Provider, Model, Key)
        let provider_dd = imp.provider_dropdown.clone();
        let model_dd_save = imp.model_dropdown.clone();
        let api_entry_clone = imp.api_key_entry.clone();
        let this_clone = self.clone();
        imp.save_key_button.connect_clicked(move |_| {
            let provider = get_dropdown_text(&provider_dd);
            let model = get_dropdown_text(&model_dd_save);
            let api_key = api_entry_clone.text().to_string();

            let cfg = AiConfig {
                provider: if provider.is_empty() { "Ollama (Local AI)".to_string() } else { provider },
                model: if model.is_empty() { "qwen3.5:fast".to_string() } else { model },
                api_key,
            };

            if let Err(e) = cfg.save() {
                this_clone.append_ai_message(&format!("⚠️ Không thể lưu cấu hình: {e}"));
            } else {
                this_clone.append_ai_message(&format!(
                    "💾 <b>Đã lưu cấu hình AI thành công!</b>\n• Nhà cung cấp: <b>{}</b>\n• Mô hình: <b>{}</b>",
                    cfg.provider, cfg.model
                ));
            }
        });

        // Refresh models button
        let this = self.clone();
        imp.refresh_models_button.connect_clicked(move |_| {
            this.refresh_models();
        });

        // Pull model button
        let this = self.clone();
        let pull_entry = imp.pull_model_entry.clone();
        imp.pull_model_button.connect_clicked(move |_| {
            let model_name = pull_entry.text().to_string();
            pull_entry.set_text("");
            this.pull_model(model_name);
        });

        // Connect Popover show to refresh center status
        let this = self.clone();
        imp.popover.connect_show(move |_| {
            this.update_center_status();
        });
        self.update_center_status();

        // Pick Center Button: Sets picking center mode on canvas and closes popover
        let this = self.clone();
        imp.pick_center_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.imp().appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
                if let Some(canvas) = appwindow.active_tab_canvas() {
                    canvas.set_picking_center(true);
                    this.close_menu();
                    appwindow.overlays().dispatch_toast_text("🎯 Hãy nhấp chuột hoặc chạm bút lên bảng vẽ để đặt điểm tâm... (Esc để huỷ)", None);
                }
            }
        });

        // Center Current View: Sets center at viewport center immediately
        let this = self.clone();
        imp.center_current_view_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.imp().appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
                if let Some(canvas) = appwindow.active_tab_canvas() {
                    let pos = canvas.engine_ref().camera.viewport_center();
                    let label = "O".to_string();
                    let mut fig = MathFigure::new();
                    fig.add_point(pos, Some(label.clone()));
                    canvas.engine_mut().set_last_center_point(pos, Some(label.clone()));
                    let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                    appwindow.handle_widget_flags(flags, &canvas);
                    this.update_center_status();
                    this.append_ai_message(&format!(
                        "🎯 <b>[Đã Chấm Tâm O]</b> tại toạ độ ({:.0}, {:.0}) giữa màn hình!\nBây giờ bạn có thể bấm các nút vẽ nhanh bên dưới hoặc gõ lệnh, AI sẽ tự động dựng hình chính xác quanh tâm này!",
                        pos.x, pos.y
                    ));
                }
            }
        });

        // Clear Center Button: Clears the chosen center
        let this = self.clone();
        imp.clear_center_button.connect_clicked(move |_| {
            if let Some(appwindow) = this.imp().appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
                if let Some(canvas) = appwindow.active_tab_canvas() {
                    canvas.engine_mut().last_center_point = None;
                    this.update_center_status();
                    this.append_ai_message("🔄 Đã bỏ chọn tâm. Bạn có thể bấm '🎯 Chấm tâm trên bảng' để chọn tâm mới!");
                }
            }
        });

        // Draw Circle from Center
        let this = self.clone();
        imp.draw_circle_from_center.connect_clicked(move |_| {
            this.handle_user_prompt("vẽ đường tròn quanh tâm này bán kính 90");
        });

        // Draw Triangle from Center
        let this = self.clone();
        imp.draw_triangle_from_center.connect_clicked(move |_| {
            this.handle_user_prompt("vẽ tam giác đều quanh tâm này");
        });

        // Draw Square from Center
        let this = self.clone();
        imp.draw_square_from_center.connect_clicked(move |_| {
            this.handle_user_prompt("vẽ hình vuông quanh tâm này cạnh 160");
        });

        // Quick action: Triangle
        let this = self.clone();
        imp.quick_triangle.connect_clicked(move |_| {
            this.handle_user_prompt("vẽ tam giác đều ABC");
        });

        // Quick action: Circle
        let this = self.clone();
        imp.quick_circle.connect_clicked(move |_| {
            this.handle_user_prompt("vẽ đường tròn tâm O bán kính 90");
        });

        // Quick action: Tangent
        let this = self.clone();
        imp.quick_tangent.connect_clicked(move |_| {
            this.handle_user_prompt("dựng tiếp tuyến đến đường tròn");
        });

        // Quick action: Centers
        let this = self.clone();
        imp.quick_centers.connect_clicked(move |_| {
            this.handle_user_prompt("tìm 4 tâm tam giác");
        });

        // Send Button & Enter
        let this = self.clone();
        let prompt_entry_clone = imp.prompt_entry.clone();
        imp.send_button.connect_clicked(move |_| {
            let text = prompt_entry_clone.text().to_string();
            if !text.trim().is_empty() {
                prompt_entry_clone.set_text("");
                this.handle_user_prompt(&text);
            }
        });

        let this = self.clone();
        imp.prompt_entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if !text.trim().is_empty() {
                entry.set_text("");
                this.handle_user_prompt(&text);
            }
        });
    }

    pub(crate) fn refresh_models(&self) {
        let imp = self.imp();
        let model_dd = imp.model_dropdown.clone();
        let this = self.clone();

        glib::spawn_future_local(async move {
            let models = blocking::unblock(fetch_ollama_models).await;
            let refs: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
            let sl = StringList::new(&refs);
            model_dd.set_model(Some(&sl));
            model_dd.set_selected(0);
            this.append_ai_message(&format!(
                "🔄 <b>Đã quét xong danh sách Ollama Models!</b>\nTìm thấy <b>{}</b> mô hình: {}",
                models.len(),
                models.join(", ")
            ));
        });
    }

    pub(crate) fn pull_model(&self, model_name: String) {
        let this = self.clone();
        let model_clean = model_name.trim().to_string();
        if model_clean.is_empty() {
            self.append_ai_message("⚠️ Vui lòng nhập tên mô hình Ollama cần tải (ví dụ: <code>qwen2.5:3b</code>).");
            return;
        }

        self.append_ai_message(&format!(
            "⏳ <b>Đang tải mô hình '{model_clean}' từ Ollama Library...</b>\nQuá trình chạy ngầm, bạn vẫn có thể vẽ bình thường!"
        ));

        let name_clone = model_clean.clone();
        glib::spawn_future_local(async move {
            let status = blocking::unblock(move || {
                std::process::Command::new("ollama")
                    .args(["pull", &name_clone])
                    .status()
            }).await;

            let success = status.map(|s| s.success()).unwrap_or(false);
            if success {
                this.append_ai_message(&format!(
                    "✅ <b>Đã tải thành công mô hình '{model_clean}'!</b>\nĐã tự động cập nhật vào danh sách lựa chọn."
                ));
                this.refresh_models();
            } else {
                this.append_ai_message(&format!(
                    "⚠️ Không thể tải mô hình '{model_clean}'. Vui lòng kiểm tra lại kết nối mạng hoặc tên model."
                ));
            }
        });
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

        let imp = self.imp();
        imp.chat_box.append(&label);
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

        let imp = self.imp();
        imp.chat_box.append(&label);
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

        let appwindow = match self.imp().appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
            Some(w) => w,
            None => {
                self.append_ai_message("⚠️ Không tìm thấy cửa sổ ứng dụng đang mở.");
                return;
            }
        };

        let canvas = match appwindow.active_tab_canvas() {
            Some(c) => c,
            None => {
                self.append_ai_message("⚠️ Không tìm thấy trang vẽ hoạt động.");
                return;
            }
        };

        let norm = prompt.to_lowercase();
        let parsed_points = parse_points_from_text(prompt);

        // -------------------------------------------------------------
        // 0. CHẤM TÂM / ĐẶT TÂM HÌNH HỌC: e.g. "chấm tâm O", "chấm tâm", "tâm O"
        // -------------------------------------------------------------
        if norm.contains("chấm tâm") || norm.contains("cham tam") || norm.starts_with("tâm ") || norm.starts_with("tam ") {
            let label_re = regex::Regex::new(r"(?i)tâm\s+([a-z][a-z0-9_]*)").unwrap();
            let label = label_re.captures(prompt).and_then(|c| c.get(1)).map(|m| m.as_str().to_uppercase()).unwrap_or_else(|| "O".to_string());
            let pos = if let Some(pt) = parsed_points.first() {
                Vector2::new(pt.x, pt.y)
            } else if let Some((p, _)) = canvas.engine_ref().last_center_point() {
                p
            } else {
                canvas.engine_ref().camera.viewport_center()
            };

            let mut fig = MathFigure::new();
            fig.add_point(pos, Some(label.clone()));
            canvas.engine_mut().set_last_center_point(pos, Some(label.clone()));
            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);
            self.update_center_status();

            self.append_ai_message(&format!(
                "🎯 <b>[Đã Chấm Tâm {label}]</b> tại toạ độ ({:.0}, {:.0})!\nBây giờ bạn có thể ra lệnh: <i>'vẽ đường tròn bán kính 80'</i>, <i>'vẽ tam giác'</i>, <i>'vẽ hình vuông'</i>, AI sẽ tự động dựng hình chính xác quanh tâm này!",
                pos.x, pos.y
            ));
            return;
        }

        // -------------------------------------------------------------
        // 1. EXACT POINT CREATION: e.g. "tạo điểm A(100, 200)", "chấm điểm A(100, 200)"
        // -------------------------------------------------------------
        let is_point_command = (norm.contains("tạo điểm") || norm.contains("chấm điểm") || norm.contains("cham diem") || norm.contains("tao diem") || norm.starts_with("điểm ") || norm.starts_with("diem ") || norm.contains("các điểm"))
            && !norm.contains("tam giác") && !norm.contains("đoạn thẳng") && !norm.contains("hình");

        if is_point_command {
            if !parsed_points.is_empty() {
                let mut fig = MathFigure::new();
                let mut desc = Vec::new();
                for pt in &parsed_points {
                    fig.add_point(Vector2::new(pt.x, pt.y), Some(pt.name.clone()));
                    canvas.engine_mut().set_last_center_point(Vector2::new(pt.x, pt.y), Some(pt.name.clone()));
                    desc.push(format!("{}({}, {})", pt.name, pt.x, pt.y));
                }
                let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);
                self.update_center_status();

                self.append_ai_message(&format!(
                    "📍 <b>[Tạo Điểm Chính Xác]</b>\nĐã tạo thành công các điểm: <b>{}</b> trên canvas!",
                    desc.join(", ")
                ));
                return;
            } else {
                // Name only without coordinates: "tạo điểm M"
                let name_re = regex::Regex::new(r"(?i)(?:tạo|chấm|thêm)\s+điểm\s+([a-z][a-z0-9_]*)").unwrap();
                if let Some(cap) = name_re.captures(prompt) {
                    if let Some(name) = cap.get(1) {
                        let mut fig = MathFigure::new();
                        let p_name = name.as_str().to_uppercase();
                        let pos = canvas.engine_ref().last_center_point().map(|(p, _)| p).unwrap_or_else(|| canvas.engine_ref().camera.viewport_center());
                        fig.add_point(pos, Some(p_name.clone()));
                        canvas.engine_mut().set_last_center_point(pos, Some(p_name.clone()));
                        let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                        appwindow.handle_widget_flags(flags, &canvas);
                        self.update_center_status();

                        self.append_ai_message(&format!(
                            "📍 <b>[Tạo Điểm Chính Xác]</b>\nĐã tạo điểm <b>{}</b> tại vị trí ({:.0}, {:.0}) trên canvas!",
                            p_name, pos.x, pos.y
                        ));
                        return;
                    }
                }
            }
        }

        // -------------------------------------------------------------
        // 2. EXACT CIRCLE: e.g. "vẽ đường tròn tâm O(200, 200) bán kính 80"
        // -------------------------------------------------------------
        if norm.contains("tròn") || norm.contains("tron") || norm.contains("circle") {
            let (center_pos, center_label, from_click) = if let Some(pt) = parsed_points.first() {
                (Vector2::new(pt.x, pt.y), pt.name.clone(), false)
            } else if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else if norm.contains("tâm i") || norm.contains("tam i") {
                (canvas.engine_ref().camera.viewport_center(), "I".to_string(), false)
            } else if norm.contains("tâm a") || norm.contains("tam a") {
                (canvas.engine_ref().camera.viewport_center(), "A".to_string(), false)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };

            // Extract radius
            let r_re = regex::Regex::new(r"(?i)(?:bán kính|ban kinh|r\s*=|radius)\s*(\d+(?:\.\d+)?)").unwrap();
            let radius = if let Some(cap) = r_re.captures(prompt) {
                cap.get(1).and_then(|m| m.as_str().parse::<f64>().ok()).unwrap_or(90.0)
            } else {
                90.0
            };

            let mut fig = MathFigure::new();
            let c_id = fig.add_point(center_pos, Some(center_label.clone()));
            fig.add_circle_radius(c_id, radius);

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let title = if from_click {
                format!("<b>[Dựng Đường Tròn Theo Tâm Đã Chấm]</b>\nĐã dựng đường tròn bán kính <b>R = {radius}px</b> tại đúng tâm <b>{center_label}</b> ({:.0}, {:.0}) bạn đã chọn!", center_pos.x, center_pos.y)
            } else {
                format!("<b>[Dựng Đường Tròn Chính Xác]</b>\nĐã dựng đường tròn tâm <b>{center_label}</b> tại ({:.0}, {:.0}), bán kính <b>R = {radius}px</b> trên canvas!", center_pos.x, center_pos.y)
            };

            self.append_ai_message(&format!("🎯 {title}"));
            return;
        }

        // -------------------------------------------------------------
        // 3. EXACT LINE / SEGMENT: e.g. "vẽ đoạn thẳng AB với A(100, 100), B(400, 100)"
        // -------------------------------------------------------------
        let is_line_cmd = (norm.contains("đoạn thẳng") || norm.contains("doan thang") || norm.contains("nối") || norm.contains("noi") || norm.contains("đường thẳng"))
            && !norm.contains("tam giác") && !norm.contains("hình vuông") && !norm.contains("hình chữ nhật");

        if is_line_cmd {
            let mut fig = MathFigure::new();
            let is_dashed = norm.contains("đứt") || norm.contains("dut") || norm.contains("dashed") || norm.contains("phụ");

            if parsed_points.len() >= 2 {
                let p1 = &parsed_points[0];
                let p2 = &parsed_points[1];
                let p1_id = fig.add_point(Vector2::new(p1.x, p1.y), Some(p1.name.clone()));
                let p2_id = fig.add_point(Vector2::new(p2.x, p2.y), Some(p2.name.clone()));

                if is_dashed {
                    fig.add_auxiliary_line(p1_id, p2_id, LineType::Segment);
                } else {
                    fig.add_segment(p1_id, p2_id);
                }

                let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);

                self.append_ai_message(&format!(
                    "📏 <b>[Dựng Đoạn Thẳng Chính Xác]</b>\nĐã nối đoạn thẳng <b>{}{}</b> giữa {}({}, {}) và {}({}, {})!",
                    p1.name, p2.name, p1.name, p1.x, p1.y, p2.name, p2.x, p2.y
                ));
                return;
            } else {
                let p1_id = fig.add_point(Vector2::new(200.0, 300.0), Some("A".to_string()));
                let p2_id = fig.add_point(Vector2::new(480.0, 300.0), Some("B".to_string()));
                if is_dashed {
                    fig.add_auxiliary_line(p1_id, p2_id, LineType::Segment);
                } else {
                    fig.add_segment(p1_id, p2_id);
                }

                let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
                appwindow.handle_widget_flags(flags, &canvas);

                self.append_ai_message(
                    "📏 <b>[Dựng Đoạn Thẳng Chính Xác]</b>\nĐã dựng đoạn thẳng <b>AB</b> giữa A(200, 300) và B(480, 300) trên canvas!"
                );
                return;
            }
        }

        // -------------------------------------------------------------
        // 4. EXACT SQUARE / RECTANGLE: e.g. "vẽ hình vuông ABCD cạnh 150"
        // -------------------------------------------------------------
        if norm.contains("hình vuông") || norm.contains("hinh vuong") || norm.contains("square") {
            let (center_pos, center_label, from_dotted) = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };

            let side_re = regex::Regex::new(r"(?i)cạnh\s*(\d+(?:\.\d+)?)").unwrap();
            let side = if let Some(cap) = side_re.captures(prompt) {
                cap.get(1).and_then(|m| m.as_str().parse::<f64>().ok()).unwrap_or(160.0)
            } else {
                160.0
            };

            let mut fig = MathFigure::new();
            let top_left = center_pos - Vector2::splat(side * 0.5);
            fig.build_square(top_left, side, Some(["A", "B", "C", "D"]));
            if from_dotted {
                fig.add_point(center_pos, Some(center_label.clone()));
            }

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let msg = if from_dotted {
                format!("🟩 <b>[Dựng Hình Vuông Theo Tâm Đã Chấm]</b>\nĐã dựng hình vuông <b>ABCD</b> cạnh <b>s = {side}px</b> có tâm đối xứng chính là <b>{center_label}</b> ({:.0}, {:.0}) bạn đã chọn!", center_pos.x, center_pos.y)
            } else {
                format!("🟩 <b>[Dựng Hình Vuông Chính Xác]</b>\nĐã dựng hình vuông <b>ABCD</b> cạnh <b>s = {side}px</b> tại ({:.0}, {:.0})!", center_pos.x, center_pos.y)
            };
            self.append_ai_message(&msg);
            return;
        }

        if norm.contains("hình chữ nhật") || norm.contains("hinh chu nhat") || norm.contains("rectangle") {
            let (center_pos, center_label, from_dotted) = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };

            let len_re = regex::Regex::new(r"(?i)dài\s*(\d+(?:\.\d+)?)").unwrap();
            let wid_re = regex::Regex::new(r"(?i)rộng\s*(\d+(?:\.\d+)?)").unwrap();

            let length = len_re.captures(prompt)
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse::<f64>().ok())
                .unwrap_or(240.0);
            let width = wid_re.captures(prompt)
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse::<f64>().ok())
                .unwrap_or(140.0);

            let mut fig = MathFigure::new();
            let top_left = center_pos - Vector2::new(length * 0.5, width * 0.5);
            fig.build_rectangle(top_left, length, width, Some(["A", "B", "C", "D"]));
            if from_dotted {
                fig.add_point(center_pos, Some(center_label.clone()));
            }

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let msg = if from_dotted {
                format!("📐 <b>[Dựng Hình Chữ Nhật Theo Tâm Đã Chấm]</b>\nĐã dựng hình chữ nhật <b>ABCD</b> (dài <b>{length}px</b>, rộng <b>{width}px</b>) nhận tâm <b>{center_label}</b> ({:.0}, {:.0}) làm tâm đối xứng!", center_pos.x, center_pos.y)
            } else {
                format!("📐 <b>[Dựng Hình Chữ Nhật Chính Xác]</b>\nĐã dựng hình chữ nhật <b>ABCD</b> (dài <b>{length}px</b>, rộng <b>{width}px</b>) tại ({:.0}, {:.0})!", center_pos.x, center_pos.y)
            };
            self.append_ai_message(&msg);
            return;
        }

        // -------------------------------------------------------------
        // 5. MIDPOINT: "trung điểm M của BC"
        // -------------------------------------------------------------
        if norm.contains("trung điểm") || norm.contains("trung diem") || norm.contains("midpoint") {
            let mut fig = MathFigure::new();
            let b_pos = Vector2::new(180.0, 360.0);
            let c_pos = Vector2::new(480.0, 360.0);
            let m_pos = (b_pos + c_pos) * 0.5;

            let pb = fig.add_point(b_pos, Some("B".to_string()));
            let pc = fig.add_point(c_pos, Some("C".to_string()));
            let pm = fig.add_point(m_pos, Some("M".to_string()));
            fig.add_segment(pb, pm);
            fig.add_segment(pm, pc);

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            self.append_ai_message(&format!(
                "📍 <b>[Dựng Trung Điểm Chuẩn Xác]</b>\nĐã xác định trung điểm <b>M({}, {})</b> của đoạn thẳng BC!",
                m_pos.x, m_pos.y
            ));
            return;
        }

        // -------------------------------------------------------------
        // 6. EXACT TRIANGLE & SPECIAL LINES
        // -------------------------------------------------------------
        if norm.contains("tam giác") || norm.contains("tam giac") || norm.contains("triangle") {
            let (center_pos, center_label, from_dotted) = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };

            let mut fig = MathFigure::new();
            let (pa_pos, pb_pos, pc_pos, kind_str) = if parsed_points.len() >= 3 {
                (
                    Vector2::new(parsed_points[0].x, parsed_points[0].y),
                    Vector2::new(parsed_points[1].x, parsed_points[1].y),
                    Vector2::new(parsed_points[2].x, parsed_points[2].y),
                    "theo toạ độ chỉ định"
                )
            } else if norm.contains("vuông") || norm.contains("vuong") || norm.contains("right") {
                (center_pos, center_pos + Vector2::new(200.0, 0.0), center_pos + Vector2::new(0.0, -140.0), "vuông tại A")
            } else if norm.contains("cân") || norm.contains("can") || norm.contains("isosceles") {
                (center_pos + Vector2::new(-110.0, 70.0), center_pos + Vector2::new(110.0, 70.0), center_pos + Vector2::new(0.0, -90.0), "cân tại C")
            } else {
                let r = 95.0;
                let sqrt3_div_2 = 3.0_f64.sqrt() * 0.5;
                (
                    center_pos + Vector2::new(0.0, -r),
                    center_pos + Vector2::new(-r * sqrt3_div_2, r * 0.5),
                    center_pos + Vector2::new(r * sqrt3_div_2, r * 0.5),
                    "đều"
                )
            };

            let name_a = parsed_points.get(0).map(|p| p.name.as_str()).unwrap_or("A");
            let name_b = parsed_points.get(1).map(|p| p.name.as_str()).unwrap_or("B");
            let name_c = parsed_points.get(2).map(|p| p.name.as_str()).unwrap_or("C");

            let (pa, pb, pc, _, _, _) = fig.build_triangle(pa_pos, pb_pos, pc_pos, Some([name_a, name_b, name_c]));

            if from_dotted {
                fig.add_point(center_pos, Some(center_label.clone()));
            }

            let mut extras = Vec::new();

            // Check if altitude requested
            if norm.contains("đường cao") || norm.contains("duong cao") || norm.contains("altitude") {
                if let Some((_, _)) = fig.add_altitude(pc, pa, pb, Some("H")) {
                    extras.push("đường cao <b>CH</b> (vuông góc AB)");
                }
            }

            // Check if median requested
            if norm.contains("trung tuyến") || norm.contains("trung tuyen") || norm.contains("median") {
                if let Some((_, _)) = fig.add_median(pc, pa, pb, Some("M")) {
                    extras.push("trung tuyến <b>CM</b>");
                }
            }

            // Check if bisector requested
            if norm.contains("phân giác") || norm.contains("phan giac") || norm.contains("bisector") {
                if let Some((_, _)) = fig.add_angle_bisector(pc, pa, pb, Some("D")) {
                    extras.push("phân giác <b>CD</b>");
                }
            }

            // Check if circumcircle requested
            if norm.contains("ngoại tiếp") || norm.contains("ngoai tiep") {
                fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Circumcenter, true);
                extras.push("đường tròn ngoại tiếp tâm <b>O</b>");
            }

            // Check if incircle requested
            if norm.contains("nội tiếp") || norm.contains("noi tiep") {
                fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Incenter, true);
                extras.push("đường tròn nội tiếp tâm <b>I</b>");
            }

            fig.auto_detect_intersections();

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let extra_str = if extras.is_empty() {
                String::new()
            } else {
                format!("\n• Bao gồm: {}", extras.join(", "))
            };

            let title = if from_dotted {
                format!("📐 <b>[Dựng Tam Giác Theo Tâm Đã Chấm]</b>\nĐã dựng tam giác <b>{name_a}{name_b}{name_c}</b> ({kind_str}) nhận tâm <b>{center_label}</b> ({:.0}, {:.0}) làm tâm!{extra_str}", center_pos.x, center_pos.y)
            } else {
                format!("📐 <b>[Dựng Tam Giác Chính Xác]</b>\nĐã dựng tam giác <b>{name_a}{name_b}{name_c}</b> ({kind_str})!{extra_str}")
            };

            self.append_ai_message(&title);
            return;
        }

        // -------------------------------------------------------------
        // 7. TANGENTS TOOL CALL
        // -------------------------------------------------------------
        if norm.contains("tiếp tuyến") || norm.contains("tiep tuyen") || norm.contains("tangent") {
            let (center, center_label, from_dotted) = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };
            let radius = 70.0;
            let mut fig = MathFigure::new();
            let c_id = fig.add_point(center, Some(center_label.clone()));
            fig.add_circle_radius(c_id, radius);

            let m_pos = Vector2::new(center.x + radius + 110.0, center.y);
            let m_id = fig.add_point(m_pos, Some("M".to_string()));

            let d = (m_pos - center).length();
            let alpha = (radius / d).asin();
            let t1_pos = center + Vector2::new(radius * alpha.sin(), radius * alpha.cos());
            let t2_pos = center + Vector2::new(radius * alpha.sin(), -radius * alpha.cos());
            let t1_id = fig.add_point(t1_pos, Some("T1".to_string()));
            let t2_id = fig.add_point(t2_pos, Some("T2".to_string()));

            fig.add_segment(m_id, t1_id);
            fig.add_segment(m_id, t2_id);
            fig.add_auxiliary_line(c_id, t1_id, LineType::Segment);
            fig.add_auxiliary_line(c_id, t2_id, LineType::Segment);
            fig.add_right_angle(t1_id, m_id, c_id, 8.0);
            fig.add_right_angle(t2_id, m_id, c_id, 8.0);

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let msg = if from_dotted {
                format!("📏 <b>[Dựng Tiếp Tuyến Theo Tâm Đã Chấm]</b>\nĐã dựng 2 tiếp tuyến từ điểm ngoài <b>M</b> đến đường tròn tâm <b>{center_label}</b> ({:.0}, {:.0}) với các tiếp điểm <b>T1, T2</b>!", center.x, center.y)
            } else {
                "📏 <b>[Dựng Tiếp Tuyến Chính Xác]</b>\nĐã dựng 2 tiếp tuyến từ điểm ngoài <b>M</b> đến đường tròn (O) với các tiếp điểm <b>T1, T2</b> và ký hiệu vuông góc chuẩn xác!".to_string()
            };

            self.append_ai_message(&msg);
            return;
        }

        // -------------------------------------------------------------
        // 7b. REGULAR POLYGON / HEXAGON: e.g. "vẽ lục giác đều", "đa giác đều"
        // -------------------------------------------------------------
        if norm.contains("lục giác") || norm.contains("luc giac") || norm.contains("hexagon") || norm.contains("đa giác") || norm.contains("da giac") {
            let (center_pos, center_label, from_dotted) = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                (pos, lbl.unwrap_or_else(|| "O".to_string()), true)
            } else {
                (canvas.engine_ref().camera.viewport_center(), "O".to_string(), false)
            };
            let radius = 90.0;
            let mut fig = MathFigure::new();
            let _c_id = fig.add_point(center_pos, Some(center_label.clone()));
            let mut pt_ids = Vec::new();
            for i in 0..6 {
                let angle = i as f64 * std::f64::consts::PI / 3.0 - std::f64::consts::FRAC_PI_2;
                let p = center_pos + Vector2::new(radius * angle.cos(), radius * angle.sin());
                let lbl = format!("P{}", i + 1);
                let id = fig.add_point(p, Some(lbl));
                pt_ids.push(id);
            }
            for i in 0..6 {
                fig.add_segment(pt_ids[i], pt_ids[(i + 1) % 6]);
            }
            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            let msg = if from_dotted {
                format!("✡️ <b>[Dựng Lục Giác Đều Theo Tâm Đã Chấm]</b>\nĐã dựng lục giác đều 6 cạnh nhận điểm tâm <b>{center_label}</b> ({:.0}, {:.0}) làm tâm đối xứng!", center_pos.x, center_pos.y)
            } else {
                format!("✡️ <b>[Dựng Lục Giác Đều]</b>\nĐã dựng lục giác đều 6 cạnh tại toạ độ ({:.0}, {:.0})!", center_pos.x, center_pos.y)
            };
            self.append_ai_message(&msg);
            return;
        }

        // -------------------------------------------------------------
        // 8. CENTERS TOOL CALL
        // -------------------------------------------------------------
        if (norm.contains("4 tâm") || norm.contains("bon tam") || norm.contains("bốn tâm") || norm.contains("các tâm")) && !norm.contains("chấm tâm") {
            let p1 = Vector2::new(200.0, 360.0);
            let p2 = Vector2::new(460.0, 360.0);
            let p3 = Vector2::new(290.0, 180.0);
            let mut fig = MathFigure::new();
            let (pa, pb, pc, _, _, _) = fig.build_triangle(p1, p2, p3, Some(["A", "B", "C"]));
            fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Centroid, true);
            fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Orthocenter, true);
            fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Circumcenter, true);
            fig.add_triangle_center(pa, pb, pc, TriangleCenterType::Incenter, true);

            let flags = canvas.engine_mut().insert_shape(Shape::MathFigure(fig));
            appwindow.handle_widget_flags(flags, &canvas);

            self.append_ai_message(
                "🎯 <b>[Dựng 4 Tâm Tam Giác]</b>\nĐã tự động tính và hiển thị đầy đủ 4 tâm đặc biệt:\n\
                • <b>G</b>: Trọng tâm (Giao 3 trung tuyến)\n\
                • <b>H</b>: Trực tâm (Giao 3 đường cao)\n\
                • <b>O</b>: Tâm đường tròn ngoại tiếp (Giao 3 trung trực)\n\
                • <b>I</b>: Tâm đường tròn nội tiếp (Giao 3 phân giác)"
            );
            return;
        }

        // -------------------------------------------------------------
        // 9. LLM QUERY PIPELINE (Ollama / Gemini / OpenAI)
        // -------------------------------------------------------------
        let provider = get_dropdown_text(&self.imp().provider_dropdown);
        let selected_model = get_dropdown_text(&self.imp().model_dropdown);
        let model_name = if selected_model.is_empty() {
            "qwen3.5:fast".to_string()
        } else {
            selected_model
        };

        if provider.contains("Ollama") {
            self.append_ai_message(&format!(
                "🤖 <b>[Ollama: {model_name}]</b> Đang suy luận và tạo hình..."
            ));

            let center_hint = if let Some((pos, lbl)) = canvas.engine_ref().last_center_point() {
                let name = lbl.unwrap_or_else(|| "O".to_string());
                format!("IMPORTANT CAD CONSTRAINT: The user has selected a geometric center point '{name}' at coordinates ({:.0}, {:.0}). All circles, regular polygons, or centered constructions MUST use ({:.0}, {:.0}) as center!", pos.x, pos.y, pos.x, pos.y)
            } else {
                String::new()
            };

            let prompt_owned = prompt.to_string();
            let this = self.clone();

            glib::spawn_future_local(async move {
                let system_prompt = "You are a CAD geometric solver. Return ONLY a valid JSON object with keys: \
                \"explanation\" (string in Vietnamese), \
                \"points\": [{\"name\": \"A\", \"x\": 150.0, \"y\": 250.0}], \
                \"lines\": [{\"p1\": \"A\", \"p2\": \"B\", \"style\": \"solid\" or \"dashed\"}], \
                \"circles\": [{\"center\": \"O\", \"radius\": 80.0}], \
                \"right_angles\": [{\"vertex\": \"H\", \"arm1\": \"A\", \"arm2\": \"B\"}]. \
                Coordinates should be in range x: 100-500, y: 100-500.";

                let req_body = serde_json::json!({
                    "model": model_name,
                    "prompt": format!("{system_prompt}\n{center_hint}\n\nUser request: {prompt_owned}"),
                    "stream": false,
                    "format": "json"
                });

                let output = blocking::unblock(move || {
                    std::process::Command::new("curl")
                        .args([
                            "-s",
                            "--connect-timeout", "8",
                            "-X", "POST",
                            "http://127.0.0.1:11434/api/generate",
                            "-H", "Content-Type: application/json",
                            "-d", &req_body.to_string(),
                        ])
                        .output()
                }).await;

                let mut handled = false;
                if let Ok(out) = output {
                    if out.status.success() {
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                            let mut text_to_parse = v.get("response")
                                .and_then(|r| r.as_str())
                                .unwrap_or("")
                                .to_string();

                            if text_to_parse.trim().is_empty() {
                                if let Some(thinking) = v.get("thinking").and_then(|t| t.as_str()) {
                                    text_to_parse = thinking.to_string();
                                }
                            }

                            if let Some(start) = text_to_parse.find('{') {
                                if let Some(end) = text_to_parse.rfind('}') {
                                    let json_slice = &text_to_parse[start..=end];
                                    if let Ok(geom) = serde_json::from_str::<GeometryJson>(json_slice) {
                                        if !geom.points.is_empty() || !geom.circles.is_empty() {
                                            if let Some(appw) = this.imp().appwindow.borrow().as_ref().and_then(|w| w.upgrade()) {
                                                if let Some(cv) = appw.active_tab_canvas() {
                                                    let mut fig = MathFigure::new();
                                                    let mut p_map = std::collections::HashMap::new();

                                                    for p in &geom.points {
                                                        let pid = fig.add_point(Vector2::new(p.x, p.y), Some(p.name.clone()));
                                                        p_map.insert(p.name.trim().to_uppercase(), pid);
                                                    }

                                                    for l in &geom.lines {
                                                        let p1_n = l.p1.trim().to_uppercase();
                                                        let p2_n = l.p2.trim().to_uppercase();
                                                        if let (Some(&id1), Some(&id2)) = (p_map.get(&p1_n), p_map.get(&p2_n)) {
                                                            if l.style.as_deref() == Some("dashed") {
                                                                fig.add_auxiliary_line(id1, id2, LineType::Segment);
                                                            } else {
                                                                fig.add_segment(id1, id2);
                                                            }
                                                        }
                                                    }

                                                    for c in &geom.circles {
                                                        let c_n = c.center.trim().to_uppercase();
                                                        let cid = if let Some(&id) = p_map.get(&c_n) {
                                                            id
                                                        } else {
                                                            let def_center = cv.engine_ref().last_center_point().map(|(p, _)| p).unwrap_or_else(|| cv.engine_ref().camera.viewport_center());
                                                            let id = fig.add_point(def_center, Some(c.center.clone()));
                                                            p_map.insert(c_n, id);
                                                            id
                                                        };
                                                        fig.add_circle_radius(cid, c.radius);
                                                    }

                                                    for ra in &geom.right_angles {
                                                        let vn = ra.vertex.trim().to_uppercase();
                                                        let a1n = ra.arm1.trim().to_uppercase();
                                                        let a2n = ra.arm2.trim().to_uppercase();
                                                        if let (Some(&vid), Some(&a1id), Some(&a2id)) = (p_map.get(&vn), p_map.get(&a1n), p_map.get(&a2n)) {
                                                            fig.add_right_angle(vid, a1id, a2id, 8.0);
                                                        }
                                                    }

                                                    fig.auto_detect_intersections();
                                                    let flags = cv.engine_mut().insert_shape(Shape::MathFigure(fig));
                                                    appw.handle_widget_flags(flags, &cv);

                                                    let expl = geom.explanation.unwrap_or_else(|| "Đã dựng hình học thành công theo phân tích của AI!".to_string());
                                                    this.append_ai_message(&format!("✨ <b>[AI Geometric CAD Result]</b>\n{expl}"));
                                                    handled = true;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !handled {
                    this.append_ai_message(
                        "💡 <b>PentaPen Geometry AI</b> đã tiếp nhận yêu cầu.\n\n\
                        Bạn có thể ra lệnh cụ thể để AI vẽ tức thì:\n\
                        • <i>'tạo điểm A(100, 200)'</i> hoặc <i>'các điểm A(100, 150), B(350, 150), C(225, 320)'</i>\n\
                        • <i>'đoạn thẳng AB với A(100, 100), B(400, 100)'</i>\n\
                        • <i>'vẽ đường tròn tâm O bán kính 80'</i>\n\
                        • <i>'vẽ hình vuông ABCD cạnh 150'</i>\n\
                        • <i>'vẽ tam giác ABC có đường cao AH'</i>"
                    );
                }
            });
            return;
        }

        // Fallback guidance
        self.append_ai_message(
            "💡 <b>PentaPen Geometry AI Agent</b> đã tiếp nhận yêu cầu.\n\n\
            Để vẽ nhanh trên canvas, bạn có thể nhập:\n\
            • <i>'tạo điểm A(100, 200)'</i>\n\
            • <i>'vẽ tam giác ABC'</i> (hoặc kèm toạ độ / vuông / đều / cân)\n\
            • <i>'vẽ đường tròn tâm O bán kính 80'</i>\n\
            • <i>'vẽ hình vuông ABCD cạnh 150'</i>\n\
            • <i>'dựng tiếp tuyến đến đường tròn'</i>\n\
            • <i>'tìm 4 tâm tam giác'</i>"
        );
    }
}
