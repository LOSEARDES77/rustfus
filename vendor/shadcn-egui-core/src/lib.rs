//! Data types shared by `shadcn-egui` and its font and icon crates.
//!
//! Users never name this crate: `shadcn-egui` re-exports everything in it.

mod blob;
mod font;
mod icon;

pub use blob::{BlobRange, BlobSegs, FillMesh};
pub use font::{Face, FontFiles, FontKind, FontSource, WEIGHTS};
pub use icon::{Icon, IconLayer, IconLayers, IconShape, IconSlots, Seg, SegIter};
