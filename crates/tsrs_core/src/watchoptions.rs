use crate::Tristate;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchOptions {
    pub interval: Option<i32>,
    pub file_kind: WatchFileKind,
    pub directory_kind: WatchDirectoryKind,
    pub fallback_polling: PollingKind,
    pub sync_watch_dir: Tristate,
    pub exclude_dir: Option<Vec<String>>,
    pub exclude_files: Option<Vec<String>>,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum WatchFileKind {
    #[default]
    None = 0,
    FixedPollingInterval = 1,
    PriorityPollingInterval = 2,
    DynamicPriorityPolling = 3,
    FixedChunkSizePolling = 4,
    UseFsEvents = 5,
    UseFsEventsOnParentDirectory = 6,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum WatchDirectoryKind {
    #[default]
    None = 0,
    UseFsEvents = 1,
    FixedPollingInterval = 2,
    DynamicPriorityPolling = 3,
    FixedChunkSizePolling = 4,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum PollingKind {
    #[default]
    None = 0,
    FixedInterval = 1,
    PriorityInterval = 2,
    DynamicPriority = 3,
    FixedChunkSize = 4,
}

impl WatchOptions {
    pub fn watch_interval(w: Option<&WatchOptions>) -> Duration {
        let mut watch_interval = Duration::from_millis(2000);
        if let Some(interval) = w.and_then(|w| w.interval) {
            watch_interval = Duration::from_millis(interval as u64);
        }
        watch_interval
    }
}
