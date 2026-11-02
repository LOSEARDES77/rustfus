//! App variants and app sizes: looks an app defines once in its theme, from
//! a built-in variant or size, and names at the call
//! ([`crate::Theme::variant`], [`crate::Theme::size`]).

use super::{Recipe, State, Style};
use crate::preset::StyleName;
use egui::Color32;
use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// The name of an app variant or app size of `X`, made by `X::app(name)`
/// (`ButtonVariant::app("danger-ghost")`). Name it through a constant, so a
/// misspelling does not compile; two constants with one name are one app
/// variant. A setter takes it as `impl Into<X>`.
pub struct AppVariant<X> {
    name: &'static str,
    _enum: PhantomData<fn() -> X>,
}

impl<X> AppVariant<X> {
    pub(crate) const fn new(name: &'static str) -> Self {
        AppVariant {
            name,
            _enum: PhantomData,
        }
    }

    /// The name the theme knows this app variant by.
    pub const fn name(self) -> &'static str {
        self.name
    }
}

// Derives would bound `X`; an app variant is a name whatever `X` is.
impl<X> Clone for AppVariant<X> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<X> Copy for AppVariant<X> {}
impl<X> PartialEq for AppVariant<X> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl<X> Eq for AppVariant<X> {}
impl<X> std::hash::Hash for AppVariant<X> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}
impl<X> std::fmt::Debug for AppVariant<X> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AppVariant").field(&self.name).finish()
    }
}

mod sealed {
    pub trait Sealed {}
}
pub(crate) use sealed::Sealed;

/// A component's variant or size enum: the crate's own enums only.
pub trait AppEnum: Sealed + Copy + Eq + Default + std::fmt::Debug + Send + Sync + 'static {
    /// What an edit gets: one [`Recipe`] per upstream part the enum
    /// reaches, named after its `data-slot` (`ButtonRecipes { button }`).
    type Recipes: Clone + Send + Sync + 'static;
    /// The app case's name; `None` for a built-in case.
    #[doc(hidden)]
    fn app_name(self) -> Option<&'static str>;
    /// Copies of the recipes a built-in case draws with.
    #[doc(hidden)]
    fn recipes(self, style: &Style) -> Self::Recipes;
    /// The state bits a built-in case adds (tabs `VARIANT_LINE`).
    #[doc(hidden)]
    fn bits(self) -> State {
        State::NONE
    }
    /// Turn a copy of the parts this built-in case shares with the
    /// component's variant into its own: a state-bit size makes its state
    /// rules unconditional (`Recipe::fold_state`), so an app size's
    /// `Prop::set` takes effect. Nothing for an enum with its own layer.
    #[doc(hidden)]
    fn fold(self, _recipes: &mut Self::Recipes) {}
    /// Every part of `recipes`, in field order.
    #[doc(hidden)]
    fn parts(recipes: &mut Self::Recipes) -> Vec<&mut Recipe>;
}

/// Run `edit` on `r` and skip it wherever it names an app colour
/// `defined` does not know (rules, App variants); return such a name.
fn edit_checked<X: AppEnum>(
    edit: Edit<X>,
    r: &mut X::Recipes,
    name: StyleName,
    defined: &dyn Fn(&str) -> bool,
) -> Option<&'static str> {
    let mut base = r.clone();
    edit(r, name);
    X::parts(&mut base)
        .into_iter()
        .zip(X::parts(r))
        .filter_map(|(b, e)| e.skip_undefined(b, defined))
        .last()
}

/// The `debug_assert!` an undefined app colour fails.
fn assert_colour(missing: Option<&'static str>) {
    debug_assert!(
        missing.is_none(),
        "app colour {:?} is not defined in the theme",
        missing.unwrap_or_default()
    );
}

/// A variant enum: what [`crate::Theme::variant`] takes.
pub trait VariantEnum: AppEnum {}

/// A size enum: what [`crate::Theme::size`] takes.
pub trait SizeEnum: AppEnum {}

/// An edit: gets copies of the base's recipes and the style the theme
/// builds for, captures nothing.
pub type Edit<X> = fn(&mut <X as AppEnum>::Recipes, StyleName);

trait Entry: Send + Sync {
    fn key(&self) -> (TypeId, &'static str);
    fn build(
        &self,
        style: &Style,
        name: StyleName,
        defined: &dyn Fn(&str) -> bool,
    ) -> (Arc<dyn Any + Send + Sync>, Option<&'static str>);
    fn same(&self, other: &dyn Entry) -> bool;
    fn as_any(&self) -> &dyn Any;
    fn describe(&self) -> String;
}

struct Typed<X: AppEnum> {
    name: &'static str,
    base: X,
    edit: Edit<X>,
}

impl<X: AppEnum> Entry for Typed<X> {
    fn key(&self) -> (TypeId, &'static str) {
        (TypeId::of::<X>(), self.name)
    }
    fn build(
        &self,
        style: &Style,
        name: StyleName,
        defined: &dyn Fn(&str) -> bool,
    ) -> (Arc<dyn Any + Send + Sync>, Option<&'static str>) {
        let mut r = self.base.recipes(style);
        let missing = edit_checked::<X>(self.edit, &mut r, name, defined);
        (Arc::new(r), missing)
    }
    fn same(&self, other: &dyn Entry) -> bool {
        other.as_any().downcast_ref::<Typed<X>>().is_some_and(|o| {
            o.name == self.name && o.base == self.base && std::ptr::fn_addr_eq(o.edit, self.edit)
        })
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn describe(&self) -> String {
        format!("{:?} from {:?}", self.name, self.base)
    }
}

/// The edited recipes of every entry, for one style.
struct Built {
    /// Held so a later `Arc::make_mut` of the theme's style copies it and
    /// the pointer comparison sees the change.
    style: Arc<Style>,
    recipes: Vec<Arc<dyn Any + Send + Sync>>,
    /// An app variant and an app size that edit the same parts, built
    /// together on first use ([`AppMap::resolve_pair`]).
    pairs: Vec<(PairKey, Arc<dyn Any + Send + Sync>)>,
}

type PairKey = (TypeId, &'static str, &'static str);

/// A theme's app variants and app colours. Shared behind an `Arc`; the
/// edited recipes are built on the first lookup for a style and kept until
/// the style changes.
#[derive(Default)]
pub(crate) struct AppMap {
    entries: Vec<Arc<dyn Entry>>,
    /// An app colour's light and dark value.
    colors: Vec<(&'static str, Color32, Color32)>,
    built: Mutex<Option<Built>>,
}

impl Clone for AppMap {
    fn clone(&self) -> Self {
        // A clone is made to change the entries: it builds again.
        AppMap {
            entries: self.entries.clone(),
            colors: self.colors.clone(),
            built: Mutex::new(None),
        }
    }
}

impl PartialEq for AppMap {
    /// Equal entries and colours; the built recipes follow from them.
    fn eq(&self, other: &Self) -> bool {
        self.colors == other.colors
            && self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(&other.entries)
                .all(|(a, b)| a.same(&**b))
    }
}

impl std::fmt::Debug for AppMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppMap")
            .field(
                "entries",
                &self
                    .entries
                    .iter()
                    .map(|e| e.describe())
                    .collect::<Vec<_>>(),
            )
            .field("colors", &self.colors)
            .finish()
    }
}

impl AppMap {
    pub(crate) fn insert<X: AppEnum>(&mut self, app: AppVariant<X>, base: X, edit: Edit<X>) {
        debug_assert!(
            base.app_name().is_none(),
            "app variant {:?}: its base {base:?} must be a built-in case",
            app.name
        );
        let base = if base.app_name().is_some() {
            X::default()
        } else {
            base
        };
        let key = (TypeId::of::<X>(), app.name);
        self.entries.retain(|e| e.key() != key);
        self.entries.push(Arc::new(Typed {
            name: app.name,
            base,
            edit,
        }));
        *self.built.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(crate) fn set_color(&mut self, name: &'static str, light: Color32, dark: Color32) {
        self.colors.retain(|(n, ..)| *n != name);
        self.colors.push((name, light, dark));
        // Which edits are skipped depends on the colours defined.
        *self.built.get_mut().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(crate) fn color(&self, name: &str, dark: bool) -> Option<Color32> {
        self.colors
            .iter()
            .find(|(n, ..)| *n == name)
            .map(|&(_, l, d)| if dark { d } else { l })
    }

    /// The recipes and state bits `x` draws with in `style`: a built-in
    /// case's own, or an app case's edited copies with its base's bits. An
    /// app case the theme does not define fails a `debug_assert!` and
    /// resolves to `X::default()`.
    pub(crate) fn resolve<X: AppEnum>(&self, style: &Arc<Style>, x: X) -> Resolved<X> {
        let (r, missing) = self.resolve_quiet(style, x);
        debug_assert!(
            missing.is_none(),
            "app variant {:?} of {} is not defined in the theme",
            missing.unwrap_or_default(),
            std::any::type_name::<X>()
        );
        r
    }

    /// [`AppMap::resolve`] without the `debug_assert!`: the name of an
    /// undefined app case comes back beside `X::default()`.
    pub(crate) fn resolve_quiet<X: AppEnum>(
        &self,
        style: &Arc<Style>,
        x: X,
    ) -> (Resolved<X>, Option<&'static str>) {
        let Some(name) = x.app_name() else {
            return (Resolved { base: x, app: None }, None);
        };
        let key = (TypeId::of::<X>(), name);
        let Some(i) = self.entries.iter().position(|e| e.key() == key) else {
            let r = Resolved {
                base: X::default(),
                app: None,
            };
            return (r, Some(name));
        };
        let base = self.entries[i]
            .as_any()
            .downcast_ref::<Typed<X>>()
            .map(|t| t.base)
            .unwrap_or_default();
        let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
        if built.as_ref().is_none_or(|b| !Arc::ptr_eq(&b.style, style)) {
            let (b, missing) = self.build_quiet(style);
            *built = Some(b);
            assert_colour(missing);
        }
        let recipes = built
            .as_ref()
            .and_then(|b| b.recipes[i].clone().downcast::<X::Recipes>().ok());
        (Resolved { base, app: recipes }, None)
    }

    /// The recipes of a component whose variant `v` and size `z` edit the
    /// same parts (a size that works through state bits, card `data-size`):
    /// an app variant's copies, an app size's, or both edits in stack order
    /// (variant, then size) on one copy. Built once per style.
    pub(crate) fn resolve_pair<V: AppEnum, Z: AppEnum<Recipes = V::Recipes>>(
        &self,
        style: &Arc<Style>,
        v: V,
        z: Z,
    ) -> ResolvedPair<V, Z> {
        let rv = self.resolve(style, v);
        let rz = self.resolve(style, z);
        let app = match (rv.app, rz.app) {
            (None, None) => None,
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (Some(a), Some(_)) => {
                let key = (
                    TypeId::of::<(V, Z)>(),
                    v.app_name().unwrap_or_default(),
                    z.app_name().unwrap_or_default(),
                );
                let mut built = self.built.lock().unwrap_or_else(|e| e.into_inner());
                let b = built.as_mut().expect("resolve built the recipes");
                let found = b.pairs.iter().find(|(k, _)| *k == key).map(|(_, r)| r);
                let r = match found.and_then(|r| r.clone().downcast::<V::Recipes>().ok()) {
                    Some(r) => r,
                    None => {
                        let mut r = (*a).clone();
                        rz.base.fold(&mut r);
                        let edit = self
                            .entries
                            .iter()
                            .find(|e| e.key() == (TypeId::of::<Z>(), key.2))
                            .and_then(|e| e.as_any().downcast_ref::<Typed<Z>>())
                            .map(|t| t.edit);
                        if let Some(edit) = edit {
                            assert_colour(edit_checked::<Z>(
                                edit,
                                &mut r,
                                StyleName::from_id(style.name).unwrap_or(StyleName::Vega),
                                &|n| self.color(n, false).is_some(),
                            ));
                        }
                        let r = Arc::new(r);
                        b.pairs.push((key, r.clone()));
                        r
                    }
                };
                Some(r)
            }
        };
        ResolvedPair {
            variant: rv.base,
            size: rz.base,
            app,
        }
    }

    /// The edited recipes of every entry for `style`, and the name of an
    /// app colour an edit names but the theme does not define (the edit
    /// is skipped there).
    fn build_quiet(&self, style: &Arc<Style>) -> (Built, Option<&'static str>) {
        let name = StyleName::from_id(style.name);
        debug_assert!(
            name.is_some(),
            "style {:?} is no upstream style: a restyled style keeps its source's name",
            style.name
        );
        let name = name.unwrap_or(StyleName::Vega);
        let defined = |n: &str| self.color(n, false).is_some();
        let mut missing = None;
        let recipes = self
            .entries
            .iter()
            .map(|e| {
                let (r, m) = e.build(style, name, &defined);
                missing = missing.or(m);
                r
            })
            .collect();
        let built = Built {
            style: Arc::clone(style),
            recipes,
            pairs: Vec::new(),
        };
        (built, missing)
    }
}

/// The variant enum of a component without upstream variants: `$x` with
/// the one built-in case `Default` and the hidden app case, and `$recipes`
/// with one field per part, each named after its `data-slot` and copied
/// from the style's recipe of that name (rules, App variants).
macro_rules! default_variant {
    (
        $(#[$m:meta])*
        $x:ident,
        $(#[$rm:meta])*
        $recipes:ident { $($(#[$fm:meta])* $field:ident),+ $(,)? }
    ) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum $x {
            /// The component as upstream draws it.
            #[default]
            Default,
            /// An app variant: [`Self::app`].
            #[doc(hidden)]
            App($crate::style::AppVariant<$x>),
        }

        impl $x {
            /// An app variant, defined in the theme ([`crate::Theme::variant`]).
            pub const fn app(name: &'static str) -> $crate::style::AppVariant<$x> {
                $crate::style::AppVariant::new(name)
            }
        }

        impl From<$crate::style::AppVariant<$x>> for $x {
            fn from(app: $crate::style::AppVariant<$x>) -> Self {
                $x::App(app)
            }
        }

        $(#[$rm])*
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub struct $recipes {
            $($(#[$fm])* pub $field: $crate::style::Recipe,)+
        }

        impl $crate::style::app::Sealed for $x {}
        impl $crate::style::AppEnum for $x {
            type Recipes = $recipes;
            fn app_name(self) -> Option<&'static str> {
                match self {
                    $x::App(a) => Some(a.name()),
                    $x::Default => None,
                }
            }
            fn recipes(self, style: &$crate::style::Style) -> $recipes {
                $recipes {
                    $($field: (*style.$field).clone(),)+
                }
            }
            fn parts(recipes: &mut $recipes) -> Vec<&mut $crate::style::Recipe> {
                vec![$(&mut recipes.$field),+]
            }
        }
        impl $crate::style::VariantEnum for $x {}

        impl $recipes {
            /// Every part, for an edit of all of them.
            #[allow(dead_code)]
            pub(crate) fn each_mut(&mut self, mut f: impl FnMut(&mut $crate::style::Recipe)) {
                $(f(&mut self.$field);)+
            }
        }
    };
}
pub(crate) use default_variant;

/// The app plumbing of a `data-size` enum `$z { Default, Sm, App }` whose
/// size works through state bits (`data-[size=sm]:` rules in the parts'
/// own recipes): an app size edits copies of the same parts as the
/// component's variant `$x` (`Recipes` shared), from its base's bits.
macro_rules! state_size {
    ($z:ident, $x:ident) => {
        impl $z {
            /// An app size, defined in the theme ([`crate::Theme::size`]).
            pub const fn app(name: &'static str) -> $crate::style::AppVariant<$z> {
                $crate::style::AppVariant::new(name)
            }
        }

        impl From<$crate::style::AppVariant<$z>> for $z {
            fn from(app: $crate::style::AppVariant<$z>) -> Self {
                $z::App(app)
            }
        }

        impl $crate::style::app::Sealed for $z {}
        impl $crate::style::AppEnum for $z {
            type Recipes = <$x as $crate::style::AppEnum>::Recipes;
            fn app_name(self) -> Option<&'static str> {
                match self {
                    $z::App(a) => Some(a.name()),
                    _ => None,
                }
            }
            fn recipes(self, style: &$crate::style::Style) -> Self::Recipes {
                let mut r = <$x as $crate::style::AppEnum>::recipes($x::Default, style);
                self.fold(&mut r);
                r
            }
            fn fold(self, recipes: &mut Self::Recipes) {
                let bits = $crate::style::AppEnum::bits(self);
                recipes.each_mut(|r| r.fold_state(bits));
            }
            fn parts(recipes: &mut Self::Recipes) -> Vec<&mut $crate::style::Recipe> {
                <$x as $crate::style::AppEnum>::parts(recipes)
            }
            fn bits(self) -> $crate::style::State {
                match self {
                    $z::Sm => $crate::style::State::SIZE_SM,
                    _ => $crate::style::State::SIZE_DEFAULT,
                }
            }
        }
        impl $crate::style::SizeEnum for $z {}
    };
}
pub(crate) use state_size;

/// What a component draws an enum value with ([`AppMap::resolve`]).
pub(crate) struct Resolved<X: AppEnum> {
    base: X,
    app: Option<Arc<X::Recipes>>,
}

impl<X: AppEnum> Resolved<X> {
    /// The built-in case: the value itself, or the app case's base.
    pub(crate) fn base(&self) -> X {
        self.base
    }

    /// The state bits of the built-in case.
    pub(crate) fn bits(&self) -> State {
        self.base.bits()
    }

    /// One part's recipe: the app case's edited copy (`app`), else the
    /// built-in case's (`builtin`).
    pub(crate) fn part<'a>(
        &'a self,
        style: &'a Style,
        builtin: fn(X, &Style) -> &Recipe,
        app: fn(&X::Recipes) -> &Recipe,
    ) -> &'a Recipe {
        match &self.app {
            Some(r) => app(r),
            None => builtin(self.base, style),
        }
    }
}

/// What a component whose variant and size edit the same parts draws
/// with ([`AppMap::resolve_pair`]).
pub(crate) struct ResolvedPair<V: AppEnum, Z: AppEnum<Recipes = V::Recipes>> {
    variant: V,
    size: Z,
    app: Option<Arc<V::Recipes>>,
}

impl<V: AppEnum, Z: AppEnum<Recipes = V::Recipes>> ResolvedPair<V, Z> {
    /// The state bits of the built-in variant and size (the app cases' bases).
    pub(crate) fn bits(&self) -> State {
        self.variant.bits().or(self.size.bits())
    }

    /// One part's recipe: the edited copy (`app`) when either is an app
    /// case, else the style's (`builtin`).
    pub(crate) fn part<'a>(
        &'a self,
        style: &'a Style,
        builtin: fn(&Style) -> &Recipe,
        app: fn(&V::Recipes) -> &Recipe,
    ) -> &'a Recipe {
        match &self.app {
            Some(r) => app(r),
            None => builtin(style),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::button::ButtonVariant;
    use crate::style::{pick, Paint, Prop, Role, Rule, When};
    use crate::Theme;

    const DANGER_GHOST: AppVariant<ButtonVariant> = ButtonVariant::app("danger-ghost");
    const DANGER_OUTLINE: AppVariant<ButtonVariant> = ButtonVariant::app("danger-outline");
    const BRAND: Role = Role::custom("brand");

    fn danger_ghost(r: &mut <ButtonVariant as AppEnum>::Recipes, _: StyleName) {
        r.button.text.set(Paint::Role(Role::Destructive, 1.0));
    }

    fn themed() -> Theme {
        Theme::vega()
            .variant(DANGER_GHOST, ButtonVariant::Ghost, danger_ghost)
            .variant(DANGER_OUTLINE, ButtonVariant::Outline, |r, _| {
                r.button.recolor(Role::Foreground, Role::Destructive)
            })
    }

    /// The variant layer `v` draws with, edited or built in.
    fn layer(t: &Theme, v: ButtonVariant) -> Recipe {
        let r = t.resolve(v);
        r.part(&t.style, ButtonVariant::layer, |r| &r.button)
            .clone()
    }

    const P: fn(Role) -> Paint = |r| Paint::Role(r, 1.0);

    /// The states to compare a folded size under: every combination of up
    /// to two of `flags` (of all of them when there are ten or fewer), in
    /// both modes, each with the size's `bits`.
    fn states_with(flags: &[State], bits: State) -> Vec<State> {
        let n = flags.len();
        let combos: Vec<State> = if n <= 10 {
            (0..1u32 << n)
                .map(|m| {
                    (0..n)
                        .filter(|i| m >> i & 1 == 1)
                        .fold(State::NONE, |s, i| s | flags[i])
                })
                .collect()
        } else {
            let mut v = vec![State::NONE];
            for i in 0..n {
                v.push(flags[i]);
                for j in i + 1..n {
                    v.push(flags[i] | flags[j]);
                }
            }
            v
        };
        combos
            .into_iter()
            .flat_map(|s| [s | bits, s | bits | State::DARK])
            .collect()
    }

    /// Every built-in state-bit size, folded, resolves every property of
    /// each part alone and of all parts stacked (both orders) as the
    /// unfolded parts do under the size's bits: value, rank and layer.
    #[test]
    fn a_folded_size_resolves_as_its_bits_do() {
        use crate::components::card::{CardSize, CardVariant};
        use crate::components::select::{SelectSize, SelectVariant};
        use crate::components::switch::{SwitchSize, SwitchVariant};
        use crate::style::{flags_used, pick_diff};
        let styles = [
            Style::vega(),
            Style::nova(),
            Style::maia(),
            Style::lyra(),
            Style::mira(),
            Style::luma(),
            Style::rhea(),
        ];
        let mut diffs = Vec::new();
        let mut checked = 0usize;
        for style in &styles {
            macro_rules! check {
                ($z:ident, $x:ident) => {
                    for z in [$z::Default, $z::Sm] {
                        let (mut unfolded, mut folded) = (Vec::new(), Vec::new());
                        $x::Default
                            .recipes(style)
                            .each_mut(|p| unfolded.push(p.clone()));
                        z.recipes(style).each_mut(|p| folded.push(p.clone()));
                        let bits = z.bits();
                        let u: Vec<&Recipe> = unfolded.iter().collect();
                        let f: Vec<&Recipe> = folded.iter().collect();
                        let sizes = State::SIZE_SM | State::SIZE_DEFAULT | State::DARK;
                        let flags: Vec<State> = flags_used(&u)
                            .into_iter()
                            .filter(|s| !sizes.contains(*s))
                            .collect();
                        let mut stacks: Vec<Vec<usize>> = (0..u.len()).map(|i| vec![i]).collect();
                        stacks.push((0..u.len()).collect());
                        stacks.push((0..u.len()).rev().collect());
                        for st in states_with(&flags, bits) {
                            for idx in &stacks {
                                let a: Vec<&Recipe> = idx.iter().map(|&i| u[i]).collect();
                                let b: Vec<&Recipe> = idx.iter().map(|&i| f[i]).collect();
                                checked += 1;
                                for field in pick_diff(&b, &a, st) {
                                    diffs.push(format!(
                                        "{} {}::{:?} parts {idx:?} {st:?}: {field}",
                                        style.name,
                                        stringify!($z),
                                        z
                                    ));
                                }
                            }
                        }
                    }
                };
            }
            check!(CardSize, CardVariant);
            check!(SelectSize, SelectVariant);
            check!(SwitchSize, SwitchVariant);
        }
        assert!(checked > 0);
        assert!(
            diffs.is_empty(),
            "{} differences, first:\n{}",
            diffs.len(),
            diffs[..diffs.len().min(40)].join("\n")
        );
    }

    #[test]
    fn app_variant_is_a_name() {
        assert_eq!(DANGER_GHOST, ButtonVariant::app("danger-ghost"));
        assert_ne!(DANGER_GHOST, DANGER_OUTLINE);
        assert_eq!(DANGER_GHOST.name(), "danger-ghost");
        assert_eq!(
            ButtonVariant::from(DANGER_GHOST),
            ButtonVariant::from(ButtonVariant::app("danger-ghost"))
        );
        assert_eq!(
            ButtonVariant::from(DANGER_GHOST).app_name(),
            Some("danger-ghost")
        );
        assert_eq!(ButtonVariant::Ghost.app_name(), None);
    }

    #[test]
    fn set_replaces_the_unconditional_value_and_keeps_state_rules() {
        let t = themed();
        let s = layer(&t, DANGER_GHOST.into());
        let ghost = &t.style.button_variant_ghost;
        assert_eq!(s.text.get(State::NONE), Some(P(Role::Destructive)));
        // Ghost's hover and expanded colours stay.
        assert_eq!(s.text.get(State::HOVER), ghost.text.get(State::HOVER));
        assert_eq!(s.text.rules, ghost.text.rules);
        assert_eq!(s.bg, ghost.bg);
    }

    #[test]
    fn set_writes_into_always_rules() {
        let mut p = Prop {
            base: None,
            rules: vec![Rule {
                when: When::ALWAYS,
                rank: 1 << 121,
                value: 16.0,
            }],
        };
        p.set(20.0);
        assert_eq!((p.base, p.get(State::NONE)), (None, Some(20.0)));
        let mut q = Prop::fixed(1.0);
        q.rules.push(Rule {
            when: When::all(State::HOVER),
            rank: 1 << 122,
            value: 2.0,
        });
        q.set(3.0);
        assert_eq!(
            (q.get(State::NONE), q.get(State::HOVER)),
            (Some(3.0), Some(2.0))
        );
    }

    #[test]
    fn on_replaces_the_rules_of_exactly_that_state() {
        let ghost = Style::vega().button_variant_ghost;
        let mut bg = ghost.bg.clone();
        let hover_rank = bg
            .rules
            .iter()
            .find(|r| r.when == When::all(State::HOVER))
            .map(|r| r.rank);
        bg.on(State::HOVER, P(Role::Accent));
        let hovers: Vec<_> = bg
            .rules
            .iter()
            .filter(|r| r.when == When::all(State::HOVER))
            .collect();
        assert_eq!(hovers.len(), 1);
        assert_eq!(Some(hovers[0].rank), hover_rank);
        assert_eq!(bg.get(State::HOVER), Some(P(Role::Accent)));
        // `dark:hover:` is another condition: it stays and wins in dark mode.
        assert_eq!(
            bg.get(State::DARK | State::HOVER),
            Some(Paint::Role(Role::Muted, 0.5))
        );
        // A state the property had no rule for ranks as a generated
        // `focus-visible:` class would.
        bg.on(State::FOCUS_VISIBLE, P(Role::Ring));
        let r = bg.rules.last().expect("pushed");
        assert_eq!(
            (r.when, r.rank),
            (When::all(State::FOCUS_VISIBLE), (1 << 29) | (1 << 122))
        );
        // `dark:sm:active:`: the variant bits combine, the class steps add.
        bg.on(State::DARK | State::SM | State::ACTIVE, P(Role::Ring));
        assert_eq!(
            bg.rules.last().map(|r| r.rank),
            Some((1 << 30) | (1 << 66) | (1 << 68) | (2 << 122))
        );
    }

    /// `on(state, v)` for a state the property has no rule for beats or
    /// loses to a generated rule of another state as the generated rules
    /// order them: ghost's `aria-expanded:` beats its `hover:`.
    #[test]
    fn on_for_a_new_state_orders_as_generated_rules() {
        for style in [Style::vega(), Style::nova(), Style::lyra()] {
            let ghost = &style.button_variant_ghost.bg;
            let only = |state| Prop {
                base: None,
                rules: ghost
                    .rules
                    .iter()
                    .filter(|r| r.when == When::all(state))
                    .cloned()
                    .collect(),
            };
            let both = State::HOVER | State::EXPANDED;
            // The generated order: the expanded rule wins.
            let mut generated = ghost.clone();
            generated
                .rules
                .retain(|r| r.when != When::all(State::DARK | State::HOVER));
            for r in &mut generated.rules {
                r.value = P(if r.when == When::all(State::EXPANDED) {
                    Role::Primary
                } else {
                    Role::Accent
                });
            }
            assert_eq!(
                generated.get(both),
                Some(P(Role::Primary)),
                "{}",
                style.name
            );
            // Hover rule kept, expanded added by `on`: `on` wins.
            let mut a = only(State::HOVER);
            a.on(State::EXPANDED, P(Role::Primary));
            assert_eq!(a.get(both), Some(P(Role::Primary)), "{}", style.name);
            // Expanded rule kept, hover added by `on`: `on` loses.
            let mut b = only(State::EXPANDED);
            b.on(State::HOVER, P(Role::Accent));
            assert_eq!(
                b.get(both),
                Some(ghost.get(State::EXPANDED).unwrap()),
                "{}",
                style.name
            );
            // The added rule's rank is the generated one.
            assert_eq!(
                a.rules.last().map(|r| r.rank),
                only(State::EXPANDED).rules.first().map(|r| r.rank)
            );
            assert_eq!(
                b.rules.last().map(|r| r.rank),
                only(State::HOVER).rules.first().map(|r| r.rank)
            );
        }
    }

    #[test]
    fn recolor_swaps_a_role_everywhere_and_keeps_opacity() {
        let t = themed();
        let s = layer(&t, DANGER_OUTLINE.into());
        let outline = &t.style.button_variant_outline;
        assert_eq!(outline.text.get(State::HOVER), Some(P(Role::Foreground)));
        assert_eq!(s.text.get(State::HOVER), Some(P(Role::Destructive)));
        // Other roles stay.
        assert_eq!(s.bg, outline.bg);
        let mut r = Recipe {
            border_color: Prop::fixed(Paint::Mix(Role::Foreground, Role::Border, 0.3)),
            ..Recipe::default()
        };
        r.ring_color.rules.push(Rule {
            when: When::all(State::FOCUS_VISIBLE),
            rank: 1,
            value: Paint::Role(Role::Foreground, 0.5),
        });
        r.recolor(Role::Foreground, BRAND);
        assert_eq!(
            r.border_color.base,
            Some(Paint::Mix(BRAND, Role::Border, 0.3))
        );
        assert_eq!(r.ring_color.rules[0].value, Paint::Role(BRAND, 0.5));
    }

    #[test]
    fn built_in_resolves_to_itself() {
        let t = themed();
        let r = t.resolve(ButtonVariant::Outline);
        assert!(r.app.is_none());
        assert_eq!(r.base(), ButtonVariant::Outline);
        assert_eq!(r.bits(), State::NONE);
    }

    #[test]
    fn app_variant_takes_the_base_place_in_the_stack() {
        let t = themed();
        let r = t.resolve(ButtonVariant::from(DANGER_GHOST));
        assert_eq!(r.base(), ButtonVariant::Ghost);
        let v = r.part(&t.style, |_, s| &s.button_variant_default, |r| &r.button);
        let stack = [&*t.style.button, v, &*t.style.button_size_default];
        let text = |s| pick(&stack, |r| &r.text, s);
        assert_eq!(text(State::NONE), Some(P(Role::Destructive)));
        assert_eq!(text(State::HOVER), Some(P(Role::Foreground)));
    }

    #[test]
    #[should_panic(expected = "is not defined in the theme")]
    fn undefined_name_fails_a_debug_assert() {
        let _ = Theme::vega().resolve(ButtonVariant::from(ButtonVariant::app("nope")));
    }

    #[test]
    fn edited_recipes_build_once_per_style() {
        let mut t = themed();
        let get = |t: &Theme| {
            t.resolve(ButtonVariant::from(DANGER_GHOST))
                .app
                .expect("defined")
        };
        let a = get(&t);
        assert!(Arc::ptr_eq(&a, &get(&t.clone())));
        // A style change builds from the new style.
        t.style = Arc::new(Style::nova());
        let b = get(&t);
        assert!(!Arc::ptr_eq(&a, &b));
        let mut want = Style::nova().button_variant_ghost.as_ref().clone();
        want.text.set(P(Role::Destructive));
        assert_eq!(b.button, want);
        // Changing the style in place counts too.
        Arc::make_mut(&mut t.style).button_variant_ghost.bg = Prop::fixed(P(Role::Accent));
        assert_eq!(get(&t).button.bg, Prop::fixed(P(Role::Accent)));
    }

    #[test]
    fn edit_gets_the_style_name() {
        const TALL: AppVariant<ButtonVariant> = ButtonVariant::app("tall");
        let mut t = Theme::vega().variant(TALL, ButtonVariant::Default, |r, style| {
            r.button
                .height
                .set(if style == StyleName::Nova { 40.0 } else { 50.0 })
        });
        let h = |t: &Theme| {
            t.resolve(ButtonVariant::from(TALL))
                .app
                .expect("defined")
                .button
                .height
                .get(State::NONE)
        };
        assert_eq!(h(&t), Some(50.0));
        t.style = Arc::new(Style::nova());
        assert_eq!(h(&t), Some(40.0));
    }

    #[test]
    fn app_colours_follow_the_mode_and_variants_do_not() {
        let t = themed().color(BRAND, egui::Color32::RED, egui::Color32::BLUE);
        let light = t.clone().in_mode(egui::Theme::Light);
        let dark = t.clone().in_mode(egui::Theme::Dark);
        assert_eq!(light.role(BRAND), egui::Color32::RED);
        assert_eq!(dark.role(BRAND), egui::Color32::BLUE);
        assert_eq!(
            dark.paint(Paint::Role(BRAND, 0.5)),
            egui::Color32::BLUE.gamma_multiply(0.5)
        );
        let v = |t: &Theme| {
            t.resolve(ButtonVariant::from(DANGER_GHOST))
                .app
                .expect("defined")
        };
        assert!(Arc::ptr_eq(&v(&light), &v(&dark)));
    }

    #[test]
    #[should_panic(expected = "is not defined in the theme")]
    fn undefined_colour_fails_a_debug_assert() {
        let _ = Theme::vega().role(BRAND);
    }

    /// An undefined app variant draws as the built-in `Default` case: the
    /// value a release build takes after the `debug_assert!`.
    #[test]
    fn an_undefined_variant_resolves_to_default() {
        let t = themed();
        let (r, missing) = t
            .apps
            .resolve_quiet(&t.style, ButtonVariant::from(ButtonVariant::app("nope")));
        assert_eq!(missing, Some("nope"));
        assert_eq!(r.base, ButtonVariant::Default);
        assert!(r.app.is_none());
        let (r, missing) = t
            .apps
            .resolve_quiet(&t.style, ButtonVariant::from(DANGER_GHOST));
        assert_eq!((missing, r.base), (None, ButtonVariant::Ghost));
    }

    /// An edit that names an undefined app colour is skipped where it
    /// names it, as a release build does after the `debug_assert!`: `set`,
    /// `on` and `recolor` leave the base's value for that property and
    /// state, and every other change of the edit stays.
    #[test]
    fn an_undefined_colour_skips_the_edit_and_keeps_the_base() {
        const GONE: Role = Role::custom("gone");
        let base = Style::vega().button_variant_outline.as_ref().clone();
        let mut r = base.clone();
        r.bg.set(P(GONE));
        r.text.on(State::HOVER, P(BRAND));
        r.border_color.set(P(BRAND));
        r.recolor(Role::Ring, GONE);
        r.outline_color.set(Paint::Mix(Role::Border, GONE, 0.5));
        r.height.set(50.0);
        let missing = r.skip_undefined(&base, |n| n == "brand");
        assert_eq!(missing, Some("gone"));
        assert_eq!(r.bg, base.bg);
        assert_eq!(r.ring_color, base.ring_color);
        assert_eq!(r.outline_color, base.outline_color);
        assert_eq!(r.text.get(State::HOVER), Some(P(BRAND)));
        assert_eq!(r.border_color.get(State::NONE), Some(P(BRAND)));
        assert_eq!(r.height.get(State::NONE), Some(50.0));

        // `on` over the base's own rules of that state puts them back.
        let mut r = base.clone();
        r.text.on(State::HOVER, P(GONE));
        assert_eq!(r.skip_undefined(&base, |_| false), Some("gone"));
        for st in [State::NONE, State::HOVER, State::HOVER | State::DARK] {
            assert_eq!(r.text.get(st), base.text.get(st), "{st:?}");
        }
        // Nothing undefined: nothing changes.
        let mut r = base.clone();
        r.text.set(P(BRAND));
        let want = r.clone();
        assert_eq!(r.skip_undefined(&base, |n| n == "brand"), None);
        assert_eq!(r, want);
    }

    /// Through the theme, the skipped edit draws the base variant's value.
    #[test]
    fn a_variant_with_an_undefined_colour_draws_its_base_there() {
        let t = Theme::vega()
            .color(BRAND, egui::Color32::RED, egui::Color32::BLUE)
            .variant(DANGER_GHOST, ButtonVariant::Ghost, |r, _| {
                r.button.text.set(P(Role::custom("gone")));
                r.button.bg.set(P(BRAND));
            });
        let (built, missing) = t.apps.build_quiet(&t.style);
        assert_eq!(missing, Some("gone"));
        let r = built.recipes[0]
            .clone()
            .downcast::<<ButtonVariant as AppEnum>::Recipes>()
            .expect("button recipes");
        let ghost = &t.style.button_variant_ghost;
        assert_eq!(r.button.text, ghost.text);
        assert_eq!(r.button.bg.get(State::NONE), Some(P(BRAND)));
    }

    #[test]
    #[should_panic(expected = "is not defined in the theme")]
    fn an_edit_with_an_undefined_colour_fails_a_debug_assert() {
        let t = Theme::vega().variant(DANGER_GHOST, ButtonVariant::Ghost, |r, _| {
            r.button.text.set(P(BRAND))
        });
        let _ = t.resolve(ButtonVariant::from(DANGER_GHOST));
    }

    #[test]
    fn theme_equality_compares_entries_not_the_cache() {
        let (a, b) = (themed(), themed());
        let _ = a.resolve(ButtonVariant::from(DANGER_GHOST));
        assert_eq!(a, b);
        let other = Theme::vega().variant(DANGER_GHOST, ButtonVariant::Ghost, |r, _| {
            r.button.text.set(P(Role::Primary))
        });
        assert_ne!(
            Theme::vega().variant(DANGER_GHOST, ButtonVariant::Ghost, danger_ghost),
            other
        );
        assert_ne!(
            Theme::vega(),
            Theme::vega().color(BRAND, egui::Color32::RED, egui::Color32::RED)
        );
        // A later definition of a name replaces the earlier one.
        let replaced = other.variant(DANGER_GHOST, ButtonVariant::Ghost, danger_ghost);
        assert_eq!(
            replaced,
            Theme::vega().variant(DANGER_GHOST, ButtonVariant::Ghost, danger_ghost)
        );
    }
}
