//! Small dialogs for input the shaper asks for with a tap: the name of a point put on a line,
//! the radius of a circle around a tapped center.

use crate::{RnAppWindow, RnCanvas};
use adw::prelude::*;
use gtk4::{Box as GtkBox, Entry, Label, Orientation, SpinButton};
use rnote_engine::pens::shaper::ShaperPrompt;

fn row(parent: &GtkBox, label: &str, widget: &impl IsA<gtk4::Widget>) {
    let r = GtkBox::new(Orientation::Horizontal, 8);
    let l = Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_width_chars(10);
    r.append(&l);
    widget.set_hexpand(true);
    r.append(widget);
    parent.append(&r);
}

fn name_entry(text: &str) -> Entry {
    let e = Entry::builder()
        .text(text)
        .activates_default(true)
        .max_length(8)
        .build();
    e.select_region(0, -1);
    e
}

/// Ask for whatever the last shaper tap wanted, then apply it as one undo step.
pub(crate) fn show(appwindow: &RnAppWindow, canvas: &RnCanvas) {
    let Some(prompt) = canvas.engine_mut().take_shaper_prompt() else {
        return;
    };
    let content = GtkBox::new(Orientation::Vertical, 8);
    let dialog = adw::AlertDialog::builder()
        .extra_child(&content)
        .default_response("ok")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", "Huỷ"), ("ok", "Đồng ý")]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);

    match prompt {
        ShaperPrompt::PointOnLine { key, line, pos } => {
            dialog.set_heading(Some("Điểm trên đường thẳng"));
            dialog.set_body("Điểm mới nằm trên đường thẳng và đi theo khi đường thẳng di chuyển.");
            let entry = name_entry(&canvas.engine_ref().suggest_point_label());
            row(&content, "Tên điểm", &entry);
            let (appwindow, canvas) = (appwindow.clone(), canvas.clone());
            dialog.connect_response(Some("ok"), move |_, _| {
                let label = entry.text().trim().to_string();
                let flags = canvas
                    .engine_mut()
                    .add_point_on_line(key, line, pos, Some(label));
                appwindow.handle_widget_flags(flags, &canvas);
            });
        }
        ShaperPrompt::CircleRadius { center, on } => {
            dialog.set_heading(Some("Đường tròn theo bán kính"));
            let name = (on.is_none()).then(|| {
                let configured = appwindow
                    .engine_config()
                    .read()
                    .pens_config
                    .shaper_config
                    .math_center_label
                    .clone();
                let engine = canvas.engine_ref();
                let used = engine
                    .named_figure_points()
                    .iter()
                    .any(|(n, _)| *n == configured);
                let entry = name_entry(&if used {
                    engine.suggest_point_label()
                } else {
                    configured
                });
                row(&content, "Tên tâm", &entry);
                entry
            });
            let radius = SpinButton::with_range(0.1, 100.0, 0.5);
            radius.set_digits(1);
            radius.set_value(3.0);
            radius.set_activates_default(true);
            row(&content, "Bán kính (cm)", &radius);
            let (appwindow, canvas) = (appwindow.clone(), canvas.clone());
            dialog.connect_response(Some("ok"), move |_, _| {
                let dpi = canvas.engine_ref().document.config.format.dpi();
                let r_px = radius.value() / 2.54 * dpi;
                let label = name.as_ref().map(|e| e.text().trim().to_string());
                let flags = canvas
                    .engine_mut()
                    .add_circle_with_radius(center, on, label, r_px);
                appwindow.handle_widget_flags(flags, &canvas);
            });
        }
    }
    dialog.present(Some(appwindow));
}
