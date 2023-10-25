use rand::{rngs::SmallRng, Fill, Rng, SeedableRng};
use rust_pairwise_testing::Generator;
use std::{hint::black_box, io, marker::PhantomData};

/// HTML page with a lot of chinese text to test UTF8 decoding speed
const INPUT_TEXT: &str = include_str!("./input.txt");

#[derive(Clone)]
pub struct FixedStringGenerator {
    string: String,
}

impl Generator for FixedStringGenerator {
    type Haystack = String;
    type Needle = ();

    fn next_haystack(&mut self) -> Self::Haystack {
        self.string.clone()
    }

    fn next_needle(&mut self) -> Self::Needle {}
}

pub struct RandomVec<T>(SmallRng, usize, PhantomData<T>);

impl<T> RandomVec<T> {
    #[allow(unused)]
    pub fn new(size: usize) -> Self {
        Self(SmallRng::seed_from_u64(42), size, PhantomData)
    }
}

impl<T: Default + Copy> Generator for RandomVec<T>
where
    [T]: Fill,
{
    type Haystack = Vec<T>;
    type Needle = ();

    fn next_haystack(&mut self) -> Self::Haystack {
        let RandomVec(rng, size, _) = self;
        let mut v = vec![T::default(); *size];
        rng.fill(&mut v[..]);
        v
    }

    fn next_needle(&mut self) -> Self::Needle {}
}

#[derive(Clone)]
pub struct RandomStringGenerator {
    char_indicies: Vec<usize>,
    rng: SmallRng,
    length: usize,
}

impl RandomStringGenerator {
    #[allow(unused)]
    pub fn new() -> io::Result<Self> {
        let char_indicies = INPUT_TEXT
            .char_indices()
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        let rng = SmallRng::from_entropy();
        Ok(Self {
            char_indicies,
            rng,
            length: 50000,
        })
    }
}
impl Generator for RandomStringGenerator {
    type Haystack = String;
    type Needle = ();

    fn next_haystack(&mut self) -> Self::Haystack {
        let start = self
            .rng
            .gen_range(0..self.char_indicies.len() - self.length);

        let from = self.char_indicies[start];
        let to = self.char_indicies[start + self.length];
        INPUT_TEXT[from..to].to_string()
    }

    fn name(&self) -> String {
        format!("RandomString<{}>", self.length)
    }

    fn next_needle(&mut self) -> Self::Needle {}
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
pub fn std<T>(s: &String, _: &T) -> usize {
    s.chars().count()
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn std_count<T>(s: &String, _: &T) -> usize {
    let mut l = 0;
    for _ in s.chars() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn std_count_rev<T>(s: &String, _: &T) -> usize {
    let mut l = 0;
    for _ in s.chars().rev() {
        l += 1;
    }
    l
}

#[cfg_attr(feature = "align", repr(align(32)))]
#[cfg_attr(feature = "align", inline(never))]
#[allow(unused)]
pub fn std_take<const N: usize, T>(s: &String, _: &T) -> usize {
    s.chars().take(N).count()
}
