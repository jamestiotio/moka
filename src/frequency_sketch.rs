// License and Copyright Notice:
//
// Some of the code and doc comments in this module were ported or copied from
// a Java class `com.github.benmanes.caffeine.cache.FrequencySketch` of Caffeine.
// https://github.com/ben-manes/caffeine/blob/master/caffeine/src/main/java/com/github/benmanes/caffeine/cache/FrequencySketch.java
//
// The original code/comments from Caffeine are licensed under the Apache License,
// Version 2.0 <https://github.com/ben-manes/caffeine/blob/master/LICENSE>
//
// Copyrights of the original code/comments are retained by their contributors.
// For full authorship information, see the version control history of
// https://github.com/ben-manes/caffeine/

// This class maintains a 4-bit CountMinSketch [1] with periodic aging to
// provide the popularity history for the TinyLfu admission policy [2].
// The time and space efficiency of the sketch allows it to cheaply estimate the
// frequency of an entry in a stream of cache access events.
//
// The counter matrix is represented as a single dimensional array holding 16
// counters per slot. A fixed depth of four balances the accuracy and cost,
// resulting in a width of four times the length of the array. To retain an
// accurate estimation the array's length equals the maximum number of entries
// in the cache, increased to the closest power-of-two to exploit more efficient
// bit masking. This configuration results in a confidence of 93.75% and error
// bound of e / width.
//
// The frequency of all entries is aged periodically using a sampling window
// based on the maximum number of entries in the cache. This is referred to as
// the reset operation by TinyLfu and keeps the sketch fresh by dividing all
// counters by two and subtracting based on the number of odd counters
// found. The O(n) cost of aging is amortized, ideal for hardware pre-fetching,
// and uses inexpensive bit manipulations per array location.
//
// [1] An Improved Data Stream Summary: The Count-Min Sketch and its Applications
//     http://dimacs.rutgers.edu/~graham/pubs/papers/cm-full.pdf
// [2] TinyLFU: A Highly Efficient Cache Admission Policy
//     https://dl.acm.org/citation.cfm?id=3149371

/// A probabilistic multi-set for estimating the popularity of an element within
/// a time window. The maximum frequency of an element is limited to 15 (4-bits)
/// and an aging process periodically halves the popularity of all elements.
#[allow(dead_code)]
pub(crate) struct FrequencySketch {
    // int sampleSize;
    sample_size: usize,
    // int tableMask;
    table_mask: usize,
    // long[] table;
    table: Vec<u64>,
    // int size;
    size: usize,
}

// A mixture of seeds from FNV-1a, CityHash, and Murmur3. (Taken from Caffeine)
#[allow(dead_code)]
static SEED: [u64; 4] = [
    0xc3a5c85c97cb3127,
    0xb492b66fbe98f273,
    0x9ae16a3b2f90404f,
    0xcbf29ce484222325,
];

#[allow(dead_code)]
static RESET_MASK: u64 = 0x7777777777777777;

#[allow(dead_code)]
static ONE_MASK: u64 = 0x1111111111111111;

impl FrequencySketch {
    /// Creates a frequency sketch with the capacity.
    #[allow(dead_code)]
    pub(crate) fn with_capacity(cap: usize) -> Self {
        let maximum = cap.min((i32::MAX >> 1) as usize);
        let table_size = if maximum == 0 {
            1
        } else {
            maximum.next_power_of_two()
        };
        let table = vec![0; table_size];
        let table_mask = 0.max(table_size - 1);
        let sample_size = if cap == 0 {
            10
        } else if let Some(n) = maximum.checked_mul(10) {
            n
        } else {
            i32::MAX as usize
        };
        Self {
            sample_size,
            table_mask,
            table,
            size: 0,
        }
    }

    /// Takes the hash value of an element, and returns the estimated number of
    /// occurrences of the element, up to the maximum (15).
    #[allow(dead_code)]
    pub(crate) fn frequency(&self, hash: u64) -> u8 {
        let start = ((hash & 3) << 2) as u8;
        let mut frequency = std::u8::MAX;
        for i in 0..4 {
            let index = self.index_of(hash, i);
            let count = (self.table[index] >> ((start + i) << 2) & 0xF) as u8;
            frequency = frequency.min(count);
        }
        frequency
    }

    /// Take a hash value of an element and increments the popularity of the
    /// element if it does not exceed the maximum (15). The popularity of all
    /// elements will be periodically down sampled when the observed events
    /// exceeds a threshold. This process provides a frequency aging to allow
    /// expired long term entries to fade away.
    #[allow(dead_code)]
    pub(crate) fn increment(&mut self, hash: u64) {
        let start = ((hash & 3) << 2) as u8;
        let mut added = false;
        for i in 0..4 {
            let index = self.index_of(hash, i);
            added |= self.increment_at(index, start + i);
        }

        if added {
            self.size += 1;
            if self.size >= self.sample_size {
                self.reset();
            }
        }
    }

    /// Takes a table index (each entry has 16 counters) and counter index, and
    /// increments the counter by 1 if it is not already at the maximum value
    /// (15). Returns `true` if incremented.
    fn increment_at(&mut self, table_index: usize, counter_index: u8) -> bool {
        let offset = (counter_index as usize) << 2;
        let mask = 0xF_u64 << offset;
        if self.table[table_index] & mask != mask {
            self.table[table_index] += 1u64 << offset;
            true
        } else {
            false
        }
    }

    /// Reduces every counter by half of its original value.
    fn reset(&mut self) {
        let mut count = 0u32;
        for entry in &mut self.table {
            // Count number of odd numbers.
            count += (*entry & ONE_MASK).count_ones();
            *entry = (*entry >> 1) & RESET_MASK;
        }
        self.size = (self.size >> 1) - (count >> 2) as usize;
    }

    /// Returns the table index for the counter at the specified depth.
    fn index_of(&self, hash: u64, depth: u8) -> usize {
        let i = depth as usize;
        let mut hash = (hash.wrapping_add(SEED[i])).wrapping_mul(SEED[i]);
        hash += hash >> 32;
        hash as usize & self.table_mask
    }
}

// Some test cases were ported from Caffeine at:
// https://github.com/ben-manes/caffeine/blob/master/caffeine/src/test/java/com/github/benmanes/caffeine/cache/FrequencySketchTest.java
//
// To see the debug prints, run test as `cargo test -- --nocapture`
#[cfg(test)]
mod tests {
    use super::FrequencySketch;
    use std::hash::{BuildHasher, Hash, Hasher};

    lazy_static::lazy_static! {
        static ref ITEM: u32 = {
            let mut buf = [0; 4];
            getrandom::getrandom(&mut buf).unwrap();
            unsafe { std::mem::transmute::<[u8; 4], u32>(buf) }
        };
    }

    // This test was ported from Caffeine.
    #[test]
    fn increment_once() {
        let mut sketch = FrequencySketch::with_capacity(512);
        let hasher = hasher();
        let item_hash = hasher(*ITEM);
        sketch.increment(item_hash);
        assert_eq!(sketch.frequency(item_hash), 1);
    }

    // This test was ported from Caffeine.
    #[test]
    fn increment_max() {
        let mut sketch = FrequencySketch::with_capacity(512);
        let hasher = hasher();
        let item_hash = hasher(*ITEM);
        for _ in 0..20 {
            sketch.increment(item_hash);
        }
        assert_eq!(sketch.frequency(item_hash), 15);
    }

    // This test was ported from Caffeine.
    #[test]
    fn increment_distinct() {
        let mut sketch = FrequencySketch::with_capacity(512);
        let hasher = hasher();
        sketch.increment(hasher(*ITEM));
        sketch.increment(hasher(ITEM.wrapping_add(1)));
        assert_eq!(sketch.frequency(hasher(*ITEM)), 1);
        assert_eq!(sketch.frequency(hasher(ITEM.wrapping_add(1))), 1);
        assert_eq!(sketch.frequency(hasher(ITEM.wrapping_add(2))), 0);
    }

    // This test was ported from Caffeine.
    #[test]
    fn index_of_around_zero() {
        let sketch = FrequencySketch::with_capacity(512);
        let mut indexes = std::collections::HashSet::new();
        let hashes = vec![std::u64::MAX, 0, 1];
        for hash in hashes.iter() {
            for depth in 0..4 {
                indexes.insert(sketch.index_of(*hash, depth));
            }
        }
        assert_eq!(indexes.len(), 4 * hashes.len())
    }

    // This test was ported from Caffeine.
    #[test]
    fn reset() {
        let mut reset = false;
        let mut sketch = FrequencySketch::with_capacity(64);
        let hasher = hasher();

        for i in 1..(20 * sketch.table.len()) {
            sketch.increment(hasher(i));
            if sketch.size != i {
                reset = true;
                break;
            }
        }

        assert!(reset);
        assert!(sketch.size <= sketch.sample_size / 2);
    }

    // This test was ported from Caffeine.
    #[test]
    fn heavy_hitters() {
        let mut sketch = FrequencySketch::with_capacity(512);
        let hasher = hasher();

        for i in 100..100_000 {
            sketch.increment(hasher(i));
        }

        for i in (0..10).step_by(2) {
            for _ in 0..i {
                sketch.increment(hasher(i));
            }
        }

        // A perfect popularity count yields an array [0, 0, 2, 0, 4, 0, 6, 0, 8, 0]
        let popularity = (0..10)
            .map(|i| sketch.frequency(hasher(i)))
            .collect::<Vec<_>>();

        for (i, freq) in popularity.iter().enumerate() {
            match i {
                2 => assert!(freq <= &popularity[4]),
                4 => assert!(freq <= &popularity[6]),
                6 => assert!(freq <= &popularity[8]),
                8 => (),
                _ => assert!(freq <= &popularity[2]),
            }
        }
    }

    fn hasher<K: Hash>() -> impl Fn(K) -> u64 {
        let build_hasher = std::collections::hash_map::RandomState::default();
        move |key| {
            let mut hasher = build_hasher.build_hasher();
            key.hash(&mut hasher);
            hasher.finish()
        }
    }
}
