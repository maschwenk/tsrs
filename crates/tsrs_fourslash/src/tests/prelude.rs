// What generated code sees: Go package names map to these modules (tools/gen-fourslash/types.go pkgModule).

pub use std::sync::{Arc, LazyLock};

pub use tsrs_core::collections::{self, OrderedMap};
pub use tsrs_core::Tristate;
pub use tsrs_ls::{lsconv, lsutil};
pub use tsrs_lsproto as lsproto;
pub use tsrs_modulespecifiers as modulespecifiers;

pub use crate::fourslash;
pub use crate::go::{self, Any};
pub use crate::ls_shim as ls;
pub use crate::testing::T;
pub use crate::tests::util;
pub use crate::testutil::{self, baseline};
pub use crate::{contentmapper, contentmappertest, stringtestutil};
