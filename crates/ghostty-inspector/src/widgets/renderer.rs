//! Renderer info window (ZIG/src/inspector/widgets/renderer.zig).

use dear_imgui_rs::{ColorButtonFlags, Condition, TreeNodeFlags, Ui};
use ghostty_renderer::overlay::{Color, Feature};

use super::help_marker;

/// Renderer information inspector widget.
#[derive(Debug, Default)]
pub struct Info {
    /// The enabled overlay features, in the order they were enabled (Zig:
    /// an array hash map keyed by feature tag).
    pub features: Vec<Feature>,
}

impl Info {
    /// Grab the features into a new allocated vec.
    pub fn overlay_features(&self) -> Vec<Feature> {
        self.features.clone()
    }

    fn set_feature(&mut self, feature: Feature, enabled: bool) {
        let pos = self.features.iter().position(|f| *f == feature);
        match (enabled, pos) {
            (false, Some(i)) => {
                self.features.remove(i);
            }
            (true, None) => self.features.push(feature),
            _ => {}
        }
    }

    /// Draw the renderer info window.
    pub fn draw(&mut self, ui: &Ui, open: bool) {
        if !open {
            return;
        }

        ui.set_next_item_open_with_cond(true, Condition::Once);
        if !ui.collapsing_header("Overlays", TreeNodeFlags::NONE) {
            return;
        }

        ui.separator_with_text("Hyperlinks");
        self.overlay_hyperlinks(ui);
        ui.separator_with_text("Semantic Prompts");
        self.overlay_semantic_prompts(ui);
    }

    fn overlay_hyperlinks(&mut self, ui: &Ui) {
        let mut hyperlinks = self.features.contains(&Feature::HighlightHyperlinks);
        ui.checkbox("Overlay Hyperlinks", &mut hyperlinks);
        ui.same_line();
        help_marker(ui, "When enabled, highlights OSC8 hyperlinks.");

        self.set_feature(Feature::HighlightHyperlinks, hyperlinks);
    }

    fn overlay_semantic_prompts(&mut self, ui: &Ui) {
        let mut semantic_prompts = self.features.contains(&Feature::SemanticPrompts);
        ui.checkbox("Overlay Semantic Prompts", &mut semantic_prompts);
        ui.same_line();
        help_marker(ui, "When enabled, highlights OSC 133 semantic prompts.");

        // Handle the checkbox results
        self.set_feature(Feature::SemanticPrompts, semantic_prompts);

        // Help
        ui.indent();

        ui.text_disabled("Colors:");

        let to_col = |c: Color| {
            let rgb = c.rgb();
            [
                f32::from(rgb[0]) / 255.0,
                f32::from(rgb[1]) / 255.0,
                f32::from(rgb[2]) / 255.0,
                1.0,
            ]
        };

        ui.color_button_config("##prompt_color", to_col(Color::SemanticPrompt))
            .flags(ColorButtonFlags::NO_TOOLTIP)
            .build();
        ui.same_line();
        ui.text("Prompt");

        ui.color_button_config("##input_color", to_col(Color::SemanticInput))
            .flags(ColorButtonFlags::NO_TOOLTIP)
            .build();
        ui.same_line();
        ui.text("Input");

        ui.unindent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_toggle() {
        let mut info = Info::default();
        info.set_feature(Feature::HighlightHyperlinks, true);
        info.set_feature(Feature::HighlightHyperlinks, true);
        info.set_feature(Feature::SemanticPrompts, true);
        assert_eq!(
            info.overlay_features(),
            vec![Feature::HighlightHyperlinks, Feature::SemanticPrompts]
        );
        info.set_feature(Feature::HighlightHyperlinks, false);
        assert_eq!(info.overlay_features(), vec![Feature::SemanticPrompts]);
    }
}
