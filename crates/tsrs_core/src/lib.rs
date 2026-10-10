pub mod arena;
pub mod ptr;
#[cfg(compressed_ptrs)]
pub mod reserve;
/// Whether `P<T>` is a 32-bit handle (`cfg(compressed_ptrs)`, see build.rs) rather than a reference.
pub const COMPRESSED_PTRS: bool = cfg!(compressed_ptrs);
/// True on targets that cannot start threads (wasm32 without atomics, such as `wasm32-wasip1`, where
/// `std::thread::spawn` returns `Unsupported`). It gates every place that would spawn even in a single-threaded run:
/// the rayon worker pool runs on the calling thread, `Program::single_threaded()` is always true, the driver forces
/// `--singleThreaded` for tsc and `tsc -b`, and `tsc -b` builds and reports on the calling thread. False on native
/// targets, where the branches it guards fold away.
pub const NO_THREADS: bool = cfg!(all(target_family = "wasm", not(target_feature = "atomics")));
#[cfg(feature = "alloc-profile")]
pub mod alloc_profile;
pub use ptr::{alloc, alloc_slice, alloc_slice_aligned4, alloc_slice_recycled, alloc_slice_scratch, alloc_str, alloc_str_scratch, alloc_vec, alloc_vec_scratch, alloc_profile_dump, arena_checkpoint, arena_pin, arena_rewindable, census_layout, census_recording, census_reset, census_scrub_none, census_scrub_slack, census_scrub_stack, CensusField, arena_rewind, free_raw, free_slice_ptr, OptionSliceCell, OptionThinSliceCell, PackedStr, PSliceCell, SliceCell, StrCell, SlicePair, StaticSlicePtr, ThinSlice, ThinSliceCell, PKey, PSlot, P, PACK_BITS, SP};

mod frozen;
pub use frozen::{FrozenCell, FrozenRef, FrozenRefMut, OwnedCell, OwnedPSliceCell, OwnedSliceCell, OwnedStrCell, OwnedTaggedStrCell};

pub mod collections;
pub mod compat;
pub mod debug;
pub mod glob;
pub mod goslices;
pub mod jsnum;
pub mod json;
pub mod lazymembers;
pub mod memsplit;
pub mod phases;
pub mod festats;
pub mod semver;
pub mod sitecount;
pub mod stringutil;
pub mod tspath;
pub mod utf8;

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
