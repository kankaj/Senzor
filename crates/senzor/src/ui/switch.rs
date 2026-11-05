use eframe::egui;

/// An on/off switch: a pill with a knob that slides right when on. Clicking it flips `on`.
/// egui has no switch widget type, so screen readers and tests know it as a checkbox named
/// `label`.
pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label)
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact_selectable(&response, *on);
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();
        ui.painter().rect(
            rect,
            radius,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let knob = egui::pos2(knob_x, rect.center().y);
        ui.painter()
            .circle(knob, 0.75 * radius, visuals.bg_fill, visuals.fg_stroke);
    }
    response
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn clicking_the_switch_turns_it_on_and_off() {
        let mut h = Harness::new_ui_state(
            |ui, on: &mut bool| {
                switch(ui, on, "Power");
            },
            false,
        );
        h.get_by_label("Power").click();
        h.run();
        assert!(*h.state());
        h.get_by_label("Power").click();
        h.run();
        assert!(!*h.state());
    }
}
