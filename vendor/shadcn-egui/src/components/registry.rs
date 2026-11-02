//! The item registry (conformance 3.2, implementation notes 12): a group
//! whose children are calls in a closure learns about them from the
//! previous pass, kept by the group's id.
//!
//! - The group calls [`Group::begin`] before its closure and builds the
//!   children's `Ui` with [`Group::push`]. Before it draws, it reads the
//!   previous pass's count, ids, sizes, enabled flags, widest width and
//!   presence flags through the handle.
//! - Each child calls [`join`] once it knows its size and gets its
//!   [`Position`]: its index in this pass, the count of the previous pass;
//!   [`Position::state`] gives the `FIRST`, `LAST`, `NOT_FIRST` and
//!   `NOT_LAST` flags.
//! - A part reports that it exists with [`report`] (`HAS_ACTION`,
//!   `HAS_ICON`, ...); the container reads it on the next pass.
//! - [`Group::end`] after the closure stores this pass's list and calls
//!   `request_discard` when anything the readers used differs from what
//!   they read, so the discarded pass is drawn again with the right values.
//!   A still frame never discards.
//!
//! The first pass of an `egui::Area` is a sizing pass: egui builds its `Ui`
//! invisible, and an invisible `Ui` is disabled. A group in a sizing pass
//! keeps nothing of that pass, so its items never reach the visible pass as
//! disabled, and the measuring pass's sizes never replace the drawn ones.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "first users: the alert and button-group ports")
)]

use crate::components::context;
use crate::style::recipe::State;
use egui::{Context, Id, Ui, UiBuilder, Vec2};
use std::cell::Cell;

const SIZE_EPSILON: f32 = 0.01; // behaviour-ok: float tolerance, not a design token

/// One child of a group, as it joined.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Item {
    /// The child's widget id.
    pub id: Id,
    /// The child's size, known before it is placed (single pass).
    pub size: Vec2,
    /// Whether the child is enabled.
    pub enabled: bool,
}

/// What a group learned about its children in one pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    /// The children in call order.
    pub items: Vec<Item>,
    /// The presence flags its parts reported.
    pub presence: State,
}

impl Snapshot {
    fn widest(&self) -> f32 {
        self.items.iter().map(|i| i.size.x).fold(0.0, f32::max)
    }
}

/// A child's place in its group: upstream's `:first-child` and
/// `:last-child`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Position {
    /// The child's index among the children of this pass.
    pub index: usize,
    /// How many children the group had in the previous pass, at least
    /// `index + 1`.
    pub count: usize,
}

impl Position {
    /// The position flags of the child.
    pub(crate) fn state(self) -> State {
        let first = self.index == 0;
        let last = self.index + 1 >= self.count;
        State::NONE
            .with(State::FIRST, first)
            .with(State::NOT_FIRST, !first)
            .with(State::LAST, last)
            .with(State::NOT_LAST, !last)
    }
}

// What the readers of one pass used, so `end` compares only that.
const COUNT: u8 = 1 << 0;
const IDS: u8 = 1 << 1;
const SIZES: u8 = 1 << 2;
const ENABLED: u8 = 1 << 3;
const WIDEST: u8 = 1 << 4;

#[derive(Clone, Default)]
struct Record {
    /// The last visible pass's list.
    stored: Snapshot,
    /// This pass's list so far.
    current: Snapshot,
    /// What the children read this pass.
    uses: u8,
    pass: u64,
}

fn key(group: Id) -> Id {
    group.with("shadcn_egui::item_registry")
}

/// The context tag a group puts on its children's `Ui`.
#[derive(Clone, Copy)]
struct Members(Id);

/// A group's handle for one pass: the previous pass's list, and what this
/// pass read from it.
#[must_use = "call `end` after the group's closure"]
pub(crate) struct Group {
    id: Id,
    sizing: bool,
    last: Snapshot,
    uses: Cell<u8>,
    presence_read: Cell<State>,
}

impl Group {
    /// Start the list of `group` for this pass; `ui` is the group's own
    /// `Ui`, whose sizing pass decides whether the pass is kept.
    pub(crate) fn begin(ui: &Ui, group: Id) -> Self {
        let pass = ui.ctx().cumulative_pass_nr();
        let last = ui.data_mut(|d| {
            let r = d.get_temp_mut_or_default::<Record>(key(group));
            r.current = Snapshot::default();
            r.uses = 0;
            r.pass = pass;
            r.stored.clone()
        });
        Self {
            id: group,
            sizing: ui.is_sizing_pass(),
            last,
            uses: Cell::new(0),
            presence_read: Cell::new(State::NONE),
        }
    }

    /// `builder` with this group as the one its children [`join`].
    pub(crate) fn push(&self, builder: UiBuilder) -> UiBuilder {
        context::push(builder, Members(self.id))
    }

    fn read(&self, what: u8) -> &Snapshot {
        self.uses.set(self.uses.get() | what);
        &self.last
    }

    /// The previous pass's child count.
    pub(crate) fn count(&self) -> usize {
        self.read(COUNT).items.len()
    }

    /// The previous pass's child ids, in call order.
    pub(crate) fn ids(&self) -> Vec<Id> {
        self.read(IDS).items.iter().map(|i| i.id).collect()
    }

    /// The previous pass's child sizes, in call order.
    pub(crate) fn sizes(&self) -> Vec<Vec2> {
        self.read(SIZES).items.iter().map(|i| i.size).collect()
    }

    /// The previous pass's enabled flags, in call order.
    pub(crate) fn enabled(&self) -> Vec<bool> {
        self.read(ENABLED).items.iter().map(|i| i.enabled).collect()
    }

    /// The previous pass's widest child width, 0 without children.
    pub(crate) fn widest(&self) -> f32 {
        self.read(WIDEST).widest()
    }

    /// Those of `flags` that a part reported in the previous pass.
    pub(crate) fn presence(&self, flags: State) -> State {
        self.presence_read.set(self.presence_read.get() | flags);
        let has = self.last.presence;
        has.without(has.without(flags))
    }

    /// End the list of this pass: keep it for the next pass and ask egui
    /// for another pass when anything read differs from it. Returns this
    /// pass's list, for what the group does after its closure.
    pub(crate) fn end(self, ctx: &Context) -> Snapshot {
        let (current, changed) = ctx.data_mut(|d| {
            let r = d.get_temp_mut_or_default::<Record>(key(self.id));
            let current = std::mem::take(&mut r.current);
            if self.sizing {
                return (current, false);
            }
            let uses = r.uses | self.uses.get();
            let changed = differs(&r.stored, &current, uses, self.presence_read.get());
            r.stored = current.clone();
            (current, changed)
        });
        if changed {
            ctx.request_discard("shadcn_egui item registry: a group's children changed");
        }
        current
    }
}

fn differs(old: &Snapshot, new: &Snapshot, uses: u8, presence: State) -> bool {
    let (a, b) = (&old.items, &new.items);
    let len = a.len() != b.len();
    let any = |f: &dyn Fn(&Item, &Item) -> bool| len || a.iter().zip(b).any(|(x, y)| f(x, y));
    (uses & COUNT != 0 && len)
        || (uses & IDS != 0 && any(&|x, y| x.id != y.id))
        || (uses & ENABLED != 0 && any(&|x, y| x.enabled != y.enabled))
        || (uses & SIZES != 0 && any(&|x, y| (x.size - y.size).abs().max_elem() > SIZE_EPSILON))
        || (uses & WIDEST != 0 && (old.widest() - new.widest()).abs() > SIZE_EPSILON)
        || old.presence.without(old.presence.without(presence))
            != new.presence.without(new.presence.without(presence))
}

/// Join the group `ui` belongs to with `item`: its position, or `None`
/// when `ui` is no direct child of a group. A crate element between the
/// group and `ui` (a card, a crate row) makes `ui` that element's child, as
/// upstream's `:first-child` counts direct children only; plain egui
/// scopes in between are layout and pass the group through.
pub(crate) fn join(ui: &Ui, item: Item) -> Option<Position> {
    let Members(group) = ui
        .stack()
        .iter()
        .find_map(|f| {
            context::on_frame::<Members>(f)
                .map(|m| Some(*m))
                .or_else(|| context::is_element(f).then_some(None))
        })
        .flatten()?;
    let pass = ui.ctx().cumulative_pass_nr();
    Some(ui.data_mut(|d| {
        let r = d.get_temp_mut_or_default::<Record>(key(group));
        debug_assert_eq!(r.pass, pass, "registry::join outside its group's pass");
        let index = r.current.items.len();
        r.current.items.push(item);
        r.uses |= COUNT;
        Position {
            index,
            count: r.stored.items.len().max(index + 1),
        }
    }))
}

/// Report that a part with `flags` exists in `group` in this pass.
pub(crate) fn report(ctx: &Context, group: Id, flags: State) {
    ctx.data_mut(|d| {
        let r = d.get_temp_mut_or_default::<Record>(key(group));
        r.current.presence = r.current.presence | flags;
    });
}

/// Those of `flags` that a part of `group` has reported so far in this
/// pass: for a part that must come after another (rules, Part: source
/// order).
pub(crate) fn reported(ctx: &Context, group: Id, flags: State) -> State {
    let has = ctx.data(|d| {
        d.get_temp::<Record>(key(group))
            .map_or(State::NONE, |r| r.current.presence)
    });
    has.without(has.without(flags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{vec2, RawInput};

    /// Run one frame and return how many passes it took.
    fn frame(ctx: &Context, mut ui_fn: impl FnMut(&mut Ui)) -> u64 {
        let before = ctx.cumulative_pass_nr();
        let _ = crate::test_ui::run(&ctx, RawInput::default(), |ui| ui_fn(ui));
        ctx.cumulative_pass_nr() - before
    }

    /// A group of `n` children of width `10 * (i + 1)`: the positions the
    /// last pass of the frame saw, and how many passes the frame took.
    fn group_frame(ctx: &Context, n: usize, width: f32) -> (Vec<State>, u64) {
        let mut seen = Vec::new();
        let passes = frame(ctx, |ui| {
            let group = Group::begin(ui, Id::new("g"));
            seen.clear();
            ui.scope_builder(group.push(UiBuilder::new()), |ui| {
                for i in 0..n {
                    let item = Item {
                        id: Id::new(i),
                        size: vec2(width * (i + 1) as f32, 20.0),
                        enabled: true,
                    };
                    seen.push(join(ui, item).unwrap().state());
                }
            });
            let list = group.end(ui.ctx());
            assert_eq!(list.items.len(), n);
        });
        (seen, passes)
    }

    #[test]
    fn positions_come_from_the_previous_pass_and_a_still_frame_does_not_discard() {
        let ctx = Context::default();
        let first = State::FIRST | State::NOT_LAST;
        let mid = State::NOT_FIRST | State::NOT_LAST;
        let last = State::NOT_FIRST | State::LAST;
        // The first pass learns the count; its discard redraws the frame
        // with the right positions.
        assert_eq!(group_frame(&ctx, 3, 10.0), (vec![first, mid, last], 2));
        // A still frame: one pass.
        assert_eq!(group_frame(&ctx, 3, 10.0), (vec![first, mid, last], 1));
        // A change: one discard, then right again.
        assert_eq!(group_frame(&ctx, 2, 10.0), (vec![first, last], 2));
        assert_eq!(group_frame(&ctx, 2, 10.0).1, 1);
        assert_eq!(
            group_frame(&ctx, 1, 10.0).0,
            vec![State::FIRST | State::LAST]
        );
    }

    #[test]
    fn only_what_a_reader_used_triggers_a_discard() {
        let ctx = Context::default();
        let _ = group_frame(&ctx, 2, 10.0);
        // Positions read the count only: a width change keeps one pass.
        assert_eq!(group_frame(&ctx, 2, 30.0).1, 1);

        // A group that reads the widest width redraws when it changes.
        let widest = |w: f32| {
            let mut seen = 0.0;
            let passes = frame(&ctx, |ui| {
                let group = Group::begin(ui, Id::new("w"));
                seen = group.widest();
                ui.scope_builder(group.push(UiBuilder::new()), |ui| {
                    for (i, w) in [w, 5.0].into_iter().enumerate() {
                        let item = Item {
                            id: Id::new(i),
                            size: vec2(w, 20.0),
                            enabled: true,
                        };
                        let _ = join(ui, item);
                    }
                });
                let _ = group.end(ui.ctx());
            });
            (seen, passes)
        };
        assert_eq!(widest(40.0), (40.0, 2));
        assert_eq!(widest(40.0), (40.0, 1));
        assert_eq!(widest(60.0), (60.0, 2));
    }

    #[test]
    fn ids_sizes_and_count_reach_the_group_on_the_next_pass() {
        let ctx = Context::default();
        let run = |ids: &[u8], height: f32| {
            let mut seen = (0, Vec::new(), Vec::new());
            let passes = frame(&ctx, |ui| {
                let group = Group::begin(ui, Id::new("list"));
                seen = (group.count(), group.ids(), group.sizes());
                ui.scope_builder(group.push(UiBuilder::new()), |ui| {
                    for &i in ids {
                        let item = Item {
                            id: Id::new(i),
                            size: vec2(10.0, height),
                            enabled: true,
                        };
                        let _ = join(ui, item);
                    }
                });
                let _ = group.end(ui.ctx());
            });
            (seen, passes)
        };
        let ((count, ids, sizes), passes) = run(&[1, 2], 20.0);
        assert_eq!(passes, 2);
        assert_eq!(count, 2);
        assert_eq!(ids, vec![Id::new(1u8), Id::new(2u8)]);
        assert_eq!(sizes, vec![vec2(10.0, 20.0); 2]);
        assert_eq!(run(&[1, 2], 20.0).1, 1);
        // Same count, other ids; then other sizes.
        assert_eq!(run(&[1, 3], 20.0).1, 2);
        assert_eq!(run(&[1, 3], 24.0).1, 2);
        assert_eq!(run(&[1, 3], 24.0).1, 1);
    }

    #[test]
    fn presence_flags_reach_the_container_on_the_next_pass() {
        let ctx = Context::default();
        let card = |action: bool, media: bool| {
            let mut seen = State::NONE;
            let passes = frame(&ctx, |ui| {
                let group = Group::begin(ui, Id::new("card"));
                seen = group.presence(State::HAS_ACTION | State::HAS_ICON);
                let parts = State::NONE
                    .with(State::HAS_ACTION, action)
                    .with(State::HAS_MEDIA, media);
                report(ui.ctx(), Id::new("card"), parts);
                let _ = group.end(ui.ctx());
            });
            (seen, passes)
        };
        assert_eq!(card(true, false), (State::HAS_ACTION, 2));
        assert_eq!(card(true, false), (State::HAS_ACTION, 1));
        // A flag no reader asked for never discards.
        assert_eq!(card(true, true), (State::HAS_ACTION, 1));
        assert_eq!(card(false, true), (State::NONE, 2));
        assert_eq!(card(false, true), (State::NONE, 1));
    }

    fn stored(ctx: &Context, group: Id) -> Snapshot {
        ctx.data(|d| d.get_temp::<Record>(key(group)))
            .unwrap_or_default()
            .stored
    }

    #[test]
    fn an_area_sizing_pass_registers_nothing() {
        // Defect 7: an `Area`'s first pass is invisible and disabled.
        let ctx = Context::default();
        let group = Id::new("menu");
        let popup = |seen: &mut Vec<bool>, sizing: &mut bool| {
            frame(&ctx, |ui| {
                egui::Area::new(Id::new("area")).show(ui.ctx(), |ui| {
                    *sizing = ui.is_sizing_pass();
                    let g = Group::begin(ui, group);
                    *seen = g.enabled();
                    ui.scope_builder(g.push(UiBuilder::new()), |ui| {
                        for i in 0..3 {
                            let item = Item {
                                id: Id::new(i),
                                size: vec2(if ui.is_sizing_pass() { 1.0 } else { 50.0 }, 20.0),
                                enabled: ui.is_enabled(),
                            };
                            let _ = join(ui, item);
                        }
                    });
                    let _ = g.end(ui.ctx());
                });
            })
        };
        let (mut seen, mut sizing) = (Vec::new(), false);

        // The sizing frame: disabled items, kept nowhere, no discard.
        assert_eq!(popup(&mut seen, &mut sizing), 1);
        assert!(sizing);
        assert_eq!(stored(&ctx, group), Snapshot::default());

        // The first visible frame learns the enabled items; the discarded
        // pass draws them right.
        assert_eq!(popup(&mut seen, &mut sizing), 2);
        assert!(!sizing);
        assert_eq!(seen, vec![true; 3]);
        let kept = stored(&ctx, group);
        assert!(kept.items.iter().all(|i| i.enabled && i.size.x == 50.0));

        // Then still.
        assert_eq!(popup(&mut seen, &mut sizing), 1);
    }

    #[test]
    fn only_a_direct_child_joins_its_group() {
        let ctx = Context::default();
        let _ = crate::test_ui::run(&ctx, RawInput::default(), |ui| {
            let group = Group::begin(ui, Id::new("direct"));
            let item = |i: u8| Item {
                id: Id::new(i),
                size: Vec2::ZERO,
                enabled: true,
            };
            ui.scope_builder(group.push(UiBuilder::new()), |ui| {
                assert!(join(ui, item(0)).is_some());
                // Plain egui scopes are layout, not elements.
                ui.horizontal(|ui| assert!(join(ui, item(1)).is_some()));
                ui.scope(|ui| ui.vertical(|ui| assert!(join(ui, item(2)).is_some())));
                // Text a container hands down is no element either.
                let look = crate::components::text::TextLook::resolve(ui, &[], State::NONE);
                crate::components::text::content_scope(ui, &look, |ui| {
                    assert!(join(ui, item(3)).is_some());
                });
                // A crate element in between makes the item its child.
                crate::layout::vstack(ui, 0.0, |ui| {
                    assert_eq!(join(ui, item(4)), None);
                    ui.horizontal(|ui| assert_eq!(join(ui, item(5)), None));
                });
                // A group inside the element is the item's group again.
                crate::layout::vstack(ui, 0.0, |ui| {
                    let inner = Group::begin(ui, Id::new("inner"));
                    ui.scope_builder(inner.push(UiBuilder::new()), |ui| {
                        assert_eq!(join(ui, item(6)).map(|p| p.index), Some(0));
                    });
                    assert_eq!(inner.end(ui.ctx()).items.len(), 1);
                });
            });
            assert_eq!(group.end(ui.ctx()).items.len(), 4);
        });
    }

    #[test]
    fn outside_a_group_a_child_has_no_position() {
        let ctx = Context::default();
        let _ = crate::test_ui::run(&ctx, RawInput::default(), |ui| {
            let item = Item {
                id: Id::new(0),
                size: Vec2::ZERO,
                enabled: true,
            };
            assert_eq!(join(ui, item), None);
        });
    }
}
