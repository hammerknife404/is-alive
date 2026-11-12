//! Hostile input for tests. Release builds end on any panic (`panic =
//! "abort"`), so a reply from a device on a client's network, or a file
//! dropped on the window, must never be able to cause one. Each parser
//! of outside data has a test that feeds it `each` of these: its good
//! sample cut short at every length, with bytes flipped, and noise.

/// A small fixed-seed generator: the same inputs every run.
pub struct Rnd(u64);

impl Default for Rnd {
    fn default() -> Self {
        Self(0x9E37_79B9_7F4A_7C15)
    }
}

impl Rnd {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n.max(1)
    }
}

/// `good` cut short, corrupted and replaced by noise, given to `f`.
/// Long samples are cut at a stride so a test stays quick.
pub fn each(good: &[u8], mut f: impl FnMut(&[u8])) {
    let mut rnd = Rnd::default();
    let stride = (good.len() / 400).max(1);
    for n in (0..=good.len()).step_by(stride) {
        f(&good[..n]);
    }
    if !good.is_empty() {
        // The bytes that say how long something is are the dangerous
        // ones: every position gets the extremes, then random damage.
        for i in (0..good.len()).step_by(stride) {
            for v in [0x00, 0x7f, 0x80, 0xff] {
                let mut d = good.to_vec();
                d[i] = v;
                f(&d);
            }
        }
        for _ in 0..400 {
            let mut d = good.to_vec();
            for _ in 0..1 + rnd.below(4) {
                let i = rnd.below(d.len());
                d[i] ^= (rnd.next() as u8) | 1;
            }
            f(&d);
        }
    }
    for len in [0, 1, 2, 3, 4, 11, 12, 13, 31, 64, 513] {
        let noise: Vec<u8> = (0..len).map(|_| rnd.next() as u8).collect();
        f(&noise);
        f(&vec![0xff; len]);
        f(&vec![0; len]);
    }
}

/// Text that trips code slicing by bytes: `good` cut at every character,
/// and with multi-byte characters, controls and odd line ends put in.
pub fn each_text(good: &str, mut f: impl FnMut(&str)) {
    const ODD: &[&str] = &[
        "\u{e9}",
        "\u{200b}",
        "\u{1f511}",
        "\u{1f469}\u{200d}\u{1f4bb}",
        "\u{fe0f}",
        "\r",
        "\n",
        "\t",
        "\0",
        "\u{ff5c}",
        "`",
        "*",
        "[",
        "](",
        "|",
        "#",
        "<",
        ">",
        "&",
        "\\",
        ":",
        "=",
        "\"",
        "{",
        "}",
        "\u{202e}",
    ];
    let mut rnd = Rnd::default();
    let cuts: Vec<usize> = good.char_indices().map(|(i, _)| i).collect();
    let stride = (cuts.len() / 300).max(1);
    for &c in cuts.iter().step_by(stride) {
        f(&good[..c]);
        f(&good[c..]);
    }
    for _ in 0..300 {
        let mut s = good.to_string();
        for _ in 0..1 + rnd.below(5) {
            let at = cuts.get(rnd.below(cuts.len())).copied().unwrap_or(0);
            let at = (0..=at).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
            s.insert_str(at.min(s.len()), ODD[rnd.below(ODD.len())]);
        }
        f(&s);
    }
    for o in ODD {
        f(o);
        f(&o.repeat(40));
    }
    f("");
}
