use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window, WindowAppearance, rgb};

/// Call after `gpui_kit::init` so the component theme registry exists.
pub fn init(cx: &mut App) {
    Theme::sync_system_appearance(None, cx);
    apply_orange(cx);
}

pub fn apply_orange(cx: &mut App) {
    // Theme::update also reconciles component tokens and the Base projection.
    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.primary = rgb(0xc2410c).into();
        colors.primary_hover = rgb(0x9a3412).into();
        colors.primary_active = rgb(0x7c2d12).into();
        colors.primary_foreground = rgb(0xffffff).into();
        colors.ring = colors.primary;
        // Buttons have independent resolved tokens in Kit 0.8.1.
        colors.button_primary = colors.primary;
        colors.button_primary_hover = colors.primary_hover;
        colors.button_primary_active = colors.primary_active;
        colors.button_primary_foreground = colors.primary_foreground;
    });
}

/// `None` follows the supplied window's appearance, or the system before opening one.
pub fn change_theme(mode: Option<ThemeMode>, window: Option<&mut Window>, cx: &mut App) {
    cx.set_window_appearance(mode.map(native_appearance));
    match mode {
        Some(mode) => Theme::change(mode, window, cx),
        // A window's appearance is cached until its change event arrives.
        None => Theme::change(cx.window_appearance(), window, cx),
    }
    apply_orange(cx);
}

fn native_appearance(mode: ThemeMode) -> WindowAppearance {
    match mode {
        ThemeMode::Light => WindowAppearance::Light,
        ThemeMode::Dark => WindowAppearance::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_chrome_matches_content_mode() {
        assert_eq!(native_appearance(ThemeMode::Light), WindowAppearance::Light);
        assert_eq!(native_appearance(ThemeMode::Dark), WindowAppearance::Dark);
    }
}
