// testutil/contentmappertest/mapper_test.go. The test binary has its own main, Go's TestMain: started with HELPER_ENV
// it is the mapper subprocess and serves the transforming mapper over its stdio, which libtest's own output on stdout
// would corrupt.

use std::io::{self, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use tsrs_contentmapper::{new_host, Definition, Manifest, Mapper, ProjectSpec, Request, Spawner};
use tsrs_contentmappertest::{serve, DECLARED_OPTIONS, PACKAGE_NAME, TRANSFORMING_MAPPER};
use tsrs_core::{CompilerOptions, Locale, ScriptTarget, P};
use tsrs_ipc::{Closer, ReadWriteCloser};

// helperEnv, when set, makes the test binary act as the mapper subprocess instead of running tests. This
// lets the out-of-process test spawn a real subprocess (itself) that speaks the mapper protocol over
// stdio, exercising the same handler code that the in-process spawner runs over a pipe.
// mapper_test.go:21
const HELPER_ENV: &str = "TSGO_CONTENT_MAPPER_HELPER";

// mapper_test.go:23
fn main() {
    if std::env::var(HELPER_ENV).as_deref() == Ok("1") {
        let _ = serve(ReadWriteCloser { reader: Box::new(io::stdin()), writer: Box::new(io::stdout()), closer: Arc::new(stdio) });
        std::process::exit(0);
    }
    test_out_of_process();
    println!("test test_out_of_process ... ok");
}

// stdio adapts the process's stdin/stdout to an io.ReadWriteCloser for the mapper server.
// mapper_test.go:32
struct stdio;

impl Closer for stdio {
    // mapper_test.go:36
    fn close(&self) -> io::Result<()> {
        Ok(())
    }
}

// mapper_test.go:38
fn test_mapper() -> &'static Mapper {
    Box::leak(Box::new(Mapper {
        definition: Definition { package: PACKAGE_NAME.to_string(), extensions: vec![".box".to_string()], ..Default::default() },
        manifest: Manifest {
            name: PACKAGE_NAME.to_string(),
            version: "1.0.0".to_string(),
            exec: vec![TRANSFORMING_MAPPER.to_string()],
            compiler_options: DECLARED_OPTIONS.iter().map(|option| option.to_string()).collect(),
            ..Default::default()
        },
        package_directory: format!("/node_modules/{PACKAGE_NAME}"),
        ..Default::default()
    }))
}

// mapper_test.go:50
fn transform_request() -> Request<'static> {
    Request { file_name: "/app.box", content: "export const version = #{target};\n" }
}

// TestOutOfProcess exercises the real out-of-process IPC path: it spawns the test binary as a mapper
// subprocess and drives it over stdio through the production content mapper host.
// mapper_test.go:59
fn test_out_of_process() {
    let host = new_host(Arc::new(execSpawner), Locale::DEFAULT);
    let mapper = test_mapper();
    let request = transform_request();
    let compiler_options = P::new(CompilerOptions { target: ScriptTarget::ES2020, ..Default::default() });
    let project = host
        .project(ProjectSpec {
            config_file_name: "/tsconfig.json".to_string(),
            mappers: vec![mapper],
            compiler_options: Some(compiler_options),
        })
        .unwrap();

    let result = project.transform(mapper, request).unwrap();
    assert!(result.text.contains("export const version = 7;"), "got {:?}", result.text);
    assert!(result.mappings.is_some());
    project.close().unwrap();
    host.close().unwrap();
}

// execSpawner spawns the test binary itself as the mapper subprocess (guarded by helperEnv), so the test
// talks to a genuinely separate process over real pipes.
// mapper_test.go:81
struct execSpawner;

impl Spawner for execSpawner {
    // mapper_test.go:83
    fn spawn(&self, _command: &[String], _dir: &str, stderr: Option<Box<dyn Write + Send>>) -> Result<ReadWriteCloser, String> {
        let mut cmd = Command::new(std::env::current_exe().map_err(|err| err.to_string())?);
        cmd.env(HELPER_ENV, "1");
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        // The host passes no writer when it does not log (Go's io.Discard); a logging host's writer is not copied to,
        // the helper's stderr goes to the test's.
        cmd.stderr(if stderr.is_some() { Stdio::inherit() } else { Stdio::null() });
        let mut child = cmd.spawn().map_err(|err| err.to_string())?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().expect("the child's stdout is piped");
        Ok(ReadWriteCloser {
            reader: Box::new(stdout),
            writer: Box::new(processStdin(Arc::clone(&stdin))),
            closer: Arc::new(process { child: Mutex::new(child), stdin }),
        })
    }
}

// process adapts a spawned subprocess's stdio to an io.ReadWriteCloser: reads come from its stdout, writes
// go to its stdin, and Close tears the process down.
// mapper_test.go:102
struct process {
    child: Mutex<Child>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
}

struct processStdin(Arc<Mutex<Option<ChildStdin>>>);

impl Write for processStdin {
    // mapper_test.go:109
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match self.0.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            Some(stdin) => stdin.write(data),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "file already closed")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            Some(stdin) => stdin.flush(),
            None => Ok(()),
        }
    }
}

impl Closer for process {
    // mapper_test.go:111
    fn close(&self) -> io::Result<()> {
        drop(self.stdin.lock().unwrap_or_else(PoisonError::into_inner).take());
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}
