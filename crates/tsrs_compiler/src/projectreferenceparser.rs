use std::sync::{Arc, OnceLock};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::collections::SyncMap;
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_tsoptions::ParsedCommandLine;

use crate::host::CompilerHost;
use crate::projectreferencefilemapper::projectReferenceFileMapper;

pub(crate) type projectReferenceParseTaskRef = Arc<projectReferenceParseTask>;

pub(crate) struct projectReferenceParseTask {
    config_name: String,
    resolved: OnceLock<Option<P<ParsedCommandLine>>>,
    sub_tasks: OnceLock<Vec<projectReferenceParseTaskRef>>,
}

// projectreferenceparser.go:13
pub(crate) struct projectReferenceParser<'a> {
    host: &'a dyn CompilerHost,
    single_threaded: bool,
    tasks_by_file_name: SyncMap<Path, projectReferenceParseTaskRef>,
}

impl<'a> projectReferenceParser<'a> {
    pub(crate) fn new(host: &'a dyn CompilerHost, single_threaded: bool) -> projectReferenceParser<'a> {
        projectReferenceParser { host, single_threaded, tasks_by_file_name: SyncMap::default() }
    }

    fn to_path(&self, file_name: &str) -> Path {
        tspath::to_path(file_name, self.host.get_current_directory(), self.host.fs().use_case_sensitive_file_names())
    }

    // projectreferenceparser.go:19
    fn parse_task(&self, task: &projectReferenceParseTask) -> (Option<P<ParsedCommandLine>>, Vec<projectReferenceParseTaskRef>) {
        let path = self.to_path(&task.config_name);
        let resolved = self.host.get_resolved_project_reference(&task.config_name, path);
        let Some(resolved) = resolved else {
            return (None, Vec::new());
        };
        ParsedCommandLine::parse_input_output_names(resolved);
        let sub_references = resolved.resolved_project_reference_paths();
        let sub_tasks = if sub_references.is_empty() { Vec::new() } else { Self::create_project_reference_parse_tasks(sub_references) };
        (Some(resolved), sub_tasks)
    }

    // projectreferenceparser.go:34
    pub(crate) fn create_project_reference_parse_tasks(project_references: &[String]) -> Vec<projectReferenceParseTaskRef> {
        project_references
            .iter()
            .map(|config_name| {
                Arc::new(projectReferenceParseTask {
                    config_name: config_name.clone(),
                    resolved: OnceLock::new(),
                    sub_tasks: OnceLock::new(),
                })
            })
            .collect()
    }

    // projectreferenceparser.go:48
    pub(crate) fn parse(&self, tasks: &mut [projectReferenceParseTaskRef], mapper: &mut projectReferenceFileMapper) {
        if self.single_threaded {
            let mut queue = Vec::new();
            self.start_single_threaded(tasks, &mut queue);
            while let Some(task) = queue.pop() {
                let (resolved, mut sub_tasks) = self.parse_task(&task);
                self.start_single_threaded(&mut sub_tasks, &mut queue);
                assert!(task.resolved.set(resolved).is_ok());
                assert!(task.sub_tasks.set(sub_tasks).is_ok());
            }
        } else {
            crate::program::worker_pool().install(|| {
                rayon::scope(|scope| self.start(scope, tasks));
            });
        }
        self.init_mapper(tasks, mapper);
    }

    // projectreferenceparser.go:54
    fn start<'scope>(&'scope self, scope: &rayon::Scope<'scope>, tasks: &mut [projectReferenceParseTaskRef]) {
        for task_slot in tasks.iter_mut() {
            let task = Arc::clone(task_slot);
            let path = self.to_path(&task.config_name);
            let (loaded_task, loaded) = self.tasks_by_file_name.load_or_store(path, Arc::clone(&task));
            if loaded {
                // dedup tasks to ensure correct file order, regardless of which task would be started first
                *task_slot = loaded_task;
            } else {
                scope.spawn(move |scope| {
                    let (resolved, mut sub_tasks) = self.parse_task(&task);
                    self.start(scope, &mut sub_tasks);
                    assert!(task.resolved.set(resolved).is_ok());
                    assert!(task.sub_tasks.set(sub_tasks).is_ok());
                });
            }
        }
    }

    fn start_single_threaded(&self, tasks: &mut [projectReferenceParseTaskRef], queue: &mut Vec<projectReferenceParseTaskRef>) {
        for task_slot in tasks.iter_mut() {
            let task = Arc::clone(task_slot);
            let path = self.to_path(&task.config_name);
            let (loaded_task, loaded) = self.tasks_by_file_name.load_or_store(path, Arc::clone(&task));
            if loaded {
                // dedup tasks to ensure correct file order, regardless of which task would be started first
                *task_slot = loaded_task;
            } else {
                queue.push(task);
            }
        }
    }

    // projectreferenceparser.go:68
    fn init_mapper(&self, tasks: &[projectReferenceParseTaskRef], mapper: &mut projectReferenceFileMapper) {
        let total_references = self.tasks_by_file_name.size() + 1;
        mapper.config_to_project_reference = FxHashMap::with_capacity_and_hasher(total_references, rustc_hash::FxBuildHasher);
        mapper.references_in_config_file = FxHashMap::with_capacity_and_hasher(total_references, rustc_hash::FxBuildHasher);
        mapper.source_to_project_reference = FxHashMap::default();
        mapper.output_dts_to_project_reference = FxHashMap::default();
        let root = self.init_mapper_worker(tasks, &mut FxHashSet::default(), mapper);
        mapper.references_in_config_file.insert(mapper.root_config_path(), root);
    }

    // projectreferenceparser.go:78
    fn init_mapper_worker(
        &self,
        tasks: &[projectReferenceParseTaskRef],
        seen: &mut FxHashSet<usize>,
        mapper: &mut projectReferenceFileMapper,
    ) -> Vec<Path> {
        if tasks.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::with_capacity(tasks.len());
        for task in tasks {
            let t = task.as_ref();
            let path = self.to_path(&t.config_name);
            results.push(path.clone());
            // ensure we only walk each task once
            if !seen.insert(Arc::as_ptr(task) as usize) {
                continue;
            }
            let resolved = *t.resolved.get().expect("parsed project-reference task");
            mapper.config_to_project_reference.insert(path.clone(), resolved);
            if let Some(resolved) = resolved {
                if mapper.config.config_file != resolved.config_file {
                    // Map current task's files first, before recursing into subtasks.
                    // This matches TypeScript's behavior where child project references
                    // overwrite parent entries when a file belongs to multiple projects.
                    if let Some(m) = resolved.source_to_project_reference() {
                        mapper.source_to_project_reference.extend(m.iter().map(|(k, v)| (k.clone(), *v)));
                    }
                    if let Some(m) = resolved.output_dts_to_project_reference() {
                        mapper.output_dts_to_project_reference.extend(m.iter().map(|(k, v)| (k.clone(), *v)));
                    }
                    if mapper.use_source_of_project_reference {
                        let options = resolved.compiler_options().unwrap();
                        let mut decl_dir = options.declaration_dir.clone();
                        if decl_dir.is_empty() {
                            decl_dir.clone_from(&options.out_dir);
                        }
                        if !decl_dir.is_empty() {
                            mapper.dts_directories.insert(self.to_path(&decl_dir));
                        }
                    }
                }
            }
            let references_in_config = self.init_mapper_worker(t.sub_tasks.get().expect("parsed project-reference subtasks"), seen, mapper);
            mapper.references_in_config_file.insert(path, references_in_config);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use tsrs_ast::{SourceFile, SourceFileParseOptions};
    use tsrs_contentmapper::{self as contentmapper, Mapper, Project};
    use tsrs_core::tspath::Path;
    use tsrs_core::P;
    use tsrs_diagnostics::Message;
    use tsrs_tsoptions::ParsedCommandLine;
    use tsrs_vfs::FS;

    use super::projectReferenceParser;
    use crate::host::CompilerHost;

    struct OverlapHost {
        calls: AtomicUsize,
        overlapped: AtomicBool,
    }

    impl CompilerHost for OverlapHost {
        fn fs(&self) -> &dyn FS {
            tsrs_vfs::osvfs::fs()
        }

        fn default_library_path(&self) -> &'static str {
            ""
        }

        fn get_current_directory(&self) -> &'static str {
            "/"
        }

        fn trace(&self, _msg: &'static Message, _args: &[&dyn std::fmt::Display]) {}

        fn get_source_file(&self, _opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
            unreachable!()
        }

        fn get_content_mapped_source_files(
            &self,
            _parse_options: SourceFileParseOptions,
            _mapper: &Mapper,
        ) -> Result<contentmapper::SourceFiles, contentmapper::Error> {
            unreachable!()
        }

        fn content_mapper_project(&self) -> Option<std::sync::Arc<dyn Project>> {
            None
        }

        fn get_resolved_project_reference(&self, _file_name: &str, _path: Path) -> Option<P<ParsedCommandLine>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                let deadline = Instant::now() + Duration::from_secs(1);
                while self.calls.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
                    std::thread::yield_now();
                }
            }
            if self.calls.load(Ordering::SeqCst) >= 2 {
                self.overlapped.store(true, Ordering::SeqCst);
            }
            None
        }
    }

    #[test]
    fn project_reference_tasks_run_concurrently() {
        let host = OverlapHost { calls: AtomicUsize::new(0), overlapped: AtomicBool::new(false) };
        let parser = projectReferenceParser::new(&host, false);
        let mut tasks = projectReferenceParser::create_project_reference_parse_tasks(&["/a/tsconfig.json".into(), "/b/tsconfig.json".into()]);
        rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap().scope(|scope| parser.start(scope, &mut tasks));
        assert!(host.overlapped.load(Ordering::SeqCst), "project references were parsed sequentially");
        assert!(tasks.iter().all(|task| task.resolved.get().is_some() && task.sub_tasks.get().is_some()));
    }
}
