use std::time::Duration;

use super::{CompileTimes, EmitInput, System};

struct tableRow {
    name: String,
    value: String,
}

#[derive(Default)]
struct table {
    rows: Vec<tableRow>,
}

impl table {
    fn add(&mut self, name: &str, value: impl ToString) {
        self.rows.push(tableRow { name: name.to_string(), value: value.to_string() });
    }

    fn add_duration(&mut self, name: &str, d: Duration) {
        self.add(name, format_duration(d));
    }

    fn print(&self, sys: &dyn System) {
        let mut name_width = 0;
        let mut value_width = 0;
        for r in &self.rows {
            name_width = name_width.max(r.name.len());
            value_width = value_width.max(r.value.len());
        }

        let mut out = String::new();
        for r in &self.rows {
            // Go: "%-*s %*s\n"
            let name = format!("{}:", r.name);
            out.push_str(&format!("{:<nw$} {:>vw$}\n", name, r.value, nw = name_width + 1, vw = value_width));
        }
        sys.write(&out);
    }
}

fn format_duration(d: Duration) -> String {
    format!("{:.3}s", d.as_secs_f64())
}

pub struct Statistics {
    files: usize,
    lines: usize,
    identifiers: usize,
    symbols: usize,
    types: usize,
    instantiations: usize,
    memory_used: u64,
    memory_allocs: u64,
    compile_times: CompileTimes,
    lazy_member_stats: Option<tsrs_core::lazymembers::LazyMemberStats>,
}

// Go reports runtime.MemStats (live heap and malloc count). There is no GC heap here; the resident
// set size of the process stands in for "Memory used", and allocation counts are not tracked.
fn memory_used_bytes() -> u64 {
    let pid = std::process::id().to_string();
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

pub fn statistics_from_program(input: &EmitInput, times: &CompileTimes) -> Statistics {
    let program = input.program;
    Statistics {
        files: program.source_files().len(),
        lines: program.line_count(),
        identifiers: program.identifier_count(),
        symbols: program.symbol_count(),
        types: program.type_count(),
        instantiations: program.instantiation_count(),
        memory_used: memory_used_bytes(),
        memory_allocs: 0,
        compile_times: *times,
        lazy_member_stats: tsrs_core::lazymembers::enabled().then(|| program.lazy_member_stats()),
    }
}

impl Statistics {
    pub fn report(&self, sys: &dyn System) {
        let mut table = table::default();

        table.add("Files", self.files);
        table.add("Lines", self.lines);
        table.add("Identifiers", self.identifiers);
        table.add("Symbols", self.symbols);
        table.add("Types", self.types);
        table.add("Instantiations", self.instantiations);
        table.add("Memory used", format!("{}K", self.memory_used / 1024));
        table.add("Memory allocs", self.memory_allocs);
        if !self.compile_times.config_time.is_zero() {
            table.add_duration("Config time", self.compile_times.config_time);
        }
        if !self.compile_times.build_info_read_time.is_zero() {
            table.add_duration("BuildInfo read time", self.compile_times.build_info_read_time);
        }
        table.add_duration("Parse time", self.compile_times.parse_time);
        if !self.compile_times.bind_time.is_zero() {
            table.add_duration("Bind time", self.compile_times.bind_time);
        }
        if !self.compile_times.check_time.is_zero() {
            table.add_duration("Check time", self.compile_times.check_time);
        }
        if !self.compile_times.emit_time.is_zero() {
            table.add_duration("Emit time", self.compile_times.emit_time);
        }
        if !self.compile_times.changes_compute_time.is_zero() {
            table.add_duration("Changes compute time", self.compile_times.changes_compute_time);
        }
        table.add_duration("Total time", self.compile_times.total_time);
        table.print(sys);
        if let Some(stats) = &self.lazy_member_stats {
            let mut table = table::default();
            for (name, value) in stats.rows() {
                table.add(name, value);
            }
            table.print(sys);
        }
    }
}
