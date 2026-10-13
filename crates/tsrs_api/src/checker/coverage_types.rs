// Per-method coverage inventory against pinned tsc/internal/api/proto.go (see coverage_table.rs).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// Owned and dispatched by this module.
    Checker,
    /// Language-service backed (find-references / completions / import adder); not owned by this lane.
    #[expect(dead_code, reason = "The protocol coverage vocabulary includes the LS lane even when this checker-only inventory has no LS rows.")]
    Ls,
    /// Owned by another lane (core, codec, runtime, ...).
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not owned by the checker lane; status tracked by the owning lane.
    NotOwned,
    /// Owned but not dispatched yet.
    Planned,
    /// Handler ported and dispatched; exercised only through compilation / shared helpers.
    #[expect(dead_code, reason = "This inventory currently exercises every implemented checker method, but retains the distinct implemented-only status.")]
    Implemented,
    /// Handler ported, dispatched and exercised by a real-program test in checker/tests.rs.
    Tested,
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub method: &'static str,
    pub lane: Lane,
    pub status: Status,
}
