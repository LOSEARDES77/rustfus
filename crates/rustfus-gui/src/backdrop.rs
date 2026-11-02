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

pub(crate) fn tint(background: Hsla, enabled: bool) -> Hsla {
    background.alpha(if enabled && SUPPORTED {
        if background.l > 0.5 { 0.08 } else { 0.20 }
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
    fn light_mode_does_not_wash_out_the_native_blur() {
        let light: Hsla = rgb(0xffffff).into();
        if SUPPORTED {
            assert_eq!(tint(light, true).a, 0.08);
            assert!(tint(light, true).a < tint(rgb(0x202020).into(), true).a);
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
