//! Editor configuration.

use crate::graph::painter::style::PainterTheme;

/// Behavior and appearance settings for a [`Graph`](crate::Graph).
///
/// Start from [`Options::default`] and adjust it with the `with_*` methods,
/// which take the options by value and return the updated copy:
///
/// ```
/// use knobs::options::Options;
///
/// let options = Options::default()
///     .with_multithreading(true)
///     .with_focus_button_enabled(false);
/// ```
///
/// The options are copied into the graph when it is created, so changing them
/// afterwards has no effect on an existing graph.
#[derive(Copy, Clone, Debug)]
pub struct Options {
    pub(crate) multithreading: bool,
    pub(crate) debug_painter: bool,
    pub(crate) show_focus_button: bool,
    pub(crate) theme: PainterTheme,
}

impl Default for Options {
    /// Single threaded evaluation, no debug overlay, focus button shown and the
    /// standard theme.
    fn default() -> Self {
        Self {
            multithreading: false,
            debug_painter: false,
            show_focus_button: true,
            theme: PainterTheme::standard(),
        }
    }
}

impl Options {
    /// Allows nodes to be computed on worker threads.
    ///
    /// Only blueprints whose [`multithreading`] returns `true` get a worker thread
    /// of their own; the others are computed on the evaluator thread. Defaults to
    /// `false`.
    ///
    /// [`multithreading`]: crate::NodeBlueprintDefinition::multithreading
    pub fn with_multithreading(mut self, multithreading: bool) -> Options {
        self.multithreading = multithreading;
        self
    }

    /// Draws the outlines of the rectangles the painter lays out, which helps
    /// when adjusting node and port sizes. Defaults to `false`.
    pub fn with_painter_debug(mut self, debug: bool) -> Options {
        self.debug_painter = debug;
        self
    }

    /// Shows the eye button used to focus a node. Defaults to `true`.
    ///
    /// Focus is what starts evaluation, so hiding the button leaves the user
    /// without a way to evaluate the graph.
    pub fn with_focus_button_enabled(mut self, focus_button_enabled: bool) -> Options {
        self.show_focus_button = focus_button_enabled;
        self
    }

    /// Edits the theme stored in the options through `set_theme`.
    pub fn with_theme_mut<F>(mut self, set_theme: F) -> Options
    where
        F: FnOnce(&mut PainterTheme),
    {
        set_theme(&mut self.theme);
        self
    }
}

#[cfg(test)]
mod tests {
    use egui::Color32;

    use super::*;

    const CUSTOM_BACKGROUND: Color32 = Color32::from_rgb(10, 20, 30);

    #[test]
    fn with_multithreading_enabled_sets_multithreading() {
        let options = Options::default();

        let result = options.with_multithreading(true);

        assert!(result.multithreading);
    }

    #[test]
    fn with_painter_debug_enabled_sets_debug_painter() {
        let options = Options::default();

        let result = options.with_painter_debug(true);

        assert!(result.debug_painter);
    }

    #[test]
    fn with_focus_button_enabled_false_hides_focus_button() {
        let options = Options::default();

        let result = options.with_focus_button_enabled(false);

        assert!(!result.show_focus_button);
    }

    #[test]
    fn with_theme_mut_applies_the_edit_to_the_theme() {
        let options = Options::default();

        let result =
            options.with_theme_mut(|theme| theme.node_background_color = CUSTOM_BACKGROUND);

        assert_eq!(result.theme.node_background_color, CUSTOM_BACKGROUND);
    }
}
