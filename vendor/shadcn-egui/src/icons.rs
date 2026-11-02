//! The icon sets this build bundles, by upstream library id. Lucide is
//! always bundled; every other set needs its `icons-<id>` feature.

/// Hugeicons, free set.
#[cfg(feature = "icons-hugeicons")]
pub use shadcn_egui_icons_hugeicons as hugeicons;
/// Lucide, the default icon set (always available).
pub use shadcn_egui_icons_lucide as lucide;
/// Phosphor Icons, regular weight.
#[cfg(feature = "icons-phosphor")]
pub use shadcn_egui_icons_phosphor as phosphor;
/// Remix Icon, pinned to its last Apache-2.0 release.
#[cfg(feature = "icons-remixicon")]
pub use shadcn_egui_icons_remixicon as remixicon;
/// Tabler Icons, outline set.
#[cfg(feature = "icons-tabler")]
pub use shadcn_egui_icons_tabler as tabler;

use crate::preset::IconLibrary;
use shadcn_egui_core::IconSlots;

/// The slot icons of `lib`, or `None` when its feature is off.
pub fn slots(lib: IconLibrary) -> Option<IconSlots> {
    match lib {
        IconLibrary::Lucide => Some(lucide::SLOTS),
        #[cfg(feature = "icons-tabler")]
        IconLibrary::Tabler => Some(tabler::SLOTS),
        #[cfg(feature = "icons-phosphor")]
        IconLibrary::Phosphor => Some(phosphor::SLOTS),
        #[cfg(feature = "icons-remixicon")]
        IconLibrary::Remixicon => Some(remixicon::SLOTS),
        #[cfg(feature = "icons-hugeicons")]
        IconLibrary::Hugeicons => Some(hugeicons::SLOTS),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// The warning `Preset::theme` logs when [`slots`] has nothing for `lib`.
pub(crate) fn fallback_warning(lib: IconLibrary) -> String {
    let id = lib.id();
    format!("shadcn-egui: icon library \"{id}\" needs the \"icons-{id}\" feature; using \"lucide\"")
}
