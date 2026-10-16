use criterion::{BenchmarkId, Criterion, criterion_group};
use tsrs_bench::PreparedProject;

#[global_allocator]
static ALLOCATOR: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

fn compile_in_memory(criterion: &mut Criterion) {
    // Criterion2's CodSpeed adapter ignores CLI filters. Register just the requested project, so
    // separate cargo-codspeed invocations cannot accidentally run and retain all four programs.
    let name = std::env::var("TSRS_BENCH_PROJECT")
        .expect("set TSRS_BENCH_PROJECT to the project to benchmark");
    let project = PreparedProject::load(&name);
    let mut group = criterion.benchmark_group("compile_in_memory");
    group.bench_function(BenchmarkId::from_parameter(&name), |b| {
        b.iter_with_setup_wrapper(|runner| {
            let (sys, args) = project.invocation();
            let result = runner.run(|| sys.compile(args));
            project.verify(&result);
        });
    });
    group.finish();
}

criterion_group!(compiler, compile_in_memory);

fn main() {
    // Compiler profiling switches can write files directly, bypassing the FS interface.
    for (name, _) in std::env::vars_os() {
        assert!(
            !name.as_encoded_bytes().starts_with(b"TSRS_") || name == "TSRS_BENCH_PROJECT",
            "unset compiler environment override {} before benchmarking",
            name.display()
        );
    }
    assert_eq!(
        std::env::var("RAYON_NUM_THREADS").as_deref(),
        Ok("1"),
        "run with RAYON_NUM_THREADS=1"
    );
    // Match the CLI's stack; constructing Criterion on this thread also keeps its Rc state local.
    std::thread::Builder::new()
        .name("tsrs-bench".to_string())
        .stack_size(512 << 20)
        .spawn(|| {
            let mut criterion = Criterion::new_instrumented();
            compiler(&mut criterion);
        })
        .unwrap()
        .join()
        .unwrap();
}
