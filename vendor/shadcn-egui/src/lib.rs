//! The [shadcn/ui](https://ui.shadcn.com/) design system for egui: the
//! shadcn v4 styles, colour themes, fonts, icon sets and presets, and
//! components drawn as plain egui widgets. Works with any egui backend.
//!
//! Apply a [`Theme`] (or a [`Preset`] from a shadcn preset code) once when
//! the context is created, then use the [`components`] like egui widgets:
//!
//! ```
//! use shadcn_egui::components::button::Button;
//! use shadcn_egui::Theme;
//!
//! fn setup(ctx: &egui::Context) {
//!     Theme::vega().apply(ctx);
//! }
//!
//! fn ui(ui: &mut egui::Ui) {
//!     if ui.add(Button::new("Save changes")).clicked() {
//!         // ...
//!     }
//! }
//! # let ctx = egui::Context::default();
//! # setup(&ctx);
//! # let mut output = ctx.run_ui(Default::default(), |u| ui(u));
//! # output.textures_delta.clear();
//! ```
//!
//! Fonts other than Inter and icon sets other than Lucide are Cargo
//! features; the README lists them. The `gallery` example in the git
//! repository (not in the crates.io package) shows every component under
//! any preset: `cargo run --example gallery --features fonts-all,icons-all`.

mod box_shadow;
#[cfg(test)]
mod test_ui {
    pub(crate) fn run(
        ctx: &egui::Context,
        input: egui::RawInput,
        run_ui: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(input, run_ui);
        // Headless tests inspect shapes but have no renderer to apply texture deltas.
        output.textures_delta.clear();
        output
    }
}
pub mod color;
pub mod components;
pub mod fonts;
pub mod icons;
pub mod layout;
mod palette;
pub mod preset;
pub mod reference;
pub mod style;
pub mod theme;

pub use preset::{
    BaseColor, FontHeading, FontName, IconLibrary, MenuAccent, MenuColor, Preset, PresetCodeError,
    RadiusChoice, StyleName, ThemeColor,
};
pub use shadcn_egui_core::*;
pub use style::Style;
pub use theme::{theme, Metrics, Palette, ShadowLayer, Theme, ThemeFonts};

// The Claude Code skills under plugin/skills/ name this crate's API. Compiling
// their rust fences as doctests makes an API change fail `cargo test` until
// the skill text follows. cfg(doctest) drops these items before include_str!
// expands, so builds and `cargo package` never read the files, which lie
// outside the package.
#[cfg(any())] // Upstream plugin documentation is not part of the published crate.
mod skill_docs {
    #[doc = include_str!("../../../plugin/skills/build-ui/SKILL.md")]
    struct BuildUi;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/component-map.md")]
    struct ComponentMap;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/cpu-rendering.md")]
    struct CpuRendering;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/gotchas.md")]
    struct Gotchas;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/icons.md")]
    struct Icons;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/layout-map.md")]
    struct LayoutMap;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/presets.md")]
    struct Presets;
    #[doc = include_str!("../../../plugin/skills/build-ui/references/token-map.md")]
    struct TokenMap;
    #[doc = include_str!("../../../plugin/skills/port-component/SKILL.md")]
    struct PortComponent;
    #[doc = include_str!("../../../plugin/skills/port-component/references/in-app.md")]
    struct InApp;
    #[doc = include_str!("../../../plugin/skills/port-component/references/styles.md")]
    struct Styles;
    #[doc = include_str!("../../../plugin/skills/port-component/references/upstream.md")]
    struct Upstream;
}
