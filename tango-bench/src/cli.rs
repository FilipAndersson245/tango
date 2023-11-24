use crate::{dylib::Spi, Benchmark, MeasurementSettings, Reporter};
use clap::Parser;
use core::fmt;
use libloading::Library;
use std::{
    collections::HashSet,
    fmt::Display,
    hash::Hash,
    num::{NonZeroU64, NonZeroUsize},
    path::PathBuf,
    time::Duration,
};

use self::reporting::{ConsoleReporter, VerboseReporter};

#[derive(Parser, Debug)]
enum BenchmarkMode {
    Pair {
        #[command(flatten)]
        bench_flags: CargoBenchFlags,

        name: Option<String>,

        #[arg(short = 's', long = "samples")]
        samples: Option<NonZeroUsize>,

        #[arg(short = 't', long = "time")]
        time: Option<NonZeroU64>,

        /// write CSV dumps of all the measurements in a given location
        #[arg(short = 'd', long = "dump")]
        path_to_dump: Option<PathBuf>,

        /// disable outlier detection
        #[arg(short = 'o', long = "no-outliers")]
        skip_outlier_detection: bool,

        #[arg(short = 'v', long = "verbose", default_value_t = false)]
        verbose: bool,
    },
    Calibrate {
        #[command(flatten)]
        bench_flags: CargoBenchFlags,
    },
    List {
        #[command(flatten)]
        bench_flags: CargoBenchFlags,
    },
    Compare {
        #[command(flatten)]
        bench_flags: CargoBenchFlags,

        /// Path to the executable to test agains. Tango will test agains itself if no executable given
        path: Option<PathBuf>,

        #[arg(short = 'f', long = "filter")]
        filter: Option<String>,

        #[arg(short = 'v', long = "verbose", default_value_t = false)]
        verbose: bool,
    },
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Opts {
    #[command(subcommand)]
    subcommand: BenchmarkMode,

    #[command(flatten)]
    bench_flags: CargoBenchFlags,
}

/// Definition of the flags required to comply with `cargo bench` calling conventions.
#[derive(Parser, Debug, Clone)]
struct CargoBenchFlags {
    #[arg(long = "bench", default_value_t = true)]
    bench: bool,
}

pub fn run<H, N>(mut benchmark: Benchmark<H, N>, settings: MeasurementSettings) {
    let opts = Opts::parse();

    match opts.subcommand {
        BenchmarkMode::Pair {
            name,
            time,
            samples,
            verbose,
            path_to_dump,
            skip_outlier_detection,
            bench_flags: _,
        } => {
            let mut reporter: Box<dyn Reporter> = if verbose {
                Box::<VerboseReporter>::default()
            } else {
                Box::<ConsoleReporter>::default()
            };

            let mut opts = settings;
            if let Some(samples) = samples {
                opts.max_samples = samples.into()
            }
            if let Some(millis) = time {
                opts.max_duration = Duration::from_millis(millis.into());
            }
            if skip_outlier_detection {
                opts.outlier_detection_enabled = false;
            }

            let name_filter = name.as_deref().unwrap_or("");
            benchmark.run_by_name(reporter.as_mut(), name_filter, &opts, path_to_dump.as_ref());
        }
        BenchmarkMode::Calibrate { bench_flags: _ } => {
            benchmark.run_calibration();
        }
        BenchmarkMode::List { bench_flags: _ } => {
            for fn_name in benchmark.list_functions() {
                println!("{}", fn_name);
            }
        }
        BenchmarkMode::Compare {
            path,
            verbose,
            filter,
            bench_flags: _,
        } => {
            let mut reporter: Box<dyn Reporter> = if verbose {
                Box::<VerboseReporter>::default()
            } else {
                Box::<ConsoleReporter>::default()
            };

            let self_path = PathBuf::from(std::env::args().next().unwrap());
            let path = path.unwrap_or(self_path);

            let lib = unsafe { Library::new(path) }.expect("Unable to load library");
            let spi_lib = Spi::for_library(&lib);
            let spi_self = Spi::for_self();

            let mut test_names = intersect_values(spi_lib.tests().keys(), spi_self.tests().keys());
            test_names.sort();

            let filter = filter.as_deref().unwrap_or("");
            for name in test_names {
                if name.contains(filter) {
                    commands::pairwise_compare(
                        &spi_self,
                        &spi_lib,
                        name.as_str(),
                        reporter.as_mut(),
                    );
                }
            }
        }
    }
}

mod commands {
    use crate::{calculate_run_result, Summary};
    use std::time::Instant;

    use super::*;

    pub(super) fn pairwise_compare(
        base: &Spi,
        candidate: &Spi,
        test_name: &str,
        reporter: &mut dyn Reporter,
    ) {
        let base_idx = *base.tests().get(test_name).unwrap();
        let candidate_idx = *candidate.tests().get(test_name).unwrap();

        // Number of iterations estimated based on the performance of base algorithm only. We assuming
        // both algorithms performs approximatley the same. We need to divide estimation by 2 to compensate
        // for the fact that 2 algorithms will be executed concurrently.
        let estimate = base.estimate_iterations(base_idx, 1) / 2;
        let iterations = estimate.max(1).min(50);

        let mut base_samples = vec![];
        let mut candidate_samples = vec![];

        let deadline = Instant::now() + Duration::from_millis(100);

        let mut baseline_first = false;
        while Instant::now() < deadline {
            if baseline_first {
                base_samples.push(base.run(base_idx, iterations) as i64 / iterations as i64);
                candidate_samples
                    .push(candidate.run(candidate_idx, iterations) as i64 / iterations as i64);
            } else {
                candidate_samples
                    .push(candidate.run(candidate_idx, iterations) as i64 / iterations as i64);
                base_samples.push(base.run(base_idx, iterations) as i64 / iterations as i64);
            }

            baseline_first = !baseline_first;
        }

        let diff: Vec<_> = base_samples
            .iter()
            .zip(candidate_samples.iter())
            .map(|(b, c)| c - b)
            .collect();

        let base_summary = Summary::from(&base_samples).unwrap();
        let candidate_summary = Summary::from(&candidate_samples).unwrap();

        let result = calculate_run_result(
            (format!("{} B", test_name), base_summary),
            (format!("{} C", test_name), candidate_summary),
            diff,
            false,
        );

        reporter.on_complete(&result);
    }
}

/// Returning the intersection of the values of two iterators buffering them in an intermediate [`HashSet`].
fn intersect_values<'a, K: Hash + Eq>(
    a: impl Iterator<Item = &'a K>,
    b: impl Iterator<Item = &'a K>,
) -> Vec<&'a K> {
    let a_values = a.collect::<HashSet<_>>();
    let b_values = b.collect::<HashSet<_>>();
    a_values
        .intersection(&b_values)
        .map(|s| *s)
        .collect::<Vec<_>>()
}

pub mod reporting {

    use crate::cli::{colorize, Color, Colored, HumanTime};
    use crate::{Reporter, RunResult};

    #[derive(Default)]
    pub(super) struct VerboseReporter;

    impl Reporter for VerboseReporter {
        fn on_complete(&mut self, results: &RunResult) {
            let base = results.baseline;
            let candidate = results.candidate;

            let significant = results.significant;

            println!(
                "{} vs. {}  (n: {}, outliers: {})",
                Colored(&results.base_name, Color::Bold),
                Colored(&results.candidate_name, Color::Bold),
                results.diff.n,
                results.outliers
            );
            println!();

            println!(
                "    {:12}   {:>15} {:>15} {:>15}",
                "",
                Colored(&results.base_name, Color::Bold),
                Colored(&results.candidate_name, Color::Bold),
                Colored("∆", Color::Bold),
            );
            println!(
                "    {:12} ╭────────────────────────────────────────────────",
                ""
            );
            println!(
                "    {:12} │ {:>15} {:>15} {:>15}",
                "min",
                HumanTime(base.min as f64),
                HumanTime(candidate.min as f64),
                HumanTime((candidate.min - base.min) as f64)
            );
            println!(
                "    {:12} │ {:>15} {:>15} {:>15}  {:+4.2}{}",
                "mean",
                HumanTime(base.mean),
                HumanTime(candidate.mean),
                colorize(
                    HumanTime(results.diff.mean),
                    significant,
                    results.diff.mean < 0.
                ),
                colorize(
                    results.diff.mean / base.mean * 100.,
                    significant,
                    results.diff.mean < 0.
                ),
                colorize("%", significant, results.diff.mean < 0.)
            );
            println!(
                "    {:12} │ {:>15} {:>15} {:>15}",
                "max",
                HumanTime(base.max as f64),
                HumanTime(candidate.max as f64),
                HumanTime((candidate.max - base.max) as f64),
            );
            println!(
                "    {:12} │ {:>15} {:>15} {:>15}",
                "std. dev.",
                HumanTime(base.variance.sqrt()),
                HumanTime(candidate.variance.sqrt()),
                HumanTime(results.diff.variance.sqrt()),
            );
            println!();
        }
    }

    #[derive(Default)]
    pub(super) struct ConsoleReporter {
        current_generator_name: Option<String>,
    }

    impl Reporter for ConsoleReporter {
        fn on_start(&mut self, generator_name: &str) {
            self.current_generator_name = Some(generator_name.into());
        }

        fn on_complete(&mut self, results: &RunResult) {
            let base = results.baseline;
            let candidate = results.candidate;
            let diff = results.diff;

            let significant = results.significant;

            let speedup = diff.mean / base.mean * 100.;
            let candidate_faster = diff.mean < 0.;
            println!(
                "{:20}  {:>30} / {:30} [ {:>8} ... {:>8} ]    {:>+7.2}{}",
                self.current_generator_name.take().as_deref().unwrap_or(""),
                results.base_name,
                colorize(&results.candidate_name, significant, candidate_faster),
                HumanTime(base.mean),
                colorize(HumanTime(candidate.mean), significant, candidate_faster),
                colorize(speedup, significant, candidate_faster),
                colorize("%", significant, candidate_faster)
            )
        }
    }
}

fn colorize<T: Display>(value: T, do_paint: bool, indicator: bool) -> Colored<T> {
    if do_paint {
        let color = if indicator { Color::Green } else { Color::Red };
        Colored(value, color)
    } else {
        Colored(value, Color::Reset)
    }
}

enum Color {
    Red,
    Green,
    Bold,
    Reset,
}

impl Color {
    fn ascii_color_code(&self) -> &'static str {
        match self {
            Color::Red => "\x1B[31m",
            Color::Green => "\x1B[32m",
            Color::Bold => "\x1B[1m",
            Color::Reset => "\x1B[0m",
        }
    }
}

struct Colored<T>(T, Color);

impl<T: Display> Display for Colored<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.1.ascii_color_code())
            .and_then(|_| self.0.fmt(f))
            .and_then(|_| write!(f, "{}", Color::Reset.ascii_color_code()))
    }
}

struct HumanTime(f64);

impl fmt::Display for HumanTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const USEC: f64 = 1_000.;
        const MSEC: f64 = USEC * 1_000.;
        const SEC: f64 = MSEC * 1_000.;

        if self.0.abs() > SEC {
            f.pad(&format!("{:.1} s", self.0 / SEC))
        } else if self.0.abs() > MSEC {
            f.pad(&format!("{:.1} ms", self.0 / MSEC))
        } else if self.0.abs() > USEC {
            f.pad(&format!("{:.1} us", self.0 / USEC))
        } else {
            f.pad(&format!("{:.0} ns", self.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_human_time() {
        assert_eq!(format!("{}", HumanTime(0.1)), "0 ns");
        assert_eq!(format!("{:>5}", HumanTime(0.)), " 0 ns");

        assert_eq!(format!("{}", HumanTime(120.)), "120 ns");

        assert_eq!(format!("{}", HumanTime(1200.)), "1.2 us");

        assert_eq!(format!("{}", HumanTime(1200000.)), "1.2 ms");

        assert_eq!(format!("{}", HumanTime(1200000000.)), "1.2 s");

        assert_eq!(format!("{}", HumanTime(-1200000.)), "-1.2 ms");
    }
}
