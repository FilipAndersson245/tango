#![cfg_attr(feature = "align", feature(fn_align))]
use rand::{rngs::SmallRng, Rng, SeedableRng};
use tango_bench::{
    benchmark_fn, benchmark_fn_with_setup, cli::run, Benchmark, Generator, MeasurementSettings,
    StaticValue,
};
use test_funcs::{factorial, str_count, str_count_rev, str_std, str_take, sum, RandomString};

mod test_funcs;

struct RandomVec(SmallRng, usize);

impl Generator for RandomVec {
    type Haystack = Vec<u32>;
    type Needle = ();

    fn next_haystack(&mut self) -> Self::Haystack {
        let RandomVec(rng, size) = self;
        let mut v = vec![0; *size];
        rng.fill(&mut v[..]);
        v
    }

    fn name(&self) -> String {
        format!("RandomVec<{}>", self.1)
    }

    fn next_needle(&mut self) -> Self::Needle {}
}

fn sort_unstable<T: Ord + Copy, N>(mut input: Vec<T>, _: &N) -> T {
    input.sort_unstable();
    input[input.len() / 2]
}

fn sort_stable<T: Ord + Copy, N>(mut input: Vec<T>, _: &N) -> T {
    input.sort();
    input[input.len() / 2]
}

fn copy_and_sort_stable<T: Ord + Copy, N>(input: &Vec<T>, _: &N) -> T {
    let mut input = input.clone();
    input.sort();
    input[input.len() / 2]
}

fn main() {
    let settings = MeasurementSettings::default();

    let mut benchmark = Benchmark::default();
    benchmark.add_generator(StaticValue((), ()));

    #[cfg(feature = "aa_test")]
    {
        benchmark.add_pair(
            benchmark_fn("sum_5000", |_, _| sum(5000)),
            benchmark_fn("sum_5000", |_, _| sum(5000)),
        );

        benchmark.add_pair(
            benchmark_fn("factorial_500", |_, _| factorial(500)),
            benchmark_fn("factorial_500", |_, _| factorial(500)),
        );
    }

    benchmark.add_pair(
        benchmark_fn("sum_5000", |_, _| sum(5000)),
        benchmark_fn("sum_4950", |_, _| sum(4950)),
    );

    benchmark.add_pair(
        benchmark_fn("factorial_500", |_, _| factorial(500)),
        benchmark_fn("factorial_495", |_, _| factorial(495)),
    );

    run(benchmark, settings);

    let mut str = Benchmark::default();
    str.add_generator(RandomString::new().unwrap());

    str.add_pair(
        benchmark_fn("str_std", str_std),
        benchmark_fn("str_count", str_count),
    );
    str.add_pair(
        benchmark_fn("str_count", str_count),
        benchmark_fn("str_count_rev", str_count_rev),
    );
    str.add_pair(
        benchmark_fn("str_5000", |h, n| str_take(5000, h, n)),
        benchmark_fn("str_4950", |h, n| str_take(4950, h, n)),
    );

    run(str, settings);

    let mut benchmark = Benchmark::default();

    for size in [100, 1_000, 10_000, 100_000] {
        benchmark.add_generator(RandomVec(SmallRng::seed_from_u64(42), size));
    }

    benchmark.add_pair(
        benchmark_fn_with_setup("stable", sort_stable, Clone::clone),
        benchmark_fn_with_setup("unstable", sort_unstable, Clone::clone),
    );

    benchmark.add_pair(
        benchmark_fn_with_setup("stable", sort_stable, Clone::clone),
        benchmark_fn("stable_clone_sort", copy_and_sort_stable),
    );

    run(benchmark, settings)
}
