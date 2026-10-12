//! A single benchmark case (port of `benchmark/Benchmark.zig`).
use std::time::{Duration, Instant};

/// The only error a benchmark can report: it failed and the run ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchmarkFailed;

impl std::fmt::Display for BenchmarkFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("benchmark failed")
    }
}
impl std::error::Error for BenchmarkFailed {}

pub type Result<T = ()> = std::result::Result<T, BenchmarkFailed>;

/// How to run: once, or repeatedly for a duration.
#[derive(Debug, Clone, Copy)]
pub enum RunMode {
    Once,
    Duration(Duration),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RunResult {
    pub iterations: u32,
    pub duration: Duration,
}

/// The benchmark interface (Zig `Benchmark.VTable`).
pub trait Benchmark {
    /// A single step; the work under test.
    fn step(&mut self) -> Result;
    /// Called once before the first step; not measured.
    fn setup(&mut self) -> Result {
        Ok(())
    }
    /// Called once after the last step.
    fn teardown(&mut self) {}
}

/// Run the benchmark.
pub fn run(b: &mut dyn Benchmark, mode: RunMode) -> Result<RunResult> {
    b.setup()?;
    let r = run_steps(b, mode);
    b.teardown();
    r
}

fn run_steps(b: &mut dyn Benchmark, mode: RunMode) -> Result<RunResult> {
    let mut result = RunResult::default();

    // On Darwin, emit an Instruments signpost interval around the run so the
    // results are easy to find in Instruments (ends when dropped).
    #[cfg(target_vendor = "apple")]
    let _signpost = crate::signpost::Interval::begin(
        c"com.mitchellh.ghostty",
        c"ghostty",
        (&raw const *b).cast::<()>() as usize,
    );

    let start = Instant::now();
    loop {
        b.step()?;
        result.iterations += 1;
        let elapsed = start.elapsed();
        let exit = match mode {
            RunMode::Once => true,
            RunMode::Duration(d) => elapsed >= d,
        };
        if exit {
            result.duration = elapsed;
            return Ok(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Simple {
        setup_i: usize,
        step_i: usize,
    }
    impl Benchmark for Simple {
        fn step(&mut self) -> Result {
            self.step_i += 1;
            Ok(())
        }
        fn setup(&mut self) -> Result {
            self.setup_i += 1;
            Ok(())
        }
    }

    #[test]
    fn benchmark_once() {
        let mut s = Simple::default();
        let r = run(&mut s, RunMode::Once).unwrap();
        assert_eq!(s.setup_i, 1);
        assert_eq!(s.step_i, 1);
        assert_eq!(r.iterations, 1);
    }
}
