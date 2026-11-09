use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

/// Render only: no HTML engine, script execution, or image/file/network loaders.
pub fn show(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    message_id: (usize, usize),
    text: &str,
) {
    let width = ui.available_width();
    ui.push_id(("markdown", message_id), |ui| {
        // Tables may have an intrinsic width larger than the conversation. Keep
        // their overflow inside the answer instead of widening the app shell.
        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_max_width(width);
                ui.style_mut().override_text_style = Some(egui::TextStyle::Body);
                CommonMarkViewer::new()
                    .default_width(Some(width.max(1.0) as usize))
                    .explicit_image_uri_scheme(true)
                    .show(ui, cache, text);
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "# 日本語の回答\n\n段落と **強調**、`inline`、[リンク](https://example.com)。\n\n- 項目\n- 次の項目\n\n> 引用\n\n| 項目 | 値 |\n|---|---|\n| Rust | 正常 |\n\n```rust\nfn main() { println!(\"日本語\"); }\n```\n\n<script>alert('display only')</script>\n";

    fn frame(
        ctx: &egui::Context,
        cache: &mut CommonMarkCache,
        text: &str,
        width: f32,
    ) -> egui::FullOutput {
        frame_with_events(ctx, cache, text, width, vec![])
    }

    fn frame_with_events(
        ctx: &egui::Context,
        cache: &mut CommonMarkCache,
        text: &str,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 540.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, cache, (0, 0), text);
                    assert!(
                        ui.min_rect().width() <= width,
                        "answer widened the viewport"
                    );
                });
            },
        )
    }

    #[test]
    fn mixed_markdown_and_long_tables_stay_inside_narrow_viewport() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let text = format!(
            "{DOCUMENT}\n\n| {} | {} |\n|---|---|\n| data | value |",
            "column".repeat(70),
            "long".repeat(100)
        );
        for width in [320.0, 700.0, 1180.0] {
            let output = frame(&ctx, &mut cache, &text, width);
            assert!(!output.shapes.is_empty());
            assert!(
                output.platform_output.commands.is_empty(),
                "rendering must not open links or execute content"
            );
        }
    }

    #[test]
    fn incremental_unclosed_code_fence_is_renderable() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut text = String::from("回答:\n\n```rust\n");
        for character in "fn main() { println!(\"日本語\"); }\n```".chars() {
            text.push(character);
            frame(&ctx, &mut cache, &text, 700.0);
        }
    }

    #[test]
    fn code_is_highlighted_and_copy_button_copies_only_the_code() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let code = "fn main() { println!(\"hello\"); }";
        let text = format!("説明\n\n```rust\n{code}\n```\n\n補足");
        frame(&ctx, &mut cache, &text, 700.0);
        let output = frame(&ctx, &mut cache, &text, 700.0);
        assert!(
            output.shapes.iter().any(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    t.galley.text().contains("fn main")
                        && t.galley
                            .job
                            .sections
                            .windows(2)
                            .any(|s| s[0].format.color != s[1].format.color)
                } else {
                    false
                }
            }),
            "Rust code must use syntax highlighting"
        );
        let copy = output
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    (t.galley.text() == "🗐").then(|| t.pos + t.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("code block copy action");
        frame_with_events(
            &ctx,
            &mut cache,
            &text,
            700.0,
            vec![
                egui::Event::PointerMoved(copy),
                egui::Event::PointerButton {
                    pos: copy,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let output = frame_with_events(
            &ctx,
            &mut cache,
            &text,
            700.0,
            vec![egui::Event::PointerButton {
                pos: copy,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(output.platform_output.commands.iter().any(|c| matches!(c,
            egui::OutputCommand::CopyText(text) if text == code)));
    }
}
