//! The parent context (rules concept 5, implementation notes 12): a crate
//! parent tags the `Ui` it builds for its children with a value, and a crate
//! widget inside reads the nearest one from `ui.stack()`. The value's type is
//! the key, so two parents never read each other's context; the tag lives as
//! long as the parent's `Ui`, one pass.
//!
//! A `Ui` tagged with [`push`] or [`info`] is an element of its own, as the
//! upstream element it stands for: a group's child is a direct child only
//! when no element sits between them ([`is_element`]). Plain egui scopes
//! carry no tag and stay transparent, as does the text a container hands
//! down ([`inherited_info`]).

use egui::{Ui, UiBuilder, UiStack, UiStackInfo};
use std::any::{type_name, Any};
use std::sync::Arc;

fn key<T: Any>() -> &'static str {
    type_name::<T>()
}

/// `builder` with `value` as the context of type `T` of the `Ui` it builds,
/// beside the tags it already carries.
pub(crate) fn push<T: Any + Send + Sync>(mut builder: UiBuilder, value: T) -> UiBuilder {
    builder.ui_stack_info = info(builder.ui_stack_info, value);
    builder
}

/// The tag that marks a `Ui` as a crate element.
const ELEMENT: &str = "shadcn_egui::element";

/// `info` with `value` as the context of type `T`, marked as an element.
pub(crate) fn info<T: Any + Send + Sync>(info: UiStackInfo, value: T) -> UiStackInfo {
    let mut info = inherited_info(info, value);
    info.tags.insert(ELEMENT, None);
    info
}

/// `info` with `value` as the context of type `T`, for a `Ui` that only
/// passes on what its element set (inherited text): no element of its own.
pub(crate) fn inherited_info<T: Any + Send + Sync>(mut info: UiStackInfo, value: T) -> UiStackInfo {
    info.tags.insert(key::<T>(), Some(Arc::new(value)));
    info
}

/// Whether this one frame of the stack is a crate element.
pub(crate) fn is_element(frame: &UiStack) -> bool {
    frame.tags().contains(ELEMENT)
}

/// The context of type `T` on this one frame of the stack, not its parents'.
pub(crate) fn on_frame<T: Any + Send + Sync>(frame: &UiStack) -> Option<&T> {
    frame.tags().get_downcast::<T>(key::<T>())
}

/// The nearest context of type `T` around `ui`, `ui`'s own included.
pub(crate) fn find<T: Any + Clone + Send + Sync>(ui: &Ui) -> Option<T> {
    ui.stack().iter().find_map(|f| on_frame::<T>(f).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq)]
    struct A(u8);
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct B(u8);

    #[test]
    fn the_nearest_context_of_the_type_wins() {
        let ctx = egui::Context::default();
        let _ = crate::test_ui::run(&ctx, egui::RawInput::default(), |ui| {
            assert_eq!(find::<A>(ui), None);
            ui.scope_builder(push(push(UiBuilder::new(), A(1)), B(9)), |ui| {
                ui.scope_builder(push(UiBuilder::new(), A(2)), |ui| {
                    assert_eq!(find::<A>(ui), Some(A(2)));
                    // Another type's context passes through.
                    assert_eq!(find::<B>(ui), Some(B(9)));
                    ui.horizontal(|ui| assert_eq!(find::<A>(ui), Some(A(2))));
                });
                assert_eq!(find::<A>(ui), Some(A(1)));
            });
        });
    }
}
