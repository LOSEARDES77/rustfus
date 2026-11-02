//! shadcn styles: recipe types (hand-written, `recipe.rs`) and generated
//! style values (`generated.rs`, `<name>.rs`, by `tools/stylegen`).
//! See `references/styles.md` in the skill.

pub mod app;
#[rustfmt::skip]
mod generated;
#[rustfmt::skip]
mod luma;
#[rustfmt::skip]
mod lyra;
#[rustfmt::skip]
mod maia;
#[rustfmt::skip]
mod mira;
#[rustfmt::skip]
mod nova;
pub mod recipe;
#[rustfmt::skip]
mod rhea;
#[rustfmt::skip]
mod vega;

pub use app::{AppEnum, AppVariant, Edit, SizeEnum, VariantEnum};
pub use generated::Style;
pub use recipe::*;

// The descriptions are upstream's (`docs/src/lib/registry/styles/index.ts`).
impl Style {
    /// The classic shadcn/ui look. Clean, neutral, and familiar.
    pub fn vega() -> Self {
        vega::style()
    }
    /// Reduced padding and margins for compact layouts.
    pub fn nova() -> Self {
        nova::style()
    }
    /// Soft and rounded, with generous spacing.
    pub fn maia() -> Self {
        maia::style()
    }
    /// Boxy and sharp. Pairs well with mono fonts.
    pub fn lyra() -> Self {
        lyra::style()
    }
    /// Compact. Made for dense interfaces.
    pub fn mira() -> Self {
        mira::style()
    }
    /// Rounded geometry. Soft elevation. Breathable layouts.
    pub fn luma() -> Self {
        luma::style()
    }
    /// Like luma, but compact.
    pub fn rhea() -> Self {
        rhea::style()
    }
}

impl Default for Style {
    fn default() -> Self {
        Style::vega()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stylegen ranks an `!important` class with bit 127; `Rule::IMPORTANT`
    /// must be that bit, or an app's important rule would lose to upstream's.
    /// The rule is found by that bit, not by value.
    #[test]
    fn generated_important_rank_is_rule_important() {
        let luma = Style::luma();
        let r = luma
            .badge
            .icon_size
            .rules
            .iter()
            .find(|r| r.rank & Rule::<f32>::IMPORTANT != 0)
            .expect("luma cn-badge [&>svg]:size-3!");
        assert_eq!((r.when, r.value), (When::ALWAYS, 12.0));
        // `[&>svg]` adds no specificity and no variant bit.
        assert_eq!(r.rank, Rule::<f32>::IMPORTANT);
    }

    #[test]
    fn generated_styles_carry_known_values() {
        let (v, n) = (Style::vega(), Style::nova());
        assert_eq!(v.name, "vega");
        assert_eq!(n.name, "nova");
        assert_eq!(v.button_size_default.height.get(State::NONE), Some(36.0));
        assert_eq!(n.button_size_default.height.get(State::NONE), Some(32.0));
        assert_eq!(n.button_size_default.pad_left.get(State::NONE), Some(10.0));
        assert_eq!(
            n.button_size_default.pad_left.get(State::ICON_START),
            Some(8.0)
        );
        assert_eq!(n.card_content.pad_left.get(State::SIZE_SM), Some(12.0));
        assert_eq!(Style::default(), Style::vega());
    }

    type Maker = fn() -> Style;

    const MAKERS: [(&str, Maker, f32); 7] = [
        ("vega", Style::vega, 36.0),
        ("nova", Style::nova, 32.0),
        ("maia", Style::maia, 36.0),
        ("lyra", Style::lyra, 32.0),
        ("mira", Style::mira, 28.0),
        ("luma", Style::luma, 36.0),
        ("rhea", Style::rhea, 32.0),
    ];

    #[test]
    fn every_exported_style_resolves_its_button_height() {
        for (name, make, height) in MAKERS {
            let s = make();
            assert_eq!(s.name, name);
            assert_eq!(
                s.button_size_default.height.get(State::NONE),
                Some(height),
                "{name}"
            );
        }
    }

    /// Windows main threads and wasm have 1 MB of stack and `Theme::current`
    /// builds its fallback on whichever thread asks first, so building a
    /// style must fit in a small stack, also in debug builds.
    #[test]
    fn every_exported_style_builds_on_a_small_stack() {
        for (name, make, _) in MAKERS {
            let built = std::thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || make().name)
                .expect("spawn")
                .join();
            assert_eq!(built.ok(), Some(name), "{name}: stack overflow");
        }
        let theme = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| crate::Theme::vega().style.name)
            .expect("spawn")
            .join();
        assert_eq!(theme.ok(), Some("vega"));
    }

    /// Every recipe lives on the heap: a debug build gives each `Style`
    /// temporary its own stack slot, so a large `Style` overflows small
    /// thread stacks as soon as a function holds a few of them.
    #[test]
    fn style_is_small() {
        // One unboxed Recipe is about 1.5 KB; allow the name and a few
        // non-recipe fields, never a recipe.
        let size = std::mem::size_of::<Style>();
        assert!(size < 1024, "size_of::<Style>() = {size}");
    }

    /// Base classes come after the style's classes in each part's string,
    /// so they win under the same condition, as in the published component.
    #[test]
    fn base_classes_reach_every_style() {
        let line_active = State::HORIZONTAL | State::VARIANT_LINE | State::SELECTED;
        for (name, make, _) in MAKERS {
            let s = make();
            assert_eq!(s.dialog_footer.gap.get(State::NONE), Some(8.0), "{name}");
            assert_eq!(
                s.tooltip_content.max_width.get(State::NONE),
                Some(320.0),
                "{name}"
            );
            assert_eq!(
                s.popover_content.width.get(State::NONE),
                Some(288.0),
                "{name}"
            );
            assert_eq!(
                s.tabs_trigger.height_inset.get(State::NONE),
                Some(1.0),
                "{name}"
            );
            assert_eq!(
                s.separator.bg.get(State::NONE),
                Some(Paint::Role(Role::Border, 1.0)),
                "{name}"
            );
            assert_eq!(
                s.tabs_trigger_after.opacity.get(line_active),
                Some(1.0),
                "{name}"
            );
            assert_eq!(
                s.tabs_trigger_after.opacity.get(State::HORIZONTAL),
                Some(0.0),
                "{name}"
            );
            assert_eq!(s.button.opacity.get(State::DISABLED), Some(0.5), "{name}");
        }
    }
}
