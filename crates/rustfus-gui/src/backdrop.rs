// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rustfus contributors
//! Native window material with tinted cards and an opaque fallback.

use gpui_kit::{App, Hsla, Styled, Window, WindowBackgroundAppearance};

pub(crate) const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

pub(crate) fn appearance(enabled: bool) -> WindowBackgroundAppearance {
    if enabled && SUPPORTED {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

#[cfg(test)]
mod contrast_tests {
    use super::*;
    use gpui_kit::rgb;

    fn luminance(color: gpui_kit::Rgba) -> f32 {
        let linear = |channel: f32| {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }

    fn over_black(color: Hsla) -> gpui_kit::Rgba {
        let color: gpui_kit::Rgba = color.into();
        gpui_kit::Rgba {
            r: color.r * color.a,
            g: color.g * color.a,
            b: color.b * color.a,
            a: 1.,
        }
    }

    #[test]
    fn light_shell_and_cards_preserve_text_contrast_over_black() {
        // Black is the darkest possible material behind the light theme.
        let foreground: gpui_kit::Rgba = rgb(0x18181b);
        for surface in [
            tint(rgb(0xffffff).into(), true),
            card_surface(rgb(0xffffff).into(), true),
        ] {
            let contrast = (luminance(over_black(surface)) + 0.05) / (luminance(foreground) + 0.05);
            assert!(contrast >= 4.5, "light text contrast was {contrast}:1");
        }
    }
}

pub(crate) fn tint(background: Hsla, enabled: bool) -> Hsla {
    background.alpha(if enabled && SUPPORTED {
        // Light text tokens are dark: retain a light base even over a black desktop.
        if background.l > 0.5 { 0.80 } else { 0.20 }
    } else {
        1.
    })
}

pub(crate) fn card_surface(color: Hsla, enabled: bool) -> Hsla {
    color.alpha(if enabled && SUPPORTED { 0.82 } else { 1. })
}

pub(crate) fn apply(enabled: bool, window: &mut Window, cx: &mut App) {
    window.set_background_appearance(appearance(enabled));
    // The component plugin's opaque root otherwise hides the native material.
    // Only this window is changed; secondary windows keep Kit's solid root.
    gpui_kit::base::Root::update(window, cx, |root, _, cx| {
        root.style().background = Some(gpui_kit::transparent_black().into());
        cx.notify();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::rgb;

    #[test]
    fn disabled_or_unsupported_material_is_opaque() {
        assert_eq!(appearance(false), WindowBackgroundAppearance::Opaque);
        assert_eq!(tint(rgb(0x202020).into(), false).a, 1.);
        if !SUPPORTED {
            assert_eq!(appearance(true), WindowBackgroundAppearance::Opaque);
            assert_eq!(tint(rgb(0x202020).into(), true).a, 1.);
        }
    }

    #[test]
    fn supported_material_leaves_a_readable_theme_tint() {
        if SUPPORTED {
            assert_eq!(appearance(true), WindowBackgroundAppearance::Blurred);
            let background: Hsla = rgb(0x202020).into();
            let tinted = tint(background, true);
            assert_eq!(tinted.a, 0.20);
            assert_eq!(tinted.h, background.h);
            assert_eq!(tinted.s, background.s);
            assert_eq!(tinted.l, background.l);
        }
    }

    #[test]
    fn light_mode_retains_a_light_base_without_hiding_native_blur() {
        let light: Hsla = rgb(0xffffff).into();
        if SUPPORTED {
            assert_eq!(tint(light, true).a, 0.80);
            assert!(tint(light, true).a > tint(rgb(0x202020).into(), true).a);
            assert!(tint(light, true).a < 1.);
        }
        assert_eq!(tint(light, false).a, 1.);
    }

    #[test]
    fn cards_use_a_stronger_tint_than_the_shell_and_an_opaque_fallback() {
        let color: Hsla = rgb(0x202020).into();
        assert_eq!(card_surface(color.alpha(0.4), false).a, 1.);
        assert_eq!(card_surface(color, true).l, color.l);
        if SUPPORTED {
            assert_eq!(card_surface(color, true).a, 0.82);
            assert!(card_surface(color, true).a > tint(color, true).a);
        } else {
            assert_eq!(card_surface(color, true).a, 1.);
        }
    }
}
