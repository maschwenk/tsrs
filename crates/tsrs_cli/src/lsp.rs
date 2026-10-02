// Port of cmd/tsc/lsp.go (+ isprocessalive_unix.go / isprocessalive_other.go): `tsrs --lsp -stdio`.

use std::sync::Arc;
use std::time::Duration;

use tsrs_core::context::{CancelFunc, Context};
use tsrs_core::tspath;
use tsrs_vfs::{bundled, osvfs, FS};

// lsp.go:20
pub fn run_lsp(args: &[String]) -> i32 {
    let mut flag = flagSet::new("lsp");
    flag.bool("stdio", "use stdio for communication");
    flag.string("pprofDir", "Generate pprof CPU/memory profiles to the given directory.");
    flag.string("pipe", "use named pipe for communication");
    flag.string("socket", "use socket for communication");
    flag.int("clientProcessId", "use the given PID for the parent process watchdog");
    if flag.parse(args).is_err() {
        return 2;
    }
    let stdio = flag.bool_value("stdio");
    let pprof_dir = flag.string_value("pprofDir");
    let _pipe = flag.string_value("pipe");
    let _socket = flag.string_value("socket");
    let client_process_id = flag.int_value("clientProcessId");

    if !stdio {
        eprintln!("only stdio is supported");
        return 1;
    }

    if !pprof_dir.is_empty() {
        // pprof is out of scope (docs/LSP.md): the message is printed, no profile is written.
        eprintln!("pprof profiles will be written to: {}", pprof_dir);
    }

    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let default_library_path = bundled::lib_path();
    let typings_location = get_global_typings_cache_location();

    // signal.NotifyContext(ctx, os.Interrupt, syscall.SIGTERM): signals keep their default action (the process
    // terminates) because the port has no signal handling; `stop` is still what the watchdog calls.
    let (ctx, stop) = Context::background().with_cancel();

    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd.to_string_lossy().into_owned(),
        Err(err) => panic!("{}", err),
    };

    #[cfg(feature = "alloc-profile")]
    crate::census::prepare_lsp();
    let s = tsrs_lsp::new_server(tsrs_lsp::ServerOptions {
        in_: tsrs_lsp::to_reader(std::io::stdin()),
        out: tsrs_lsp::to_writer(std::io::stdout()),
        err: Box::new(std::io::stderr()),
        cwd,
        fs: Some(fs),
        default_library_path,
        typings_location,
        parse_cache: None,
        npm_install: Some(Arc::new(|cwd: &str, args: &[String]| {
            let output = std::process::Command::new("npm").args(args).current_dir(cwd).stderr(std::process::Stdio::piped()).output();
            match output {
                Ok(output) if output.status.success() => Ok(output.stdout),
                Ok(output) => Err(format!("exit status {}", output.status.code().unwrap_or(-1))),
                Err(err) => Err(err.to_string()),
            }
        })),
        progress_delay: Duration::from_millis(250),
        set_parent_process_id: new_parent_process_watchdog(&ctx, &stop, client_process_id),
    });

    let result = s.run(&ctx);
    #[cfg(feature = "alloc-profile")]
    crate::census::run_lsp(&s);
    if let Err(err) = result {
        eprintln!("{}", err);
        return 1;
    }
    0
}

// lsp.go:79
// newParentProcessWatchdog returns a SetParentProcessID callback if the platform
// supports process-alive checking and no client process ID override was provided,
// or nil otherwise.
fn new_parent_process_watchdog(ctx: &Context, stop: &CancelFunc, client_process_id: i64) -> Option<Box<dyn Fn(i32) + Send + Sync>> {
    if !PROCESS_ALIVE_SUPPORTED {
        return None;
    }
    if client_process_id > 0 {
        start_parent_process_watchdog(ctx, stop, client_process_id);
        return None;
    }
    let ctx = ctx.clone();
    let stop = stop.clone();
    Some(Box::new(move |parent_pid: i32| start_parent_process_watchdog(&ctx, &stop, parent_pid as i64)))
}

// lsp.go:95
// startParentProcessWatchdog starts a goroutine that monitors the parent process
// and cancels the context if the parent dies. This prevents orphaned language
// server processes when the editor crashes or is killed.
fn start_parent_process_watchdog(ctx: &Context, stop: &CancelFunc, parent_pid: i64) {
    if parent_pid <= 0 {
        return;
    }
    let ctx = ctx.clone();
    let stop = stop.clone();
    std::thread::Builder::new()
        .name("lsp-parent-watchdog".to_string())
        .spawn(move || loop {
            // time.NewTicker(5 * time.Second): select on ctx.Done() and the ticker.
            if ctx.wait_timeout(Duration::from_secs(5)) {
                return;
            }
            if !is_process_alive(parent_pid) {
                eprintln!("Parent process {} has exited, shutting down.", parent_pid);
                stop.call();
                return;
            }
        })
        .expect("failed to spawn the parent process watchdog");
}

#[cfg(unix)]
const PROCESS_ALIVE_SUPPORTED: bool = true;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

// isprocessalive_unix.go:18
// isProcessAlive checks if a process with the given PID is still running.
// On Unix, FindProcess always succeeds, so we send signal 0 to probe the
// process. If the signal returns nil or EPERM, the process exists (EPERM
// means it exists but we lack permission to signal it). ESRCH or any
// other error indicates the process is gone.
#[cfg(unix)]
fn is_process_alive(pid: i64) -> bool {
    const EPERM: i32 = 1;
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: kill(2) with signal 0 only checks for the existence of the process and sends nothing.
    let rc = unsafe { kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(EPERM)
}

// isprocessalive_other.go (the Windows variant, isprocessalive_windows.go, is not ported).
#[cfg(not(unix))]
const PROCESS_ALIVE_SUPPORTED: bool = false;

#[cfg(not(unix))]
fn is_process_alive(_pid: i64) -> bool {
    false
}

// vfs/osvfs/os.go:193
fn get_global_typings_cache_location() -> String {
    let cache_dir = user_cache_dir().unwrap_or_else(|| std::env::temp_dir().to_string_lossy().trim_end_matches('/').to_string());

    let subdir = if cfg!(windows) { "Microsoft/TypeScript" } else { "typescript" };
    tspath::combine_paths(&cache_dir, &[subdir, tsrs_core::version_major_minor()])
}

// Go os.UserCacheDir.
fn user_cache_dir() -> Option<String> {
    let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if cfg!(windows) {
        return non_empty("LocalAppData");
    }
    if cfg!(target_os = "macos") {
        return non_empty("HOME").map(|home| home + "/Library/Caches");
    }
    if let Some(dir) = non_empty("XDG_CACHE_HOME") {
        if !dir.starts_with('/') {
            return None;
        }
        return Some(dir);
    }
    non_empty("HOME").map(|home| home + "/.cache")
}

// The subset of Go's `flag` package (FlagSet with ContinueOnError) that runLSP uses.
enum flagValue {
    Bool(bool),
    String(String),
    Int(i64),
}

struct flagDef {
    name: &'static str,
    usage: &'static str,
    value: flagValue,
}

struct flagSet {
    name: &'static str,
    flags: Vec<flagDef>,
}

impl flagSet {
    fn new(name: &'static str) -> flagSet {
        flagSet { name, flags: Vec::new() }
    }

    fn bool(&mut self, name: &'static str, usage: &'static str) {
        self.flags.push(flagDef { name, usage, value: flagValue::Bool(false) });
    }

    fn string(&mut self, name: &'static str, usage: &'static str) {
        self.flags.push(flagDef { name, usage, value: flagValue::String(String::new()) });
    }

    fn int(&mut self, name: &'static str, usage: &'static str) {
        self.flags.push(flagDef { name, usage, value: flagValue::Int(0) });
    }

    fn lookup(&self, name: &str) -> Option<&flagDef> {
        self.flags.iter().find(|f| f.name == name)
    }

    fn bool_value(&self, name: &str) -> bool {
        matches!(self.lookup(name).map(|f| &f.value), Some(flagValue::Bool(true)))
    }

    fn string_value(&self, name: &str) -> String {
        match self.lookup(name).map(|f| &f.value) {
            Some(flagValue::String(s)) => s.clone(),
            _ => String::new(),
        }
    }

    fn int_value(&self, name: &str) -> i64 {
        match self.lookup(name).map(|f| &f.value) {
            Some(flagValue::Int(i)) => *i,
            _ => 0,
        }
    }

    // flag.(*FlagSet).PrintDefaults via the default Usage.
    fn usage(&self) {
        let mut out = format!("Usage of {}:\n", self.name);
        let mut flags: Vec<&flagDef> = self.flags.iter().collect();
        flags.sort_by_key(|f| f.name);
        for f in flags {
            let mut line = format!("  -{}", f.name);
            match f.value {
                flagValue::Bool(_) => {}
                flagValue::String(_) => line.push_str(" string"),
                flagValue::Int(_) => line.push_str(" int"),
            }
            // Boolean flags of one ASCII character are so common we treat them specially, putting their usage on
            // the same line.
            if line.len() <= 4 {
                line.push('\t');
            } else {
                line.push_str("\n    \t");
            }
            line.push_str(&f.usage.replace('\n', "\n    \t"));
            out.push_str(&line);
            out.push('\n');
        }
        eprint!("{}", out);
    }

    fn fail(&self, message: String) -> Result<(), ()> {
        eprintln!("{}", message);
        self.usage();
        Err(())
    }

    // flag.(*FlagSet).Parse
    fn parse(&mut self, args: &[String]) -> Result<(), ()> {
        let mut i = 0;
        while i < args.len() {
            let s = &args[i];
            if s.len() < 2 || !s.starts_with('-') {
                return Ok(());
            }
            let mut num_minuses = 1;
            if s.as_bytes()[1] == b'-' {
                num_minuses += 1;
                if s.len() == 2 {
                    // "--" terminates the flags
                    return Ok(());
                }
            }
            let name = &s[num_minuses..];
            if name.is_empty() || name.starts_with('-') || name.starts_with('=') {
                return self.fail(format!("bad flag syntax: {}", s));
            }
            i += 1;

            let (name, value) = match name.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (name, None),
            };

            let Some(index) = self.flags.iter().position(|f| f.name == name) else {
                if name == "help" || name == "h" {
                    // special case for nice help message.
                    self.usage();
                    return Err(());
                }
                return self.fail(format!("flag provided but not defined: -{}", name));
            };

            match self.flags[index].value {
                flagValue::Bool(_) => {
                    let b = match value.as_deref() {
                        None => true,
                        Some("1" | "t" | "T" | "TRUE" | "true" | "True") => true,
                        Some("0" | "f" | "F" | "FALSE" | "false" | "False") => false,
                        Some(v) => return self.fail(format!("invalid boolean value {:?} for -{}: parse error", v, name)),
                    };
                    self.flags[index].value = flagValue::Bool(b);
                }
                _ => {
                    let value = match value {
                        Some(value) => value,
                        None => {
                            if i >= args.len() {
                                return self.fail(format!("flag needs an argument: -{}", name));
                            }
                            i += 1;
                            args[i - 1].clone()
                        }
                    };
                    let new_value = match self.flags[index].value {
                        flagValue::Int(_) => match parse_go_int(&value) {
                            Ok(n) => flagValue::Int(n),
                            Err(reason) => return self.fail(format!("invalid value {:?} for flag -{}: {}", value, name, reason)),
                        },
                        _ => flagValue::String(value),
                    };
                    self.flags[index].value = new_value;
                }
            }
        }
        Ok(())
    }
}

// strconv.ParseInt(s, 0, 64) as flag's intValue.Set uses it; the error is flag's numError text.
fn parse_go_int(s: &str) -> Result<i64, &'static str> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (radix, digits) = if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        (16, rest)
    } else if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        (2, rest)
    } else if let Some(rest) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        (8, rest)
    } else if body.len() > 1 && body.starts_with('0') {
        (8, &body[1..])
    } else {
        (10, body)
    };
    let digits = digits.replace('_', "");
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err("parse error");
    }
    let magnitude = i128::from_str_radix(&digits, radix).map_err(|_| "value out of range")?;
    let value = if neg { -magnitude } else { magnitude };
    i64::try_from(value).map_err(|_| "value out of range")
}
