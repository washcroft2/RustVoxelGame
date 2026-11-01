use voxel_core::CHUNK_VOLUME;

/// Values per chunk array, as an index bound.
const LEN: usize = CHUNK_VOLUME;

#[derive(Clone, Debug)]
enum Storage {
    Uniform(u16),
    /// `bits` is 1, 2, 4 or 8. Entries never straddle a word, so reads and
    /// writes touch one `u64`. `palette[i]` is the value for index `i`.
    Packed {
        bits: u8,
        palette: Vec<u16>,
        words: Vec<u64>,
    },
    /// 16 bits per entry, no palette.
    Direct(Vec<u16>),
}

/// A 32³ array of `u16` values stored as uniform, 1/2/4/8-bit palette
/// indices, or 16-bit direct values, growing as more distinct values appear.
///
/// Used for block IDs and fluid IDs. Equality compares contents, not the
/// storage form.
#[derive(Clone, Debug)]
pub struct PalettedArray {
    storage: Storage,
}

fn bits_for(distinct: usize) -> u8 {
    match distinct {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        17..=256 => 8,
        _ => 16,
    }
}

fn words_for(bits: u8) -> usize {
    LEN * bits as usize / 64
}

impl PalettedArray {
    /// An array where every entry is `value`. Costs no payload memory.
    pub fn uniform(value: u16) -> Self {
        Self {
            storage: Storage::Uniform(value),
        }
    }

    /// Builds the smallest form that holds `values`; `None` unless it has exactly [`CHUNK_VOLUME`] entries.
    pub fn from_slice(values: &[u16]) -> Option<Self> {
        if values.len() != LEN {
            return None;
        }
        // 64K-bit presence set: distinct values come out sorted, so the
        // palette can be searched by bisection while packing.
        let mut seen = [0u64; 1024];
        let mut distinct = 0usize;
        for &v in values {
            let (w, b) = (v as usize >> 6, v as u64 & 63);
            if seen[w] >> b & 1 == 0 {
                seen[w] |= 1 << b;
                distinct += 1;
            }
        }
        if distinct == 1 {
            return Some(Self::uniform(values[0]));
        }
        if distinct > 256 {
            return Some(Self {
                storage: Storage::Direct(values.to_vec()),
            });
        }
        let mut palette = Vec::with_capacity(distinct);
        for (w, &word) in seen.iter().enumerate() {
            let mut m = word;
            while m != 0 {
                palette.push((w * 64) as u16 + m.trailing_zeros() as u16);
                m &= m - 1;
            }
        }
        let bits = bits_for(palette.len());
        let mut words = vec![0u64; words_for(bits)];
        for (i, &v) in values.iter().enumerate() {
            // Present by construction.
            let idx = palette.binary_search(&v).unwrap_or(0) as u64;
            write_packed(&mut words, bits, i, idx);
        }
        Some(Self {
            storage: Storage::Packed {
                bits,
                palette,
                words,
            },
        })
    }

    /// Value at flat index `i` (see `LocalPos::index`). Out-of-range reads return 0.
    pub fn get(&self, i: usize) -> u16 {
        if i >= LEN {
            return 0;
        }
        match &self.storage {
            Storage::Uniform(v) => *v,
            Storage::Packed {
                bits,
                palette,
                words,
            } => {
                // A valid index is below the palette length by construction.
                palette
                    .get(read_packed(words, *bits, i) as usize)
                    .copied()
                    .unwrap_or(0)
            }
            Storage::Direct(v) => v[i],
        }
    }

    /// Stores `value` at `i` and returns the old value. Out-of-range writes do nothing and return 0.
    pub fn set(&mut self, i: usize, value: u16) -> u16 {
        if i >= LEN {
            return 0;
        }
        let old = self.get(i);
        if old == value {
            return old;
        }
        match &mut self.storage {
            Storage::Uniform(u) => {
                let mut words = vec![0u64; words_for(1)];
                write_packed(&mut words, 1, i, 1);
                self.storage = Storage::Packed {
                    bits: 1,
                    palette: vec![*u, value],
                    words,
                };
            }
            Storage::Direct(v) => v[i] = value,
            Storage::Packed {
                bits,
                palette,
                words,
            } => {
                let idx = match palette.iter().position(|&p| p == value) {
                    Some(idx) => idx,
                    None => {
                        palette.push(value);
                        let needed = bits_for(palette.len());
                        if needed != *bits {
                            if needed == 16 {
                                // Leave the palette push undone: Direct needs none.
                                palette.pop();
                                let mut direct = self.to_vec();
                                direct[i] = value;
                                self.storage = Storage::Direct(direct);
                                return old;
                            }
                            *words = repack(words, *bits, needed);
                            *bits = needed;
                        }
                        palette.len() - 1
                    }
                };
                write_packed(words, *bits, i, idx as u64);
            }
        }
        old
    }

    /// Makes every entry `value` and releases the payload.
    pub fn fill(&mut self, value: u16) {
        self.storage = Storage::Uniform(value);
    }

    pub fn is_uniform(&self) -> bool {
        matches!(self.storage, Storage::Uniform(_))
    }

    /// The value, when every entry is the same by storage form. A packed array
    /// that has drifted to one value reports `None` until [`Self::compact`].
    pub fn uniform_value(&self) -> Option<u16> {
        match self.storage {
            Storage::Uniform(v) => Some(v),
            _ => None,
        }
    }

    /// Bits per entry: 0 for uniform, else 1, 2, 4, 8 or 16.
    pub fn bits(&self) -> u8 {
        match &self.storage {
            Storage::Uniform(_) => 0,
            Storage::Packed { bits, .. } => *bits,
            Storage::Direct(_) => 16,
        }
    }

    /// Bytes of block data, excluding the palette and struct overhead.
    pub fn payload_bytes(&self) -> usize {
        match &self.storage {
            Storage::Uniform(_) => 0,
            Storage::Packed { words, .. } => words.len() * 8,
            Storage::Direct(v) => v.len() * 2,
        }
    }

    /// Re-packs into the smallest form that holds the values now present.
    /// Palettes only grow on writes, so call this after bulk edits or before saving.
    pub fn compact(&mut self) {
        let values = self.to_vec();
        if let Some(fresh) = Self::from_slice(&values) {
            *self = fresh;
        }
    }

    /// All entries in flat-index order.
    pub fn to_vec(&self) -> Vec<u16> {
        let mut out = Vec::with_capacity(LEN);
        out.extend((0..LEN).map(|i| self.get(i)));
        out
    }

    /// Number of entries equal to `value`.
    pub fn count(&self, value: u16) -> usize {
        match &self.storage {
            Storage::Uniform(v) => {
                if *v == value {
                    LEN
                } else {
                    0
                }
            }
            _ => (0..LEN).filter(|&i| self.get(i) == value).count(),
        }
    }
}

impl PartialEq for PalettedArray {
    fn eq(&self, other: &Self) -> bool {
        (0..LEN).all(|i| self.get(i) == other.get(i))
    }
}

impl Eq for PalettedArray {}

fn read_packed(words: &[u64], bits: u8, i: usize) -> u64 {
    let per = 64 / bits as usize;
    let shift = (i % per) as u32 * bits as u32;
    let mask = (1u64 << bits) - 1;
    words.get(i / per).map_or(0, |w| w >> shift & mask)
}

fn write_packed(words: &mut [u64], bits: u8, i: usize, value: u64) {
    let per = 64 / bits as usize;
    let shift = (i % per) as u32 * bits as u32;
    let mask = (1u64 << bits) - 1;
    if let Some(w) = words.get_mut(i / per) {
        *w = (*w & !(mask << shift)) | ((value & mask) << shift);
    }
}

/// Re-encodes palette indices at a wider bit width.
fn repack(words: &[u64], from: u8, to: u8) -> Vec<u64> {
    let mut out = vec![0u64; words_for(to)];
    for i in 0..LEN {
        write_packed(&mut out, to, i, read_packed(words, from, i));
    }
    out
}
