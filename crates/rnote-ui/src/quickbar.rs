//! Quick bar next to the color picker: one tap for the geometry tools, dashed lines,
//! board backgrounds and function graphs, instead of digging through the sidebar and settings.

use crate::RnAppWindow;
use gtk4::{
    Box as GtkBox, Button, MenuButton, Orientation, Popover, ToggleButton, gdk, glib, prelude::*,
};
use rnote_compose::Color;
use rnote_engine::document::background::PatternStyle;
use rnote_engine::ext::GdkRGBAExt;
use rnote_engine::graph::Function;

pub(crate) fn build(appwindow: &RnAppWindow, bar: &GtkBox) {
    // Geometry tools (the win.math-tool action also switches to the shaper)
    let tools = [
        ("shapebuilder-mathpoint-symbolic", "point", "Điểm (P)"),
        (
            "shapebuilder-mathline-symbolic",
            "line",
            "Đoạn thẳng có điểm (L)",
        ),
        (
            "shapebuilder-line-symbolic",
            "line-nopoints",
            "Đoạn thẳng không hiện điểm",
        ),
        (
            "shapebuilder-mathcircle-symbolic",
            "circle",
            "Đường tròn (C)",
        ),
        (
            "shapebuilder-mathtriangle-symbolic",
            "triangle",
            "Tam giác (T)",
        ),
        (
            "shapebuilder-rectangle-symbolic",
            "rectangle",
            "Hình chữ nhật (S)",
        ),
        (
            "shapebuilder-coordsystem2d-symbolic",
            "coords",
            "Hệ trục toạ độ",
        ),
    ];
    for (icon, target, tip) in tools {
        let b = Button::builder()
            .icon_name(icon)
            .tooltip_text(tip)
            .action_name("win.math-tool")
            .action_target(&target.to_variant())
            .build();
        b.add_css_class("flat");
        bar.append(&b);
    }

    bar.append(&gtk4::Separator::new(Orientation::Vertical));
    bar.append(&dashed_toggle(appwindow));
    bar.append(&graph_button(appwindow));
    bar.append(&background_button(appwindow));
}

/// Solid <-> dashed for the next shapes (all shaper shapes, math figures included).
fn dashed_toggle(appwindow: &RnAppWindow) -> ToggleButton {
    use rnote_compose::style::smooth::LineStyle;
    let t = ToggleButton::builder()
        .label("╌ ╌")
        .tooltip_text("Nét đứt cho hình vẽ tiếp theo (chọn một hình rồi bấm D để đổi hình có sẵn)")
        .build();
    t.add_css_class("flat");
    t.set_active(
        appwindow
            .engine_config()
            .read()
            .pens_config
            .shaper_config
            .smooth_options
            .line_style
            != LineStyle::Solid,
    );
    let w = appwindow.downgrade();
    t.connect_toggled(move |t| {
        let Some(appwindow) = w.upgrade() else { return };
        appwindow
            .engine_config()
            .write()
            .pens_config
            .shaper_config
            .smooth_options
            .line_style = if t.is_active() {
            LineStyle::DashedEquidistant
        } else {
            LineStyle::Solid
        };
        appwindow
            .overlays()
            .penssidebar()
            .shaper_page()
            .refresh_ui(&appwindow);
    });
    t
}

fn labeled_spin(row: &GtkBox, label: &str, min: f64, max: f64, value: f64) -> gtk4::SpinButton {
    row.append(&gtk4::Label::new(Some(label)));
    let s = gtk4::SpinButton::with_range(min, max, 1.0);
    s.set_value(value);
    s.set_width_chars(4);
    row.append(&s);
    s
}

/// f(x): typed formula or pasted from MathType (LaTeX / MathML), drawn with axes and,
/// optionally, its variation table.
fn graph_button(appwindow: &RnAppWindow) -> MenuButton {
    let content = GtkBox::new(Orientation::Vertical, 8);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);
    content.set_width_request(330);

    let title = gtk4::Label::new(Some("Vẽ đồ thị hàm số y = f(x)"));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    content.append(&title);

    let entry = gtk4::Entry::builder()
        .placeholder_text("vd: x^3 - 3x + 1   hoặc   (2x+1)/(x-1)")
        .build();
    content.append(&entry);
    let hint = gtk4::Label::new(Some(
        "Có thể dán công thức từ MathType (LaTeX hoặc MathML). Hỗ trợ: + − × / ^, √ sqrt, |x|, sin cos tan ln log exp, π.",
    ));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    content.append(&hint);

    let row1 = GtkBox::new(Orientation::Horizontal, 6);
    let x_min = labeled_spin(&row1, "x từ", -100.0, 100.0, -5.0);
    let x_max = labeled_spin(&row1, "đến", -100.0, 100.0, 5.0);
    content.append(&row1);
    let row2 = GtkBox::new(Orientation::Horizontal, 6);
    let y_min = labeled_spin(&row2, "y từ", -100.0, 100.0, -5.0);
    let y_max = labeled_spin(&row2, "đến", -100.0, 100.0, 5.0);
    content.append(&row2);
    let row3 = GtkBox::new(Orientation::Horizontal, 6);
    let unit = labeled_spin(&row3, "1 đơn vị =", 10.0, 200.0, 40.0);
    row3.append(&gtk4::Label::new(Some("px")));
    content.append(&row3);

    let with_table = gtk4::CheckButton::with_label("Vẽ kèm bảng biến thiên");
    with_table.set_active(true);
    content.append(&with_table);

    let draw = Button::with_label("Vẽ đồ thị");
    draw.add_css_class("suggested-action");
    content.append(&draw);

    let popover = Popover::builder().child(&content).build();
    let mb = MenuButton::builder()
        .label("f(x)")
        .tooltip_text("Vẽ đồ thị hàm số, bảng biến thiên")
        .popover(&popover)
        .build();
    mb.add_css_class("flat");

    let w = appwindow.downgrade();
    let go = glib::clone!(
        #[weak]
        popover,
        #[weak]
        entry,
        move || {
            let Some(appwindow) = w.upgrade() else { return };
            let Some(canvas) = appwindow.active_tab_canvas() else {
                return;
            };
            let f = match Function::parse(&entry.text()) {
                Ok(f) => f,
                Err(e) => {
                    appwindow.overlays().dispatch_toast_text(&e, None);
                    return;
                }
            };
            let (x0, x1) = (x_min.value().min(x_max.value() - 1.0), x_max.value());
            let (y0, y1) = (y_min.value().min(y_max.value() - 1.0), y_max.value());
            let u = unit.value();
            let center = canvas.engine_ref().camera.viewport_center();
            let mut flags =
                canvas
                    .engine_mut()
                    .insert_function_graph(&f, center, u, (x0, x1), (y0, y1));
            if with_table.is_active() {
                let top_left = center
                    + p2d::math::Vector2::new(0.5 * (x1 - x0) * u + 60.0, -0.5 * (y1 - y0) * u);
                match canvas.engine_mut().insert_variation_table(&f, top_left) {
                    Ok(f) => flags |= f,
                    Err(e) => {
                        appwindow.overlays().dispatch_toast_text(&e, None);
                    }
                }
            }
            appwindow.handle_widget_flags(flags, &canvas);
            popover.popdown();
        }
    );
    let go2 = go.clone();
    entry.connect_activate(move |_| go2());
    draw.connect_clicked(move |_| go());
    mb
}

fn rgba(r: f32, g: f32, b: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, 1.0)
}

/// Board presets and patterns in one click (they used to be deep in the settings).
fn background_button(appwindow: &RnAppWindow) -> MenuButton {
    let content = GtkBox::new(Orientation::Vertical, 8);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(10);
    content.set_margin_end(10);

    let title = gtk4::Label::new(Some("Nền bảng"));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    content.append(&title);

    // (name, board color, ink color): a dark board switches the ink to chalk white
    let presets = [
        ("Bảng trắng", rgba(1.0, 1.0, 1.0), rgba(0.0, 0.0, 0.0)),
        ("Giấy kem", rgba(0.99, 0.97, 0.90), rgba(0.0, 0.0, 0.0)),
        ("Bảng xanh", rgba(0.13, 0.29, 0.22), rgba(1.0, 1.0, 1.0)),
        ("Bảng đen", rgba(0.11, 0.12, 0.13), rgba(1.0, 1.0, 1.0)),
    ];
    let row = GtkBox::new(Orientation::Horizontal, 4);
    row.add_css_class("linked");
    for (name, board, ink) in presets {
        let b = Button::with_label(name);
        let w = appwindow.downgrade();
        b.connect_clicked(move |_| {
            if let Some(appwindow) = w.upgrade() {
                set_background(&appwindow, Some(board.into_compose_color()), None);
                appwindow.overlays().colorpicker().set_stroke_color(ink);
            }
        });
        row.append(&b);
    }
    content.append(&row);

    let patterns = [
        ("Trơn", PatternStyle::None),
        ("Dòng kẻ", PatternStyle::Lines),
        ("Ô vuông", PatternStyle::Grid),
        ("Chấm", PatternStyle::Dots),
    ];
    let row = GtkBox::new(Orientation::Horizontal, 4);
    row.add_css_class("linked");
    for (name, pattern) in patterns {
        let b = Button::with_label(name);
        let w = appwindow.downgrade();
        b.connect_clicked(move |_| {
            if let Some(appwindow) = w.upgrade() {
                set_background(&appwindow, None, Some(pattern));
            }
        });
        row.append(&b);
    }
    content.append(&row);

    let custom = GtkBox::new(Orientation::Horizontal, 6);
    custom.append(&gtk4::Label::new(Some("Màu khác")));
    let color_button = gtk4::ColorDialogButton::new(Some(gtk4::ColorDialog::new()));
    let w = appwindow.downgrade();
    color_button.connect_rgba_notify(move |b| {
        if let Some(appwindow) = w.upgrade() {
            set_background(&appwindow, Some(b.rgba().into_compose_color()), None);
        }
    });
    custom.append(&color_button);
    content.append(&custom);

    let popover = Popover::builder().child(&content).build();
    let mb = MenuButton::builder()
        .label("Nền")
        .tooltip_text("Đổi nền bảng: màu bảng, dòng kẻ, ô vuông")
        .popover(&popover)
        .build();
    mb.add_css_class("flat");
    mb
}

/// Change the board color and/or pattern of the active document; the pattern color follows
/// the board brightness so lines stay faint on both light and dark boards.
fn set_background(appwindow: &RnAppWindow, color: Option<Color>, pattern: Option<PatternStyle>) {
    let Some(canvas) = appwindow.active_tab_canvas() else {
        return;
    };
    {
        let mut engine = canvas.engine_mut();
        let bg = &mut engine.document.config.background;
        if let Some(c) = color {
            bg.color = c;
        }
        if let Some(p) = pattern {
            bg.pattern = p;
        }
        let light = bg.color.r + bg.color.g + bg.color.b > 1.5;
        bg.pattern_color = if light {
            Color::new(0.6, 0.72, 0.85, 1.0)
        } else {
            Color::new(1.0, 1.0, 1.0, 0.28)
        };
    }
    let mut widget_flags = canvas.engine_mut().background_rendering_regenerate();
    widget_flags.store_modified = true;
    appwindow.handle_widget_flags(widget_flags, &canvas);
    appwindow.sidebar().settings_panel().refresh_ui(appwindow);
}
