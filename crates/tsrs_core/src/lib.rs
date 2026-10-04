pub mod arena;
pub mod ptr;
#[cfg(feature = "compressed-ptrs")]
pub mod reserve;
/// Whether `P<T>` is a 32-bit handle (feature `compressed-ptrs`) rather than a reference.
pub const COMPRESSED_PTRS: bool = cfg!(feature = "compressed-ptrs");
#[cfg(feature = "alloc-profile")]
pub mod alloc_profile;
pub use ptr::{alloc, alloc_slice, alloc_slice_aligned4, alloc_slice_recycled, alloc_str, alloc_vec, alloc_profile_dump, arena_checkpoint, arena_pin, census_layout, census_recording, census_reset, census_scrub_none, census_scrub_slack, census_scrub_stack, CensusField, arena_rewind, free_raw, OptionSliceCell, OptionThinSliceCell, PackedStr, SliceCell, StrCell, SlicePair, StaticSlicePtr, ThinSlice, ThinSliceCell, PKey, PSlot, P, SP};

mod frozen;
pub use frozen::{FrozenCell, FrozenRef, FrozenRefMut, OwnedCell, OwnedPSliceCell, OwnedSliceCell, OwnedStrCell, OwnedTaggedStrCell};

pub mod collections;
pub mod debug;
pub mod glob;
pub mod goslices;
pub mod jsnum;
pub mod json;
pub mod lazymembers;
pub mod phases;
pub mod semver;
pub mod sitecount;
pub mod stringutil;
pub mod tspath;

mod bfs;
mod binarysearch;
mod buildoptions;
mod compileroptions;
pub mod context;
#[path = "core.rs"]
mod core_go;
mod languagevariant;
mod linkstore;
mod nodemodules;
mod pattern;
mod projectreference;
mod scriptkind;
mod stack;
mod text;
mod textchange;
mod tristate;
mod typeacquisition;
mod version;
mod watchoptions;
mod workgroup;

pub use bfs::*;
pub use binarysearch::*;
pub use buildoptions::*;
pub use compileroptions::*;
pub use context::*;
pub use core_go::*;
pub use languagevariant::*;
pub use linkstore::*;
pub use nodemodules::*;
pub use pattern::*;
pub use projectreference::*;
pub use scriptkind::*;
pub use stack::*;
pub use text::*;
pub use textchange::*;
pub use tristate::*;
pub use typeacquisition::*;
pub use version::*;
pub use watchoptions::*;
pub use workgroup::*;
