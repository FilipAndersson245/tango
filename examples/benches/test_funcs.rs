use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::{hint::black_box, rc::Rc};
use tango_bench::{benchmark_fn_with_setup, MeasureTarget};

/// HTML page with a lot of chinese text to test UTF8 decoding speed
pub const INPUT_TEXT: &str = include_str!("./input.txt");

#[allow(unused)]
pub(crate) fn create_str_benchmark(
    name: &'static str,
    input: &Rc<IndexedString>,
    f: fn(&str) -> usize,
) -> Box<dyn MeasureTarget> {
    let input = Rc::clone(input);
    benchmark_fn_with_setup(name, move |p| {
        let mut rng = SmallRng::seed_from_u64(p.seed);
        let input = Rc::clone(&input);
        move || f(random_substring(&input, &mut rng))
    })
}

fn random_substring<'a>(input: &'a IndexedString, rng: &mut impl Rng) -> &'a str {
    let length = 50_000;
    let indices = &input.indices;
    let start = rng.gen_range(0..indices.len() - length);
    let range = indices[start]..indices[start + length];
    &input.string[range]
}

pub(crate) struct IndexedString {
    string: String,
    indices: Vec<usize>,
}

impl From<&str> for IndexedString {
    fn from(value: &str) -> Self {
        Self {
            string: value.to_owned(),
            indices: build_char_indicies(value),
        }
    }
}

pub fn build_char_indicies(text: &str) -> Vec<usize> {
    text.char_indices().map(|(idx, _)| idx).collect()
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn sum(n: usize) -> usize {
    let mut sum = 0;
    for i in 0..black_box(n) {
        sum += black_box(i);
    }
    sum
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn factorial(mut n: usize) -> usize {
    let mut result = 1usize;
    while n > 0 {
        result = result.wrapping_mul(black_box(n));
        n -= 1;
    }
    result
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
#[allow(clippy::ptr_arg)]
pub fn str_count_rev(s: &str) -> usize {
    let mut l = 0;
    for _ in s.chars().rev() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
#[allow(clippy::ptr_arg)]
pub fn str_count(s: &str) -> usize {
    let mut l = 0;
    for _ in s.chars() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
#[allow(clippy::ptr_arg)]
pub fn str_take(n: usize, s: &str) -> usize {
    s.chars().take(black_box(n)).count()
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn sort_unstable<T: Ord + Copy, N>(input: &Vec<T>, _: &N) -> T {
    let mut input = input.clone();
    input.sort_unstable();
    input[input.len() / 2]
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn sort_stable<T: Ord + Copy, N>(input: &Vec<T>, _: &N) -> T {
    let mut input = input.clone();
    input.sort();
    input[input.len() / 2]
}
