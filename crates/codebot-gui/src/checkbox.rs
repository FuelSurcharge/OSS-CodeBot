use eframe::egui::{self, Response, Stroke, Ui, Widget, WidgetText};

/// Keep egui's checkbox interaction and accessibility, with a ring and filled dot.
pub struct Checkbox<'a> {
    checked: &'a mut bool,
    text: WidgetText,
}

impl<'a> Checkbox<'a> {
    pub fn new(checked: &'a mut bool, text: impl Into<WidgetText>) -> Self {
        Self {
            checked,
            text: text.into(),
        }
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self { checked, text } = self;
        let text = text.color(ui.visuals().text_color());
        let response = ui
            .scope(|ui| {
                // Only replace the indicator painting. egui still handles the
                // label layout, hit area, focus, keyboard, and checked state.
                let widgets = &mut ui.visuals_mut().widgets;
                for visual in [
                    &mut widgets.noninteractive,
                    &mut widgets.inactive,
                    &mut widgets.hovered,
                    &mut widgets.active,
                    &mut widgets.open,
                ] {
                    visual.bg_fill = egui::Color32::TRANSPARENT;
                    visual.bg_stroke = Stroke::NONE;
                    visual.fg_stroke = Stroke::NONE;
                }
                ui.add(egui::Checkbox::new(&mut *checked, text))
            })
            .inner;

        if ui.is_rect_visible(response.rect) {
            let visual = ui.style().interact(&response);
            let (_, icon) = ui.spacing().icon_rectangles(response.rect);
            if *checked {
                let radius = icon.width() / 2.0 + visual.expansion;
                let stroke = Stroke::new(1.5_f32, crate::ACCENT);
                let fill = if response.hovered() || response.has_focus() {
                    crate::SELECTED
                } else {
                    crate::SURFACE
                };
                ui.painter().circle_filled(icon.center(), radius, fill);
                ui.painter().circle_stroke(icon.center(), radius, stroke);
                ui.painter()
                    .circle_filled(icon.center(), radius * 0.45, crate::ACCENT);
            } else {
                ui.painter().rect(
                    icon.expand(visual.expansion),
                    visual.corner_radius,
                    visual.bg_fill,
                    visual.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        response
    }
}
