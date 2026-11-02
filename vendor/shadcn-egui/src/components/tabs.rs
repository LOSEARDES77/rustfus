//! shadcn Tabs, ported from shadcn-svelte `ui/tabs/*.svelte` at
//! `tools/stylegen/PIN`. Every value comes from the style's `cn-tabs-list`,
//! `cn-tabs-list-variant-*`, `cn-tabs-trigger` recipes and the trigger's
//! `after:` recipe (the line indicator). Behaviour follows bits-ui Tabs: a press focuses and selects,
//! Left/Right move the selection (and wrap), Home/End jump to the first/last
//! tab (automatic activation). The list is one Tab stop, on the selected
//! trigger (roving focus).

use crate::color::Tone;
use crate::components::shared::{
    interaction_state, line_width, part_painter, pressable, transitioned_opacity,
    transitioned_text, ActivationKeys, RecipeBox,
};
use crate::components::text::TextLook;
use crate::style::app::Sealed;
use crate::style::{
    pick, pick_len, AppEnum, AppVariant, Paint, Prop, Recipe, Role, State, Style, VariantEnum,
};
use crate::{layout, Theme};
use egui::{pos2, vec2, Key, Modifiers, Rect, Response, Sense, TextWrapMode, Ui, Widget};

/// shadcn `TabsList` `variant`, or an app variant ([`TabsVariant::app`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabsVariant {
    /// A muted track with the active trigger as a raised pill.
    #[default]
    Default,
    /// No track; the active trigger gets an underline.
    Line,
    /// An app variant: [`TabsVariant::app`].
    #[doc(hidden)]
    App(AppVariant<TabsVariant>),
}

impl TabsVariant {
    /// An app variant, defined in the theme ([`Theme::variant`]).
    pub const fn app(name: &'static str) -> AppVariant<TabsVariant> {
        AppVariant::new(name)
    }

    /// The list's variant layer (`cn-tabs-list-variant-*`) of a built-in
    /// variant.
    fn layer(self, s: &Style) -> &Recipe {
        match self {
            TabsVariant::Default | TabsVariant::App(_) => &s.tabs_list_variant_default,
            TabsVariant::Line => &s.tabs_list_variant_line,
        }
    }
}

impl From<AppVariant<TabsVariant>> for TabsVariant {
    fn from(app: AppVariant<TabsVariant>) -> Self {
        TabsVariant::App(app)
    }
}

/// The tabs' recipes an app variant's edit changes: copies of the base's.
/// The trigger and its line take their look from the base's state bits
/// (`group-data-[variant=line]/tabs-list`), so they are copies of the
/// part recipes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TabsRecipes {
    /// `data-slot="tabs-list"`: the variant's layer.
    pub tabs_list: Recipe,
    /// `data-slot="tabs-trigger"`.
    pub tabs_trigger: Recipe,
    /// The trigger's `::after`: the line under the active tab.
    pub tabs_trigger_after: Recipe,
}

impl Sealed for TabsVariant {}
impl AppEnum for TabsVariant {
    type Recipes = TabsRecipes;
    fn app_name(self) -> Option<&'static str> {
        match self {
            TabsVariant::App(a) => Some(a.name()),
            _ => None,
        }
    }
    fn parts(r: &mut TabsRecipes) -> Vec<&mut crate::style::Recipe> {
        vec![
            &mut r.tabs_list,
            &mut r.tabs_trigger,
            &mut r.tabs_trigger_after,
        ]
    }
    fn recipes(self, style: &Style) -> TabsRecipes {
        TabsRecipes {
            tabs_list: self.layer(style).clone(),
            tabs_trigger: (*style.tabs_trigger).clone(),
            tabs_trigger_after: (*style.tabs_trigger_after).clone(),
        }
    }
    fn bits(self) -> State {
        match self {
            TabsVariant::Line => State::VARIANT_LINE,
            _ => State::VARIANT_DEFAULT,
        }
    }
}
impl VariantEnum for TabsVariant {}

/// shadcn `Tabs` `orientation`. The crate offers `Horizontal` only: a vertical
/// list needs a content layout beside the list that the crate does not have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabsOrientation {
    #[default]
    Horizontal,
}

/// A tab list (shadcn `TabsList` with its `TabsTrigger`s). By default the list
/// is as wide as its triggers and each trigger as wide as its label;
/// [`Tabs::width`] makes it wider. The response is the
/// list's, marked changed when the selection moved. Content below the list
/// sits [`Tabs::content_gap`] under it (shadcn `Tabs` root `gap`).
pub struct Tabs<'a> {
    active: &'a mut usize,
    labels: Vec<String>,
    variant: TabsVariant,
    orientation: TabsOrientation,
    enabled: bool,
    width: Option<f32>,
}

impl<'a> Tabs<'a> {
    /// `active` is the selected index into `labels`.
    pub fn new(active: &'a mut usize, labels: &[&str]) -> Self {
        Self {
            active,
            labels: labels.iter().map(|l| (*l).to_owned()).collect(),
            variant: TabsVariant::Default,
            orientation: TabsOrientation::Horizontal,
            enabled: true,
            width: None,
        }
    }
    /// shadcn `TabsList` `variant`, or an app variant ([`TabsVariant::app`]).
    pub fn variant(mut self, v: impl Into<TabsVariant>) -> Self {
        self.variant = v.into();
        self
    }
    pub fn orientation(mut self, o: TabsOrientation) -> Self {
        self.orientation = o;
        self
    }
    /// `false` draws every trigger disabled: no hover, no focus, no selecting.
    pub fn enabled(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }
    /// List width in points, padding included (shadcn `class="w-full"`
    /// on `TabsList`). Pass `f32::INFINITY` for full-width. The width is
    /// capped to the available width and never smaller than the triggers'
    /// own; the triggers share it as their `flex-1` does: equal parts, and a
    /// trigger whose label needs more keeps its own width. A NaN, zero or
    /// negative width is ignored, and so is `f32::INFINITY` in a `Ui` of
    /// unbounded width: the list then stays as wide as its triggers. In a
    /// `ScrollArea`, `f32::INFINITY` fills the visible width.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }
    /// The gap between the list and the content below it (shadcn `Tabs` root `gap`).
    pub fn content_gap(ui: &Ui) -> f32 {
        let t = Theme::current(ui.ctx());
        pick_len(
            &[&*t.style.tabs],
            |r| &r.gap,
            t.state(ui.ctx(), State::NONE),
        )
    }
}

impl Widget for Tabs<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let t = Theme::current(ui.ctx());
        let s = &t.style;
        let enabled = self.enabled && ui.is_enabled();
        let v = t.resolve(self.variant);
        let variant_bit = v.bits();
        let TabsOrientation::Horizontal = self.orientation;
        let root = [&*s.tabs];
        let list = [
            &*s.tabs_list,
            v.part(s, |b, s| b.layer(s), |r| &r.tabs_list),
        ];
        let trigger = [v.part(s, |_, s| &s.tabs_trigger, |r| &r.tabs_trigger)];
        let after = [v.part(s, |_, s| &s.tabs_trigger_after, |r| &r.tabs_trigger_after)];

        // Geometry depends neither on the selection nor on interaction.
        let g = t.state(ui.ctx(), State::HORIZONTAL.or(variant_bit));
        let geo = |l: &[&Recipe], f: fn(&Recipe) -> &Prop<f32>| pick_len(l, f, g);
        let (pad_t, pad_r, pad_b, pad_l) = (
            geo(&list, |r| &r.pad_top),
            geo(&list, |r| &r.pad_right),
            geo(&list, |r| &r.pad_bottom),
            geo(&list, |r| &r.pad_left),
        );
        let (tpad_l, tpad_r, border_w) = (
            geo(&trigger, |r| &r.pad_left),
            geo(&trigger, |r| &r.pad_right),
            line_width(
                geo(&trigger, |r| &r.border_width),
                ui.ctx().pixels_per_point(),
            ),
        );
        let gap = geo(&list, |r| &r.gap);
        // The text colour depends on the state: lay out in egui's placeholder
        // colour, paint in the final one.
        let look = TextLook::resolve(ui, &trigger, g);
        let galleys: Vec<_> = self
            .labels
            .iter()
            .map(|l| look.layout(ui, l.as_str(), TextWrapMode::Extend, f32::INFINITY))
            .collect();
        // No list height: CSS's initial `auto`, the triggers' own height,
        // which `h-[calc(100%-1px)]` of an `auto` parent leaves at `auto` too.
        // A trigger's content is its line box.
        let trigger_auto = look.line_height()
            + geo(&trigger, |r| &r.pad_top)
            + geo(&trigger, |r| &r.pad_bottom)
            + border_w * 2.0;
        let (list_h, trigger_h) = match pick(&list, |r| &r.height, g) {
            Some(h) => (h, h - pad_t - pad_b - geo(&trigger, |r| &r.height_inset)),
            None => (trigger_auto + pad_t + pad_b, trigger_auto),
        };
        let widths: Vec<f32> = galleys
            .iter()
            .map(|g| g.size().x + tpad_l + tpad_r + border_w * 2.0)
            .collect();
        let n = widths.len();
        let chrome = gap * n.saturating_sub(1) as f32 + pad_l + pad_r;
        let hug_w = widths.iter().sum::<f32>() + chrome;
        // A NaN width, one below the triggers' own, or an unbounded `Ui`
        // leaves the list as wide as its triggers. Without a width, a
        // justified layout gives the list its width, as egui's `Button`
        // takes it (conformance 3.1), and the triggers share it (`flex-1`).
        let list_w = match self.width {
            Some(w) => layout::capped_width(ui, w, hug_w),
            None if ui.layout().horizontal_justify() => {
                layout::capped_width(ui, f32::INFINITY, hug_w)
            }
            None => hug_w,
        };
        let widths = if list_w > hug_w {
            flex_shares(&widths, list_w - chrome)
        } else {
            widths
        };

        // Wider than its room, the list overflows as `w-fit` does in the
        // browser: the column keeps its width.
        let (list_rect, mut resp) =
            layout::allocate_overflowing(ui, vec2(list_w, list_h), Sense::hover());
        let ids: Vec<egui::Id> = (0..n).map(|i| resp.id.with(("tab", i))).collect();
        let mut x = list_rect.left() + pad_l;
        let trigger_rects: Vec<Rect> = widths
            .iter()
            .map(|w| {
                let r = Rect::from_min_size(
                    pos2(x, list_rect.center().y - trigger_h / 2.0),
                    vec2(*w, trigger_h),
                );
                x += w + gap;
                r
            })
            .collect();
        // Roving focus: the list is one Tab stop, on the selected trigger.
        // A trigger that holds the focus keeps it, so the focus survives
        // the app moving the selection.
        let resps: Vec<Response> = trigger_rects
            .iter()
            .zip(&ids)
            .enumerate()
            .map(|(i, (r, id))| {
                let sense = if !enabled {
                    Sense::hover()
                } else if i == *self.active || ui.memory(|m| m.has_focus(*id)) {
                    Sense::click()
                } else {
                    Sense::CLICK
                };
                let mut resp = ui.interact(*r, *id, sense);
                pressable(&mut resp, ActivationKeys::EnterAndSpace);
                resp
            })
            .collect();

        // Selection: a click, focus (bits' automatic activation, so a press
        // selects as it focuses), or the arrow/Home/End keys on a focused
        // trigger, which move the focus along.
        let mut next = None;
        let mut by_key = false;
        if enabled {
            for (i, r) in resps.iter().enumerate() {
                if r.clicked() || r.gained_focus() {
                    next = Some(i);
                }
                if r.has_focus() {
                    // Keep egui from using the arrows to move focus on its own.
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            r.id,
                            egui::EventFilter {
                                horizontal_arrows: true,
                                ..Default::default()
                            },
                        )
                    });
                    let pressed = |key| ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, key));
                    let key_next = if pressed(Key::ArrowRight) {
                        Some((i + 1) % n)
                    } else if pressed(Key::ArrowLeft) {
                        Some((i + n - 1) % n)
                    } else if pressed(Key::Home) {
                        Some(0)
                    } else if pressed(Key::End) {
                        Some(n - 1)
                    } else {
                        None
                    };
                    if key_next.is_some() {
                        next = key_next;
                        by_key = true;
                    }
                }
            }
        }
        if let Some(i) = next {
            if *self.active != i {
                *self.active = i;
                resp.mark_changed();
            }
            if by_key {
                ui.memory_mut(|m| m.request_focus(ids[i]));
            }
        }
        let active = *self.active;

        // The line indicator may reach below the list (`after:bottom-[-5px]`).
        let reach = (-geo(&after, |r| &r.inset_bottom)).max(0.0);
        for (i, r) in resps.iter().enumerate() {
            r.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    enabled,
                    i == active,
                    &self.labels[i],
                )
            });
        }
        if ui.is_rect_visible(list_rect.expand(reach)) {
            // The list sits in the tabs root; a trigger in the list.
            RecipeBox::resolve(ui.ctx(), &t, &list, g).paint(
                &part_painter(ui, resp.id.with("list"), &[&list[..], &root[..]], g),
                list_rect,
            );
            let inherited = t.role(Role::Foreground); // behaviour-ok: inherits Foreground where neither trigger nor list sets a colour
            let list_text = pick(&list, |r| &r.text, g).map(|p| t.tone(p));

            for (i, (rect, r)) in trigger_rects.iter().zip(&resps).enumerate() {
                let context = State::HORIZONTAL
                    .or(variant_bit)
                    .with(State::SELECTED, i == active);
                let st = interaction_state(&t, r, context.with(State::DISABLED, !enabled));
                let painter = part_painter(ui, r.id, &[&trigger[..], &list[..], &root[..]], st);
                let tone = |l: &[&Recipe], f: fn(&Recipe) -> &Prop<Paint>| {
                    pick(l, f, st).map(|p| t.tone(p))
                };
                let px = |l: &[&Recipe], f: fn(&Recipe) -> &Prop<f32>| pick_len(l, f, st);

                let fg = tone(&trigger, |r| &r.text)
                    .or(list_text)
                    .unwrap_or(Tone::of(inherited));
                let fg = transitioned_text(ui.ctx(), r.id, fg, &trigger, st);
                let mut shapes = RecipeBox::resolve(ui.ctx(), &t, &trigger, st)
                    .transitioned(ui.ctx(), r.id, &trigger, st)
                    .shapes_at(ui.ctx(), *rect, painter.opacity());
                // The `after:` line indicator, placed against the padding box.
                // The painter already carries the trigger's chain.
                let bar_alpha =
                    transitioned_opacity(ui.ctx(), r.id.with("after"), &[&after[..]], st);
                if bar_alpha > 0.0 {
                    let inner = rect.shrink(border_w);
                    let bottom = inner.bottom() - px(&after, |r| &r.inset_bottom);
                    let bar = Rect::from_min_max(
                        pos2(
                            inner.left() + px(&after, |r| &r.inset_left),
                            bottom - px(&after, |r| &r.height),
                        ),
                        pos2(inner.right() - px(&after, |r| &r.inset_right), bottom),
                    );
                    shapes.extend(
                        RecipeBox::resolve(ui.ctx(), &t, &after, st)
                            .faded(bar_alpha)
                            .shapes_at(ui.ctx(), bar, painter.opacity()),
                    );
                }
                painter.extend(shapes);
                let galley = &galleys[i];
                galley.paint(&painter, rect.center() - galley.size() / 2.0, fg);
            }
        }
        resp
    }
}

/// The trigger widths of a list `inner` wide (padding and gaps taken off),
/// as CSS lays out `flex-1` items: equal parts of `inner`, and an item whose
/// content is wider than its part keeps its content width while the others
/// share the rest.
fn flex_shares(content: &[f32], inner: f32) -> Vec<f32> {
    let mut frozen = vec![false; content.len()];
    loop {
        let open = frozen.iter().filter(|f| !**f).count();
        if open == 0 {
            return content.to_vec();
        }
        let taken: f32 = content
            .iter()
            .zip(&frozen)
            .filter(|(_, f)| **f)
            .map(|(c, _)| c)
            .sum();
        let share = (inner - taken) / open as f32;
        let mut froze = false;
        for (c, f) in content.iter().zip(frozen.iter_mut()) {
            if !*f && *c > share {
                *f = true;
                froze = true;
            }
        }
        if !froze {
            return content
                .iter()
                .zip(&frozen)
                .map(|(c, f)| if *f { *c } else { share })
                .collect();
        }
    }
}

/// Renders a tab bar in the default variant. `active` is the selected index.
/// Returns true if it changed.
pub fn tab_bar(ui: &mut Ui, active: &mut usize, labels: &[&str]) -> bool {
    ui.add(Tabs::new(active, labels)).changed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app variant's edit gets `TabsRecipes`, keeps its base's bits and
    /// changes only what it names.
    #[test]
    fn edit_compiles_and_changes_its_part() {
        const BRAND_LINE: AppVariant<TabsVariant> = TabsVariant::app("brand-line");
        let t = Theme::vega().variant(BRAND_LINE, TabsVariant::Line, |r: &mut TabsRecipes, _| {
            r.tabs_trigger_after
                .recolor(Role::Foreground, Role::Primary)
        });
        let v = t.resolve(TabsVariant::from(BRAND_LINE));
        assert_eq!(v.bits(), State::VARIANT_LINE);
        let s = &t.style;
        let list = v.part(s, |b, s| b.layer(s), |r| &r.tabs_list);
        assert_eq!(list, &*s.tabs_list_variant_line);
        let trigger = v.part(s, |_, s| &s.tabs_trigger, |r| &r.tabs_trigger);
        assert_eq!(trigger, &*s.tabs_trigger);
        let after = v.part(s, |_, s| &s.tabs_trigger_after, |r| &r.tabs_trigger_after);
        assert_ne!(after, &*s.tabs_trigger_after);
        let mut want = (*s.tabs_trigger_after).clone();
        want.recolor(Role::Foreground, Role::Primary);
        assert_eq!(after, &want);
    }
}
