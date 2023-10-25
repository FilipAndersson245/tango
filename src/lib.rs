use num_traits::ToPrimitive;
use statrs::distribution::Normal;
use std::{
    any::type_name,
    cmp::Ordering,
    collections::BTreeMap,
    fs::File,
    hint::black_box,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use timer::{ActiveTimer, Timer};

pub mod cli;

pub fn benchmark_fn<H, N, O, F: Fn(&H, &N) -> O>(
    name: impl Into<String>,
    func: F,
) -> impl BenchmarkFn<H, N, O> {
    let name = name.into();
    assert!(!name.is_empty());
    Func { name, func }
}

pub fn benchmark_fn_with_setup<H, N, O, I: Clone, F: Fn(I, &N) -> O, S: Fn(&H) -> I>(
    name: impl Into<String>,
    func: F,
    setup: S,
) -> impl BenchmarkFn<H, N, O> {
    let name = name.into();
    assert!(!name.is_empty());
    SetupFunc { name, func, setup }
}

pub trait BenchmarkFn<H, N, O> {
    fn measure(&self, haystack: &H, needle: &N, iterations: usize) -> u64;
    fn name(&self) -> &str;
}

struct Func<F> {
    name: String,
    func: F,
}

impl<F, H, N, O> BenchmarkFn<H, N, O> for Func<F>
where
    F: Fn(&H, &N) -> O,
{
    fn measure(&self, haystack: &H, needle: &N, iterations: usize) -> u64 {
        let mut result = Vec::with_capacity(iterations);
        let start = ActiveTimer::start();
        for _ in 0..iterations {
            result.push(black_box((self.func)(haystack, needle)));
        }
        let time = ActiveTimer::stop(start);
        drop(result);
        time
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }
}

struct SetupFunc<S, F> {
    name: String,
    setup: S,
    func: F,
}

impl<S, F, H, N, I, O> BenchmarkFn<H, N, O> for SetupFunc<S, F>
where
    S: Fn(&H) -> I,
    F: Fn(I, &N) -> O,
    I: Clone,
{
    fn measure(&self, haystack: &H, needle: &N, iterations: usize) -> u64 {
        let mut results = Vec::with_capacity(iterations);
        let haystack = (self.setup)(haystack);
        let start = ActiveTimer::start();
        for _ in 0..iterations {
            results.push(black_box((self.func)(haystack.clone(), needle)));
        }
        let time = ActiveTimer::stop(start);
        drop(results);
        time
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }
}

pub trait Generator {
    type Haystack;
    type Needle;

    fn next_haystack(&mut self) -> Self::Haystack;
    fn next_needle(&mut self) -> Self::Needle;

    fn name(&self) -> String {
        type_name::<Self>().to_string()
    }
}

pub struct StaticValue<H, N>(pub H, pub N);

impl<H: Copy, N: Copy> Generator for StaticValue<H, N> {
    type Haystack = H;
    type Needle = N;

    fn next_haystack(&mut self) -> Self::Haystack {
        self.0
    }

    fn next_needle(&mut self) -> Self::Needle {
        self.1
    }
}

pub trait Reporter {
    fn on_start(&mut self, _payloads_name: &str) {}
    fn on_complete(&mut self, _results: &RunResult) {}
}

type FnPair<H, N, O> = (Box<dyn BenchmarkFn<H, N, O>>, Box<dyn BenchmarkFn<H, N, O>>);

pub struct Benchmark<H, N, O> {
    funcs: BTreeMap<String, FnPair<H, N, O>>,
    reporters: Vec<Box<dyn Reporter>>,
}

impl<H, N, O> Default for Benchmark<H, N, O> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RunOpts {
    name_filter: Option<String>,

    /// Directory location for CSV dump of individual mesurements
    ///
    /// The format is as follows
    /// ```txt
    /// b_1,c_1
    /// b_2,c_2
    /// ...
    /// b_n,c_n
    /// ```
    /// where `b_1..b_n` are baseline absolute time (in nanoseconds) measurements
    /// and `c_1..c_n` are candidate time measurements
    measurements_path: Option<PathBuf>,
    max_iterations: usize,
    max_duration: Duration,
    outlier_detection_enabled: bool,
    haystack_frequency: usize,
    needle_frequency: usize,
}

impl<H, N, O> Benchmark<H, N, O> {
    pub fn new() -> Self {
        Self {
            funcs: BTreeMap::new(),
            reporters: vec![],
        }
    }

    pub fn add_reporter(&mut self, reporter: impl Reporter + 'static) {
        self.reporters.push(Box::new(reporter))
    }

    pub fn add_pair(
        &mut self,
        baseline: impl BenchmarkFn<H, N, O> + 'static,
        candidate: impl BenchmarkFn<H, N, O> + 'static,
    ) {
        let key = format!("{}-{}", baseline.name(), candidate.name());
        self.funcs
            .insert(key, (Box::new(baseline), Box::new(candidate)));
    }

    pub fn run_by_name(
        &mut self,
        payloads: &mut dyn Generator<Haystack = H, Needle = N>,
        opts: &RunOpts,
    ) {
        let name_filter = opts.name_filter.as_deref().unwrap_or("");
        let generator_name = payloads.name();
        let mut start_reported = false;
        for (key, (baseline, candidate)) in &self.funcs {
            if key.contains(name_filter) || generator_name.contains(name_filter) {
                if !start_reported {
                    for reporter in self.reporters.iter_mut() {
                        reporter.on_start(generator_name.as_str());
                    }
                    start_reported = true;
                }
                let (baseline_summary, candidate_summary, diff) =
                    measure_function_pair(payloads, baseline.as_ref(), candidate.as_ref(), &opts);

                let run_result = calculate_run_result(
                    baseline.name(),
                    candidate.name(),
                    baseline_summary,
                    candidate_summary,
                    diff,
                    opts.outlier_detection_enabled,
                );

                for reporter in self.reporters.iter_mut() {
                    reporter.on_complete(&run_result);
                }
            }
        }
    }

    pub fn run_calibration(&mut self, payloads: &mut dyn Generator<Haystack = H, Needle = N>) {
        const TRIES: usize = 10;

        // H0 testing
        println!("H0 testing...");
        for (baseline, candidate) in self.funcs.values() {
            let significant =
                Self::calibrate(payloads, baseline.as_ref(), baseline.as_ref(), TRIES);
            println!("  {:20} {}/{}", baseline.name(), TRIES - significant, TRIES);

            let significant =
                Self::calibrate(payloads, candidate.as_ref(), candidate.as_ref(), TRIES);
            println!(
                "  {:20} {}/{}",
                candidate.name(),
                TRIES - significant,
                TRIES
            );
        }

        println!("H1 testing...");
        for (baseline, candidate) in self.funcs.values() {
            let significant =
                Self::calibrate(payloads, baseline.as_ref(), candidate.as_ref(), TRIES);
            println!(
                "  {} / {:20} {}/{}",
                baseline.name(),
                candidate.name(),
                significant,
                TRIES
            );
        }
    }

    /// Runs a given test multiple times and return the the number of times difference is statistically significant
    fn calibrate(
        payloads: &mut (dyn Generator<Haystack = H, Needle = N>),
        a: &dyn BenchmarkFn<H, N, O>,
        b: &dyn BenchmarkFn<H, N, O>,
        tries: usize,
    ) -> usize {
        let mut succeed = 0;
        let opts = RunOpts {
            name_filter: None,
            measurements_path: None,
            max_iterations: 1_000_000,
            max_duration: Duration::from_millis(1000),
            outlier_detection_enabled: true,
            haystack_frequency: 1,
            needle_frequency: 1,
        };
        for _ in 0..tries {
            let (a_summary, b_summary, diff) = measure_function_pair(payloads, a, b, &opts);

            let result = calculate_run_result(a.name(), b.name(), a_summary, b_summary, diff, true);
            succeed += usize::from(result.significant);
        }
        succeed
    }

    pub fn list_functions(&self) -> impl Iterator<Item = &str> {
        self.funcs.keys().map(String::as_str)
    }
}

fn measure_function_pair<H, N, O>(
    generator: &mut dyn Generator<Haystack = H, Needle = N>,
    base: &dyn BenchmarkFn<H, N, O>,
    candidate: &dyn BenchmarkFn<H, N, O>,
    opts: &RunOpts,
) -> (Summary<i64>, Summary<i64>, Vec<i64>) {
    let mut base_time = Vec::with_capacity(opts.max_iterations);
    let mut candidate_time = Vec::with_capacity(opts.max_iterations);

    let iterations = estimate_iterations_per_ms(generator, base, candidate);

    let deadline = Instant::now() + opts.max_duration;

    let mut haystack = generator.next_haystack();
    let mut needle = generator.next_needle();

    let iterations_choices = (1..=iterations.min(100)).into_iter().collect::<Vec<_>>();

    for i in 0..opts.max_iterations {
        if i % 10 == 0 && Instant::now() >= deadline {
            break;
        }
        if i % opts.haystack_frequency == 0 {
            haystack = generator.next_haystack();
        }
        if i % opts.needle_frequency == 0 {
            needle = generator.next_needle();
        }

        let iterations = iterations_choices[(i / 2) % iterations_choices.len()];

        let (base_sample, candidate_sample) = if i % 2 == 0 {
            let base_sample = base.measure(&haystack, &needle, iterations);
            let candidate_sample = candidate.measure(&haystack, &needle, iterations);

            (base_sample, candidate_sample)
        } else {
            let candidate_sample = candidate.measure(&haystack, &needle, iterations);
            let base_sample = base.measure(&haystack, &needle, iterations);

            (base_sample, candidate_sample)
        };

        base_time.push(base_sample as i64 / iterations as i64);
        candidate_time.push(candidate_sample as i64 / iterations as i64);
    }

    // let base_time = base_time[base_time.len() / 50..].to_vec();
    // let candidate_time = candidate_time[candidate_time.len() / 50..].to_vec();

    if let Some(path) = opts.measurements_path.as_ref() {
        let file_name = format!("{}-{}.csv", base.name(), candidate.name());
        let file_path = path.join(file_name);
        write_raw_measurements(file_path, &base_time, &candidate_time);
    }

    let base = Summary::from(&base_time).unwrap();
    let candidate = Summary::from(&candidate_time).unwrap();
    let diff = base_time
        .into_iter()
        .zip(candidate_time)
        .map(|(b, c)| c - b)
        .collect();
    (base, candidate, diff)
}

/// Estimates the number of iterations achievable in 1 ms by given pair of functions.
///
/// If functions are to slow to be executed in 1ms, the number of iterations will be 1.
fn estimate_iterations_per_ms<H, N, O>(
    generator: &mut dyn Generator<Haystack = H, Needle = N>,
    base: &dyn BenchmarkFn<H, N, O>,
    candidate: &dyn BenchmarkFn<H, N, O>,
) -> usize {
    let haystack = generator.next_haystack();
    let needle = generator.next_needle();

    // Measure the amount of iterations achievable in (factor * 1ms) and later divide by factor
    // to calculate average number of iterations per 1ms
    let factor = 10;
    let duration = Duration::from_millis(1);
    let deadline = Instant::now() + duration * factor;
    let mut iterations = 0;
    while Instant::now() < deadline {
        candidate.measure(&haystack, &needle, 1);
        base.measure(&haystack, &needle, 1);
        iterations += 1;
    }

    let rounding_factor = 10;
    1.max(iterations / factor as usize / rounding_factor * rounding_factor)
}

fn calculate_run_result(
    baseline_name: impl Into<String>,
    candidate_name: impl Into<String>,
    baseline_summary: Summary<i64>,
    candidate_summary: Summary<i64>,
    diff: Vec<i64>,
    filter_outliers: bool,
) -> RunResult {
    let n = diff.len();

    let diff_summary = if filter_outliers {
        let input = diff.to_vec();
        let (min, max) = iqr_variance_thresholds(input).unwrap_or((i64::MIN, i64::MAX));

        let measurements = diff
            .iter()
            .copied()
            .filter(|i| min < *i && *i < max)
            .collect::<Vec<_>>();
        Summary::from(&measurements).unwrap()
    } else {
        Summary::from(&diff).unwrap()
    };

    let outliers_filtered = n - diff_summary.n;

    let std_dev = diff_summary.variance.sqrt();
    let std_err = std_dev / (diff_summary.n as f64).sqrt();
    let z_score = diff_summary.mean / std_err;

    RunResult {
        base_name: baseline_name.into(),
        candidate_name: candidate_name.into(),
        baseline: baseline_summary,
        candidate: candidate_summary,
        diff: diff_summary,
        // significant result is far away from 0 and have more than 0.5%
        // base/candidate difference
        // z_score = 2.6 corresponds to 99% significance level
        significant: z_score.abs() >= 2.6
            && (diff_summary.mean / candidate_summary.mean).abs() > 0.005,
        outliers: outliers_filtered,
    }
}

/// Describes the results of a single benchmark run
pub struct RunResult {
    /// name of a baseline function
    pub base_name: String,

    /// name of a candidate function
    pub candidate_name: String,

    /// statistical summary of baseline function measurements
    pub baseline: Summary<i64>,

    /// statistical summary of candidate function measurements
    pub candidate: Summary<i64>,

    /// individual measurements of a benchmark (candidate - baseline)
    pub diff: Summary<i64>,

    /// Is difference is statistically significant
    pub significant: bool,

    /// Numbers of detected and filtered outliers
    pub outliers: usize,
}

fn write_raw_measurements(path: impl AsRef<Path>, base: &[i64], candidate: &[i64]) {
    let mut file = BufWriter::new(File::create(path).unwrap());

    for (b, c) in base.iter().zip(candidate) {
        writeln!(&mut file, "{},{}", b, c).unwrap();
    }
}

/// Statistical summary for a given iterator of numbers.
///
/// Calculates all the information using single pass over the data. Mean and variance are calculated using
/// streaming algorithm described in [1].
///
/// [1]: Art of Computer Programming, Vol 2, page 232
#[derive(Clone, Copy)]
pub struct Summary<T> {
    pub n: usize,
    pub min: T,
    pub max: T,
    pub mean: f64,
    pub variance: f64,
}

impl<'a, T: PartialOrd + Copy + 'a> Summary<T> {
    pub fn from<C>(values: C) -> Option<Self>
    where
        T: ToPrimitive,
        C: IntoIterator<Item = &'a T>,
    {
        Self::running(values.into_iter().copied())?.last()
    }

    pub fn running<I>(mut iter: I) -> Option<impl Iterator<Item = Summary<T>>>
    where
        T: ToPrimitive,
        I: Iterator<Item = T>,
    {
        let head = iter.next()?;
        Some(RunningSummary {
            iter,
            n: 1,
            min: head,
            max: head,
            mean: head.to_f64().unwrap(),
            s: 0.,
        })
    }
}

struct RunningSummary<T, I> {
    iter: I,
    n: usize,
    min: T,
    max: T,
    mean: f64,
    s: f64,
}

impl<T, I> Iterator for RunningSummary<T, I>
where
    T: Copy + PartialOrd,
    I: Iterator<Item = T>,
    T: ToPrimitive,
{
    type Item = Summary<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.iter.next()?;
        let fvalue = value.to_f64().unwrap();

        if let Some(Ordering::Less) = value.partial_cmp(&self.min) {
            self.min = value;
        }
        if let Some(Ordering::Greater) = value.partial_cmp(&self.max) {
            self.max = value;
        }

        self.n += 1;
        let mean_p = self.mean;
        self.mean += (fvalue - self.mean) / self.n as f64;
        self.s += (fvalue - mean_p) * (fvalue - self.mean);

        Some(Summary {
            n: self.n,
            min: self.min,
            max: self.max,
            mean: self.mean,
            variance: self.s / (self.n - 1) as f64,
        })
    }
}

/// Running variance iterator
///
/// Provides a running (streaming variance) for a given iterator of observations.
/// Uses simple variance formula: `Var(X) = E[X^2] - E[X]^2`.
pub struct RunningVariance<T> {
    iter: T,
    m: f64,
    s: f64,
    n: f64,
}

impl<T: Iterator<Item = i64>> Iterator for RunningVariance<T> {
    type Item = f64;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.iter.next()? as f64;
        self.n += 1.;
        let m_p = self.m;
        self.m += (value - self.m) / self.n;
        self.s += (value - m_p) * (value - self.m);

        if self.n == 1. {
            Some(0.)
        } else {
            Some(self.s / (self.n - 1.))
        }
    }
}

impl<I, T: Iterator<Item = I>> From<T> for RunningVariance<T> {
    fn from(value: T) -> Self {
        Self {
            iter: value,
            m: 0.,
            s: 0.,
            n: 0.,
        }
    }
}

/// Outlier detection algorithm based on interquartile range
///
/// Outliers are observations are 5 IQR away from the corresponding quartile.
fn iqr_variance_thresholds(mut input: Vec<i64>) -> Option<(i64, i64)> {
    input.sort();
    let (q1_idx, q3_idx) = (input.len() / 4, input.len() * 3 / 4);
    if q1_idx < q3_idx && input[q1_idx] < input[q3_idx] && q3_idx < input.len() {
        let iqr = input[q3_idx] - input[q1_idx];
        let low_threshold = input[q1_idx] - iqr * 5;
        let high_threshold = input[q3_idx] + iqr * 5;

        // Calculating the indicies of the thresholds in an dataset
        let low_threshold_idx = match input[0..q1_idx].binary_search(&low_threshold) {
            Ok(idx) => idx,
            Err(idx) => idx,
        };

        let high_threshold_idx = match input[q3_idx..].binary_search(&high_threshold) {
            Ok(idx) => idx,
            Err(idx) => idx,
        };

        if low_threshold_idx == 0 || high_threshold_idx >= input.len() {
            return None;
        }

        // Calculating the equal number of observations which should be removed from each "side" of observations
        let outliers_cnt = low_threshold_idx.min(input.len() - high_threshold_idx);

        Some((input[outliers_cnt], input[input.len() - outliers_cnt]))
        // Some((low_threshold, high_threshold))
    } else {
        None
    }
}

/// Outlier threshold detection
///
/// This functions detects optimal threshold for outlier filtering. Algorithm finds a threshold
/// that split the set of all observations `M` into two different subsets `S` and `O`. Each observation
/// is considered as a split point. Algorithm chooses split point in such way that it maximizes
/// the ration of `S` with this observation and without.
///
/// For example in a set of observations `[1, 2, 3, 100, 200, 300]` the target observation will be 100.
/// It is the observation including which will raise variance the most.
/// `outliers_cnt` - maximum number of outliers to ignore.
fn max_variance_thresholds(mut input: Vec<i64>, mut outliers_cnt: usize) -> Option<(i64, i64)> {
    // TODO(bazhenov) sorting should be done by difference with median
    input.sort_by_key(|a| a.abs());

    let variance = RunningVariance::from(input.iter().copied());

    // Looking only topmost values
    let skip = input.len() - outliers_cnt;
    let mut candidate_outliers = input[skip..].iter().filter(|i| **i < 0).count();
    let value_and_variance = input.iter().copied().zip(variance).skip(skip);

    let mut prev_variance = 0.;
    for (value, var) in value_and_variance {
        if prev_variance > 0. {
            let deviance = (var / prev_variance) - 1.;
            let target = 100. / ((input.len() - outliers_cnt) as f64);
            if deviance > target {
                if let Some((min, max)) = binomial_interval_approximation(outliers_cnt, 0.5, 0.5) {
                    if min > candidate_outliers && candidate_outliers < max {
                        return Some((-value.abs(), value.abs()));
                    }
                } else {
                    // Normal approximation of binomial doesn't work for small amount of observations
                    // n * p < 10. But it means that we do not have justification of imbalanced outliers
                    return Some((-value.abs(), value.abs()));
                }
            }
        }

        prev_variance = var;
        outliers_cnt -= 1;
        if value < 0 {
            candidate_outliers -= 1;
        }
    }

    None
}

fn binomial_interval_approximation(n: usize, p: f64, width: f64) -> Option<(usize, usize)> {
    use statrs::distribution::ContinuousCDF;
    let nf = n as f64;
    if nf * p < 10. || nf * (1. - p) < 10. {
        return None;
    }
    let mu = nf * p;
    let sigma = (nf * p * (1. - p)).sqrt();
    let distribution = Normal::new(mu, sigma).unwrap();
    let min = distribution.inverse_cdf(width / 2.).floor() as usize;
    let max = n - min;
    Some((min, max))
}

mod timer {
    use std::time::Instant;

    #[cfg(all(feature = "hw_timer", target_arch = "x86_64"))]
    pub(super) type ActiveTimer = x86::RdtscpTimer;

    #[cfg(not(feature = "hw_timer"))]
    pub(super) type ActiveTimer = PlatformTimer;

    pub(super) trait Timer<T> {
        fn start() -> T;
        fn stop(start_time: T) -> u64;
    }

    pub(super) struct PlatformTimer;

    impl Timer<Instant> for PlatformTimer {
        #[inline]
        fn start() -> Instant {
            Instant::now()
        }

        #[inline]
        fn stop(start_time: Instant) -> u64 {
            start_time.elapsed().as_nanos() as u64
        }
    }

    #[cfg(all(feature = "hw_timer", target_arch = "x86_64"))]
    pub(super) mod x86 {
        use super::Timer;
        use std::arch::x86_64::{__rdtscp, _mm_mfence};

        pub struct RdtscpTimer;

        impl Timer<u64> for RdtscpTimer {
            #[inline]
            fn start() -> u64 {
                unsafe {
                    _mm_mfence();
                    __rdtscp(&mut 0)
                }
            }

            #[inline]
            fn stop(start: u64) -> u64 {
                unsafe {
                    let end = __rdtscp(&mut 0);
                    _mm_mfence();
                    end - start
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::SmallRng, RngCore, SeedableRng};
    use std::iter::Sum;

    #[test]
    fn check_summary_statistics() {
        for i in 2u32..100 {
            let range = 1..=i;
            let values = range.collect::<Vec<_>>();
            let stat = Summary::from(&values).unwrap();

            let sum = (i * (i + 1)) as f64 / 2.;
            let expected_mean = sum as f64 / i as f64;
            let expected_variance = naive_variance(values.as_slice());

            assert_eq!(stat.min, 1);
            assert_eq!(stat.n, i as usize);
            assert_eq!(stat.max, i);
            assert!(
                (stat.mean - expected_mean).abs() < 1e-5,
                "Expected close to: {}, given: {}",
                expected_mean,
                stat.mean
            );
            assert!(
                (stat.variance - expected_variance).abs() < 1e-5,
                "Expected close to: {}, given: {}",
                expected_variance,
                stat.variance
            );
        }
    }

    #[test]
    fn check_summary_statistics_types() {
        let _ = Summary::from(<&[i64]>::default());
        let _ = Summary::from(<&[u32]>::default());
        let _ = Summary::from(&Vec::<i64>::default());
    }

    #[test]
    fn check_naive_variance() {
        assert_eq!(naive_variance(&[1, 2, 3]), 1.0);
        assert_eq!(naive_variance(&[1, 2, 3, 4, 5]), 2.5);
    }

    #[test]
    fn check_running_variance() {
        let input = [1i64, 2, 3, 4, 5, 6, 7];
        let variances = RunningVariance::from(input.into_iter()).collect::<Vec<_>>();
        let expected = &[0., 0.5, 1., 1.6666, 2.5, 3.5, 4.6666];

        assert_eq!(variances.len(), expected.len());

        for (value, expected_value) in variances.iter().zip(expected) {
            assert!(
                (value - expected_value).abs() < 1e-3,
                "Expected close to: {}, given: {}",
                expected_value,
                value
            );
        }
    }

    #[test]
    fn check_running_variance_stress_test() {
        let rng = RngIterator(SmallRng::seed_from_u64(0)).map(|i| i as i64);
        let mut variances = RunningVariance::from(rng);

        assert!(variances.nth(10000000).unwrap() > 0.)
    }

    struct RngIterator<T>(T);

    impl<T: RngCore> Iterator for RngIterator<T> {
        type Item = u32;

        fn next(&mut self) -> Option<Self::Item> {
            Some(self.0.next_u32())
        }
    }

    fn naive_variance<T>(values: &[T]) -> f64
    where
        T: Sum + Copy,
        f64: From<T>,
    {
        let n = values.len() as f64;
        let mean = f64::from(values.iter().copied().sum::<T>()) / n;
        let mut sum_of_squares = 0.;
        for value in values.into_iter().copied() {
            sum_of_squares += (f64::from(value) - mean).powi(2);
        }
        sum_of_squares / (n - 1.)
    }

    #[test]
    fn check_filter_outliers() {
        let input = vec![
            1i64, -2, 3, -4, 5, -6, 7, -8, 9, -10, //
            101, -102,
        ];

        let (min, max) = max_variance_thresholds(input, 3).unwrap();
        assert!(min < 1, "Minimum is: {}", min);
        assert!(10 < max && max <= 101, "Maximum is: {}", max);
    }

    #[test]
    fn check_binomial_approximation() {
        assert_eq!(
            binomial_interval_approximation(10000000, 0.5, 0.2),
            Some((4997973, 5002027))
        );
    }
}
