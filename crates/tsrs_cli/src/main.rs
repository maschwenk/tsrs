#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod execute;
mod sys;
mod tsc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Type checking recurses deeply; run on a thread with a large stack
    // (Go's goroutine stacks grow on demand).
    let status = std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || {
            let sys: &'static sys::osSys = Box::leak(Box::new(sys::new_system()));
            let result = execute::command_line(sys, args);
            tsc::System::flush(sys);
            tsrs_core::alloc_profile_dump();
            result.status
        })
        .unwrap()
        .join()
        .unwrap_or(tsc::ExitStatus::NotImplemented);
    std::process::exit(status as i32);
}