// Go package `transformers/tstransforms` (the legacy decorator transforms; typeeraser, importelision,
// runtimesyntax and utilities belong to emit/transforms).

mod legacydecorators;
mod metadata;
mod typeserializer;

pub use legacydecorators::*;
pub use metadata::*;
pub use typeserializer::*;
