use std::cell::RefCell;
use std::sync::Arc;

use egui::FontFamily::Proportional;
use egui::containers::menu;
use egui::style::StyleModifier;
use egui::{
    Color32, CornerRadius, FontId, Margin, Shadow, Spacing, Stroke, Style as EguiStyle, TextStyle,
    Ui, UiBuilder, Vec2, Visuals, vec2,
};

pub const UNZOOMED: f32 = 1.0;

const SUBHEADING_TEXT_STYLE: &str = "Heading2";
const CONTEXT_TEXT_STYLE: &str = "Context";
const ACTIVE_BORDER_THICKNESS_FACTOR: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemePreset {
    #[default]
    Standard,
}

impl ThemePreset {
    pub const ALL: &'static [Self] = &[Self::Standard];

    pub fn theme(&self) -> PainterTheme {
        match self {
            Self::Standard => PainterTheme::standard(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
        }
    }
}

impl From<ThemePreset> for PainterTheme {
    fn from(preset: ThemePreset) -> Self {
        preset.theme()
    }
}

impl std::fmt::Display for ThemePreset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PainterTheme {
    pub background_color: Color32,

    pub node_header_height: f32,
    pub node_header_margin: f32,
    pub node_padding: f32,
    pub node_background_color: Color32,
    pub node_error_background_color: Color32,
    pub node_border_color: Color32,
    pub node_border_thickness: f32,
    pub node_selected_border_color: Color32,
    pub node_selected_border_thickness: f32,
    pub node_selected_border_distance: f32,
    pub node_highlight_color: Color32,

    pub textbox_background_color: Color32,
    pub textbox_border_color: Color32,
    pub textbox_border_thickness: f32,

    pub font_color: Color32,
    pub font_size: f32,
    pub font_size_ports: f32,
    pub font_size_heading: f32,
    pub font_size_subheading: f32,
    pub font_size_context: f32,
    pub font_size_monospace: f32,
    pub font_size_small: f32,

    pub port_radius: f32,
    pub port_radius_hover: f32,
    pub port_text_margin: f32,
    pub port_spacing: f32,
    pub port_height: f32,

    pub button_corner_radius: f32,
    pub corner_radius: f32,
    pub padding: f32,

    pub temporary_connection_color: Color32,
    pub temporary_connection_thickness: f32,
    pub connection_color: Color32,
    pub connection_thickness: f32,
    pub connection_selected_thickness: f32,

    pub menu_button_size: Vec2,
    pub menu_margin: f32,

    pub panel_background_color: Color32,
    pub panel_padding: f32,

    pub grid_spacing: f32,
    pub grid_color: Color32,
    pub grid_thickness: f32,
}

impl Default for PainterTheme {
    fn default() -> Self {
        ThemePreset::default().theme()
    }
}

impl PainterTheme {
    pub fn standard() -> Self {
        Self {
            background_color: Color32::from_rgb(0, 0, 0),

            node_header_height: 26.0,
            node_header_margin: 14.0,
            node_padding: 6.0,
            node_background_color: Color32::from_rgb(45, 45, 45),
            node_error_background_color: Color32::from_rgb(80, 45, 45),
            node_border_color: Color32::from_rgb(70, 70, 70),
            node_border_thickness: 1.0,
            node_selected_border_color: Color32::WHITE,
            node_selected_border_thickness: 2.0,
            node_selected_border_distance: 3.0,
            node_highlight_color: Color32::from_rgb(45, 75, 110),

            textbox_background_color: Color32::from_rgb(30, 30, 30),
            textbox_border_color: Color32::from_rgb(70, 70, 70),
            textbox_border_thickness: 1.0,

            font_color: Color32::WHITE,
            font_size: 16.0,
            font_size_ports: 12.0,
            font_size_heading: 24.0,
            font_size_subheading: 18.0,
            font_size_context: 16.0,
            font_size_monospace: 10.0,
            font_size_small: 8.0,

            port_radius: 5.0,
            port_radius_hover: 6.0,
            port_text_margin: 12.0,
            port_spacing: 2.0,
            port_height: 22.0,

            button_corner_radius: 2.0,
            corner_radius: 4.0,
            padding: 6.0,

            temporary_connection_color: Color32::YELLOW,
            temporary_connection_thickness: 2.0,
            connection_color: Color32::WHITE,
            connection_thickness: 2.0,
            connection_selected_thickness: 3.0,

            menu_button_size: vec2(180.0, 20.0),
            menu_margin: 4.0,

            panel_background_color: Color32::from_rgb(35, 35, 35),
            panel_padding: 4.0,

            grid_spacing: 16.0,
            grid_color: Color32::from_rgb(45, 45, 45),
            grid_thickness: 1.0,
        }
    }

    pub fn style_modifier(&self, zoom: f32) -> StyleModifier {
        let theme = *self;
        StyleModifier::new(move |style| theme.apply(style, zoom))
    }

    pub fn menu_style_modifier(&self) -> StyleModifier {
        let theme = *self;
        StyleModifier::new(move |style| {
            theme.apply(style, UNZOOMED);
            menu::menu_style(style);
            style.visuals.window_fill = theme.panel_background_color;
        })
    }

    fn apply(&self, style: &mut EguiStyle, zoom: f32) {
        style.visuals = Visuals::dark();
        self.apply_text_styles(style, zoom);
        self.apply_widget_visuals(style, zoom);
        self.apply_window_visuals(style, zoom);
        self.apply_spacing(style, zoom);
    }

    fn apply_text_styles(&self, style: &mut EguiStyle, zoom: f32) {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(self.font_size_heading * zoom, Proportional),
            ),
            (
                TextStyle::Name(SUBHEADING_TEXT_STYLE.into()),
                FontId::new(self.font_size_subheading * zoom, Proportional),
            ),
            (
                TextStyle::Name(CONTEXT_TEXT_STYLE.into()),
                FontId::new(self.font_size_context * zoom, Proportional),
            ),
            (
                TextStyle::Body,
                FontId::new(self.font_size_ports * zoom, Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(self.font_size_monospace * zoom, Proportional),
            ),
            (
                TextStyle::Button,
                FontId::new(self.font_size_ports * zoom, Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(self.font_size_small * zoom, Proportional),
            ),
        ]
            .into();
    }

    fn apply_widget_visuals(&self, style: &mut EguiStyle, zoom: f32) {
        let corner_radius = self.scaled_button_corner_radius(zoom);
        let widgets = &mut style.visuals.widgets;

        for visuals in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            visuals.corner_radius = corner_radius;
            visuals.bg_fill = self.textbox_background_color;
        }

        widgets.inactive.weak_bg_fill = self.textbox_background_color;
        widgets.hovered.weak_bg_fill = self.node_background_color;
        widgets.active.weak_bg_fill = self.node_background_color;
        widgets.open.weak_bg_fill = self.node_background_color;

        widgets.inactive.bg_stroke =
            Stroke::new(self.textbox_border_thickness * zoom, self.node_border_color);
        widgets.hovered.bg_stroke = Stroke::new(
            self.textbox_border_thickness * zoom,
            self.node_selected_border_color,
        );
        widgets.active.bg_stroke = Stroke::new(
            self.textbox_border_thickness * ACTIVE_BORDER_THICKNESS_FACTOR * zoom,
            self.node_selected_border_color,
        );
    }

    fn apply_window_visuals(&self, style: &mut EguiStyle, zoom: f32) {
        style.visuals.window_shadow = Shadow::NONE;
        style.visuals.window_fill = self.background_color;
        style.visuals.window_stroke =
            Stroke::new(self.node_border_thickness * zoom, self.node_border_color);
        style.visuals.menu_corner_radius = self.scaled_corner_radius(zoom);
        style.visuals.override_text_color = Some(self.font_color);
        style.visuals.text_edit_bg_color = Some(self.textbox_background_color);
        style.visuals.selection.bg_fill = self.node_highlight_color;
    }

    fn apply_spacing(&self, style: &mut EguiStyle, zoom: f32) {
        let unzoomed = Spacing::default();
        let spacing = &mut style.spacing;

        spacing.menu_margin = Margin::from(self.menu_margin * zoom);
        spacing.interact_size = (unzoomed.interact_size * zoom).round();
        spacing.button_padding = (unzoomed.button_padding * zoom).round();
        spacing.icon_width = (unzoomed.icon_width * zoom).round();
        spacing.icon_width_inner = (unzoomed.icon_width_inner * zoom).round();
        spacing.icon_spacing = (unzoomed.icon_spacing * zoom).round();
    }

    pub fn scaled_corner_radius(&self, zoom: f32) -> CornerRadius {
        CornerRadius::from(self.corner_radius * zoom)
    }

    pub fn scaled_button_corner_radius(&self, zoom: f32) -> CornerRadius {
        CornerRadius::from(self.button_corner_radius * zoom)
    }

    pub fn scaled_border_stroke(&self, zoom: f32) -> Stroke {
        Stroke::new(self.node_border_thickness * zoom, self.node_border_color)
    }
}

pub struct StylePainter {
    theme: PainterTheme,
    cache: RefCell<Option<CachedStyle>>,
}

struct CachedStyle {
    zoom: f32,
    base: Arc<EguiStyle>,
    zoomed: Arc<EguiStyle>,
}

impl Clone for StylePainter {
    fn clone(&self) -> Self {
        Self::new(self.theme)
    }
}

impl PartialEq for StylePainter {
    fn eq(&self, other: &Self) -> bool {
        self.theme == other.theme
    }
}

impl std::fmt::Debug for StylePainter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StylePainter")
            .field("theme", &self.theme)
            .finish()
    }
}

impl StylePainter {
    pub fn new(theme: impl Into<PainterTheme>) -> Self {
        Self {
            theme: theme.into(),
            cache: RefCell::new(None),
        }
    }

    pub fn theme(&self) -> &PainterTheme {
        &self.theme
    }

    pub fn scoped<R>(&self, ui: &mut Ui, zoom: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
        let zoomed = self.zoomed_style(ui.style(), zoom);

        ui.scope_builder(UiBuilder::new().style(zoomed), add_contents)
            .inner
    }

    pub fn zoomed_style(&self, base: &Arc<EguiStyle>, zoom: f32) -> Arc<EguiStyle> {
        let hit = self.cache.borrow().as_ref().and_then(|cached| {
            (cached.zoom == zoom && Arc::ptr_eq(&cached.base, base)).then(|| cached.zoomed.clone())
        });

        if let Some(zoomed) = hit {
            return zoomed;
        }

        let mut zoomed = (**base).clone();
        self.theme.style_modifier(zoom).apply(&mut zoomed);
        let zoomed = Arc::new(zoomed);

        *self.cache.borrow_mut() = Some(CachedStyle {
            zoom,
            base: base.clone(),
            zoomed: zoomed.clone(),
        });

        zoomed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_returns_standard_preset() {
        let standard = ThemePreset::Standard.theme();

        let theme = PainterTheme::default();

        assert_eq!(theme, standard);
    }

    #[test]
    fn from_every_preset_returns_its_theme() {
        for preset in ThemePreset::ALL {
            let expected = preset.theme();

            let theme = PainterTheme::from(*preset);

            assert_eq!(theme, expected);
        }
    }

    #[test]
    fn name_of_every_preset_is_not_empty() {
        for preset in ThemePreset::ALL {
            let name = preset.name();

            assert!(!name.is_empty());
        }
    }

    #[test]
    fn style_modifier_with_zoom_scales_font_sizes() {
        let theme = PainterTheme::default();
        let zoom = 2.0;
        let mut style = EguiStyle::default();

        theme.style_modifier(zoom).apply(&mut style);

        assert_eq!(
            style.text_styles[&TextStyle::Body].size,
            theme.font_size_ports * zoom
        );
        assert_eq!(
            style.text_styles[&TextStyle::Heading].size,
            theme.font_size_heading * zoom
        );
    }

    #[test]
    fn style_modifier_with_zoom_scales_stroke_widths() {
        let theme = PainterTheme::default();
        let zoom = 3.0;
        let mut style = EguiStyle::default();

        theme.style_modifier(zoom).apply(&mut style);

        assert_eq!(
            style.visuals.window_stroke.width,
            theme.node_border_thickness * zoom
        );
    }

    #[test]
    fn style_modifier_with_zoom_scales_inline_spacing() {
        let theme = PainterTheme::default();
        let zoom = 2.0;
        let unzoomed = Spacing::default();
        let mut style = EguiStyle::default();

        theme.style_modifier(zoom).apply(&mut style);

        assert_eq!(style.spacing.interact_size, unzoomed.interact_size * zoom);
        assert_eq!(style.spacing.button_padding, unzoomed.button_padding * zoom);
        assert_eq!(style.spacing.icon_width, unzoomed.icon_width * zoom);
        assert_eq!(
            style.spacing.icon_width_inner,
            unzoomed.icon_width_inner * zoom
        );
        assert_eq!(style.spacing.icon_spacing, unzoomed.icon_spacing * zoom);
    }

    #[test]
    fn style_modifier_applied_twice_does_not_compound_spacing() {
        let theme = PainterTheme::default();
        let zoom = 2.0;
        let mut once = EguiStyle::default();
        theme.style_modifier(zoom).apply(&mut once);
        let mut twice = once.clone();

        theme.style_modifier(zoom).apply(&mut twice);

        assert_eq!(twice.spacing.icon_width, once.spacing.icon_width);
        assert_eq!(twice.spacing.button_padding, once.spacing.button_padding);
    }

    #[test]
    fn style_modifier_leaves_unowned_base_fields_untouched() {
        let theme = PainterTheme::default();
        let animation_time = 0.42;
        let mut style = EguiStyle {
            animation_time,
            ..EguiStyle::default()
        };

        theme.style_modifier(UNZOOMED).apply(&mut style);

        assert_eq!(style.animation_time, animation_time);
    }

    #[test]
    fn menu_style_modifier_clears_button_background_strokes() {
        let theme = PainterTheme::default();
        let mut style = EguiStyle::default();

        theme.menu_style_modifier().apply(&mut style);

        assert_eq!(style.visuals.widgets.hovered.bg_stroke, Stroke::NONE);
        assert_eq!(style.visuals.widgets.active.bg_stroke, Stroke::NONE);
    }

    #[test]
    fn menu_style_modifier_uses_panel_background_for_popups() {
        let theme = PainterTheme::default();
        let mut style = EguiStyle::default();

        theme.menu_style_modifier().apply(&mut style);

        assert_eq!(style.visuals.window_fill, theme.panel_background_color);
    }

    #[test]
    fn scaled_border_stroke_multiplies_thickness_by_zoom() {
        let theme = PainterTheme::default();
        let zoom = 2.5;

        let stroke = theme.scaled_border_stroke(zoom);

        assert_eq!(stroke.width, theme.node_border_thickness * zoom);
        assert_eq!(stroke.color, theme.node_border_color);
    }

    #[test]
    fn zoomed_style_with_unchanged_inputs_reuses_the_cached_style() {
        let painter = StylePainter::new(ThemePreset::Standard);
        let base = Arc::new(EguiStyle::default());
        let zoom = 1.5;

        let first = painter.zoomed_style(&base, zoom);
        let second = painter.zoomed_style(&base, zoom);

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn zoomed_style_with_changed_zoom_rebuilds_the_style() {
        let painter = StylePainter::new(ThemePreset::Standard);
        let base = Arc::new(EguiStyle::default());
        let first = painter.zoomed_style(&base, 1.0);

        let second = painter.zoomed_style(&base, 2.0);

        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn zoomed_style_with_changed_base_rebuilds_the_style() {
        let painter = StylePainter::new(ThemePreset::Standard);
        let zoom = 1.0;
        let first = painter.zoomed_style(&Arc::new(EguiStyle::default()), zoom);

        let second = painter.zoomed_style(&Arc::new(EguiStyle::default()), zoom);

        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn new_with_preset_uses_the_preset_theme() {
        let preset = ThemePreset::Standard;

        let painter = StylePainter::new(preset);

        assert_eq!(*painter.theme(), preset.theme());
    }

    #[test]
    fn new_with_custom_theme_uses_that_theme() {
        let theme = PainterTheme {
            node_background_color: Color32::from_rgb(10, 20, 30),
            ..PainterTheme::standard()
        };

        let painter = StylePainter::new(theme);

        assert_eq!(*painter.theme(), theme);
    }
}
