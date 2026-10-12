use eframe::egui::{self, Color32};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default = "default_dark")]
    pub dark: bool,
    pub ui: UiColors,
    pub syntax: SyntaxColors,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UiColors {
    pub background: HexColor,
    pub panel: HexColor,
    pub editor: HexColor,
    #[serde(default)]
    pub title_bar: Option<HexColor>,
    #[serde(default)]
    pub activity_bar: Option<HexColor>,
    pub text: HexColor,
    pub muted: HexColor,
    pub accent: HexColor,
    pub selection: HexColor,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SyntaxColors {
    pub plain: HexColor,
    pub keyword: HexColor,
    pub number: HexColor,
    pub string: HexColor,
    pub comment: HexColor,
    pub doc_comment: HexColor,
    pub identifier: HexColor,
    pub operator: HexColor,
    pub delimiter: HexColor,
    pub render_marker: HexColor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HexColor(pub Color32);

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        parse_hex(&value)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

fn default_dark() -> bool {
    true
}

fn parse_hex(value: &str) -> Result<Color32, String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 && hex.len() != 8 {
        return Err(format!(
            "color `{value}` must contain 6 or 8 hexadecimal digits"
        ));
    }
    let component = |start| {
        u8::from_str_radix(&hex[start..start + 2], 16)
            .map_err(|_| format!("invalid hexadecimal color `{value}`"))
    };
    let red = component(0)?;
    let green = component(2)?;
    let blue = component(4)?;
    let alpha = if hex.len() == 8 { component(6)? } else { 255 };
    Ok(Color32::from_rgba_unmultiplied(red, green, blue, alpha))
}

impl Theme {
    pub fn title_bar_color(&self) -> Color32 {
        self.ui.title_bar.map_or(self.ui.panel.0, |color| color.0)
    }

    pub fn activity_bar_color(&self) -> Color32 {
        self.ui
            .activity_bar
            .map_or(self.ui.background.0, |color| color.0)
    }

    fn bundled(source: &str) -> Self {
        toml::from_str(source).expect("bundled themes must be valid")
    }

    pub fn bundled_themes() -> Vec<Self> {
        vec![
            Self::bundled(include_str!("../../../themes/dark-modern.toml")),
            Self::bundled(include_str!("../../../themes/light-modern.toml")),
            Self::bundled(include_str!("../../../themes/one-dark-pro.toml")),
            Self::bundled(include_str!("../../../themes/dracula.toml")),
            Self::bundled(include_str!("../../../themes/tokyo-night.toml")),
            Self::bundled(include_str!("../../../themes/catppuccin-mocha.toml")),
            Self::bundled(include_str!("../../../themes/nord.toml")),
            Self::bundled(include_str!("../../../themes/github-dark.toml")),
            Self::bundled(include_str!("../../../themes/solarized-dark.toml")),
            Self::bundled(include_str!("../../../themes/monokai.toml")),
            Self::bundled(include_str!("../../../themes/gruber-darker.toml")),
            Self::bundled(include_str!("../../../themes/paper-light.toml")),
        ]
    }

    pub fn contrast_on_accent(&self) -> Color32 {
        let accent = self.ui.accent.0;
        let luminance = 0.2126 * f32::from(accent.r())
            + 0.7152 * f32::from(accent.g())
            + 0.0722 * f32::from(accent.b());
        if luminance > 150.0 {
            Color32::from_rgb(24, 24, 24)
        } else {
            Color32::WHITE
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        toml::from_str(&source)
            .map_err(|error| format!("invalid theme {}: {error}", path.display()))
    }

    pub fn apply(&self, context: &egui::Context) {
        let mut style = (*context.style()).clone();
        style.visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        style.visuals.panel_fill = self.ui.panel.0;
        style.visuals.window_fill = self.ui.panel.0;
        style.visuals.extreme_bg_color = self.ui.editor.0;
        style.visuals.faint_bg_color = self.ui.background.0;
        style.visuals.override_text_color = Some(self.ui.text.0);
        style.visuals.selection.bg_fill = self.ui.selection.0;
        style.visuals.selection.stroke.color = self.ui.text.0;
        style.visuals.hyperlink_color = self.ui.accent.0;
        style.visuals.widgets.noninteractive.bg_fill = self.ui.panel.0;
        style.visuals.widgets.noninteractive.fg_stroke.color = self.ui.text.0;
        style.visuals.widgets.inactive.bg_fill = self.ui.panel.0;
        style.visuals.widgets.inactive.fg_stroke.color = self.ui.text.0;
        style.visuals.widgets.inactive.rounding = egui::Rounding::same(2.0);
        style.visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, self.ui.muted.0.gamma_multiply(0.28));
        style.visuals.widgets.hovered.bg_fill = self.ui.selection.0;
        style.visuals.widgets.hovered.fg_stroke.color = self.ui.text.0;
        style.visuals.widgets.hovered.rounding = egui::Rounding::same(2.0);
        style.visuals.widgets.hovered.expansion = 0.0;
        style.visuals.widgets.active.bg_fill = self.ui.accent.0;
        style.visuals.widgets.active.fg_stroke.color = self.contrast_on_accent();
        style.visuals.widgets.active.rounding = egui::Rounding::same(2.0);
        style.visuals.window_rounding = egui::Rounding::same(4.0);
        style.visuals.window_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 6.0),
            blur: 18.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(80),
        };
        style.visuals.panel_fill = self.ui.panel.0;
        style.visuals.faint_bg_color = self.ui.background.0;
        style.visuals.popup_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 8.0),
            blur: 24.0,
            spread: 2.0,
            color: egui::Color32::from_black_alpha(95),
        };
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
        style.spacing.window_margin = egui::Margin::same(12.0);
        style.spacing.combo_width = 210.0;
        style.spacing.indent = 14.0;
        style.spacing.interact_size.y = 24.0;
        style.animation_time = 0.16;
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(16.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(11.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(13.0, egui::FontFamily::Monospace),
        );
        context.set_style(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_themes_parse() {
        let themes = Theme::bundled_themes();
        assert_eq!(themes.len(), 12);
        assert_eq!(themes[0].name, "Dark Modern");
        assert!(themes.iter().any(|theme| theme.name == "Dark Modern"));
        assert!(themes
            .iter()
            .any(|theme| theme.name == "Light Modern" && !theme.dark));
    }

    #[test]
    fn accepts_rgb_and_rgba_colors() {
        assert_eq!(
            parse_hex("#ff0080").unwrap(),
            Color32::from_rgb(255, 0, 128)
        );
        assert_eq!(
            parse_hex("11223344").unwrap(),
            Color32::from_rgba_unmultiplied(17, 34, 51, 68)
        );
    }
}
