//! Peel a UniFi backup down to its MongoDB dump (`db.gz`, gunzipped).
//!
//! | File | Cipher | Inside |
//! |---|---|---|
//! | `.unf` (site export, classic autobackup) | AES-128-CBC, static key and IV | zip with `db.gz` |
//! | `.unifi` (UniFi OS console backup) | AES-256-CBC, static key, IV = first 16 bytes | gzip → tar with `backup/network/db.gz` |
//!
//! Both use no padding: plaintext is zero-filled to a block, so the zip and
//! gzip readers here tolerate trailing bytes. Format is sniffed from content,
//! not the extension, so a `.zip` wrapping a `.unf` or a bare `db.gz` work too.

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::NoPadding};
use flate2::read::{DeflateDecoder, GzDecoder};
use std::io::{self, Read};

// UniFi's backup-format constants: the same in every controller, and
// published by every tool that reads a .unf or .unifi backup. They're not
// secrets (the format can't be read without them); the backup's own
// passwords and keys are what the converter strips.
const UNF_KEY: &[u8; 16] = b"bcyangkmluohmars";
const UNF_IV: &[u8; 16] = b"ubntenterpriseap";
const UNIFI_KEY: [u8; 32] = [
    0xe3, 0x83, 0xb7, 0xc5, 0x36, 0x98, 0xb3, 0x6d, 0x4b, 0xae, 0xa4, 0xed, 0x22, 0x18, 0x1e, 0xf7,
    0x36, 0x76, 0xbf, 0xd5, 0xd5, 0xb9, 0x00, 0x05, 0xd9, 0x84, 0x5f, 0xfd, 0x5d, 0xce, 0x98, 0x5f,
];

const ZIP_LOCAL: &[u8] = b"PK\x03\x04";
const ZIP_CENTRAL: &[u8] = b"PK\x01\x02";
const ZIP_END: &[u8] = b"PK\x05\x06";
const GZIP: &[u8] = &[0x1f, 0x8b];
const MAX_DEPTH: u8 = 4;

pub struct Dump {
    /// Raw BSON document stream.
    pub bson: Vec<u8>,
    /// Layers peeled, outermost first, e.g. [".unf (AES-128)", "zip", "db.gz"].
    pub layers: Vec<String>,
    /// UniFi Network version, from the backup's `version` entry.
    pub version: Option<String>,
}

pub fn open(data: Vec<u8>) -> Result<Dump, String> {
    let mut d = Dump {
        bson: Vec::new(),
        layers: Vec::new(),
        version: None,
    };
    d.bson = peel(data, &mut d, 0)?;
    Ok(d)
}

fn peel(data: Vec<u8>, d: &mut Dump, depth: u8) -> Result<Vec<u8>, String> {
    if depth > MAX_DEPTH {
        return Err("backup nested too deeply".into());
    }
    if data.starts_with(ZIP_LOCAL) {
        d.layers.push("zip".into());
        return from_zip(&data, d, depth);
    }
    if data.starts_with(GZIP) {
        let mut gz = GzDecoder::new(&data[..]);
        // A tar shows its "ustar" magic at byte 257; db.gz is BSON straight away.
        let mut head = vec![0; 512];
        let n = read_full(&mut gz, &mut head).map_err(|e| format!("gzip: {e}"))?;
        let tar = n == 512 && &head[257..262] == b"ustar";
        head.truncate(n);
        let rest = io::Cursor::new(head).chain(gz);
        if tar {
            d.layers.extend(["gzip".into(), "tar".into()]);
            return from_tar(rest, d, depth);
        }
        d.layers.push("db.gz".into());
        return gunzip_rest(rest);
    }
    if starts_with_after::<aes::Aes128>(UNF_KEY, UNF_IV, &data, ZIP_LOCAL) {
        d.layers.push(".unf (AES-128)".into());
        return peel(decrypt::<aes::Aes128>(UNF_KEY, UNF_IV, data)?, d, depth + 1);
    }
    if data.len() > 16
        && starts_with_after::<aes::Aes256>(&UNIFI_KEY, &data[..16], &data[16..], GZIP)
    {
        let iv: [u8; 16] = data[..16].try_into().unwrap();
        let mut body = data;
        body.drain(..16);
        d.layers.push(".unifi (AES-256)".into());
        return peel(decrypt::<aes::Aes256>(&UNIFI_KEY, &iv, body)?, d, depth + 1);
    }
    Err("not a UniFi backup: expected a .unf, .unifi, a zip holding one, or db.gz".into())
}

/// Whether AES-CBC decryption of `data` would start with `magic`. Only the
/// first block is decrypted, so a wrong guess costs nothing.
fn starts_with_after<C>(key: &[u8], iv: &[u8], data: &[u8], magic: &[u8]) -> bool
where
    C: aes::cipher::BlockCipherDecrypt + aes::cipher::KeyInit,
    cbc::Decryptor<C>: KeyIvInit,
{
    if data.len() < 16 || !data.len().is_multiple_of(16) {
        return false;
    }
    let mut first = data[..16].to_vec();
    decrypt_in_place::<C>(key, iv, &mut first).is_ok() && first.starts_with(magic)
}

/// AES-CBC, no padding, in place: a large backup isn't held twice.
fn decrypt<C>(key: &[u8], iv: &[u8], mut data: Vec<u8>) -> Result<Vec<u8>, String>
where
    C: aes::cipher::BlockCipherDecrypt + aes::cipher::KeyInit,
    cbc::Decryptor<C>: KeyIvInit,
{
    decrypt_in_place::<C>(key, iv, &mut data)?;
    Ok(data)
}

fn decrypt_in_place<C>(key: &[u8], iv: &[u8], buf: &mut [u8]) -> Result<(), String>
where
    C: aes::cipher::BlockCipherDecrypt + aes::cipher::KeyInit,
    cbc::Decryptor<C>: KeyIvInit,
{
    cbc::Decryptor::<C>::new_from_slices(key, iv)
        .map_err(|_| "bad key length")?
        .decrypt_padded::<NoPadding>(buf)
        .map(|_| ())
        .map_err(|_| "decryption failed".to_string())
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Most any layer may unpack to. A real controller's dump is far smaller;
/// a crafted file that inflates without end stops here instead of taking
/// all the memory.
pub const MAX_UNPACKED: usize = 2 << 30;

/// Most a file picked out of a zip by name (a map, a manifest) unpacks to.
const FIND_MAX: usize = 64 << 20;
/// Most all the files `zip_all` reads unpack to together.
const ALL_MAX: usize = 512 << 20;
/// Most a backup's `version` entry may be; a real one is a line.
const VERSION_MAX: usize = 4096;

/// Read to the end, but no further than MAX_UNPACKED.
fn read_capped(r: impl Read, what: &str) -> Result<Vec<u8>, String> {
    read_upto(r, what, MAX_UNPACKED)
}

/// Read to the end, but no further than `max` bytes.
fn read_upto(r: impl Read, what: &str, max: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    r.take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("{what}: {e}"))?;
    if out.len() > max {
        return Err(if max >= 1 << 30 {
            format!(
                "{what} unpacks to more than {} GB: not a UniFi backup",
                max >> 30
            )
        } else if max >= 1 << 20 {
            format!("{what} unpacks to more than {} MB", max >> 20)
        } else {
            format!("{what} unpacks to more than {max} bytes")
        });
    }
    Ok(out)
}

fn gunzip_rest(r: impl Read) -> Result<Vec<u8>, String> {
    read_capped(r, "db.gz")
}

/// Single gzip member; trailing padding is ignored.
fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    gunzip_rest(GzDecoder::new(data))
}

/// What to do with an archive entry, by name.
enum Want {
    Dump,
    Version,
    Nested,
    Skip,
}

fn want(name: &str) -> Want {
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    match base {
        "db.gz" => Want::Dump,
        "version" => Want::Version,
        _ if [".unf", ".unifi", ".zip"].iter().any(|e| base.ends_with(e)) => Want::Nested,
        _ => Want::Skip,
    }
}

/// Take the first dump found: the network `db.gz` directly, else one inside
/// a nested backup.
fn take_entry(
    name: &str,
    read: impl FnOnce() -> Result<Vec<u8>, String>,
    d: &mut Dump,
    depth: u8,
) -> Result<Option<Vec<u8>>, String> {
    Ok(match want(name) {
        Want::Dump => {
            d.layers.push(name.to_string());
            Some(gunzip(&read()?).map_err(|e| format!("{name}: {e}"))?)
        }
        Want::Version => {
            let v = read()?;
            d.version = Some(crate::input::line(&String::from_utf8_lossy(&v), 64));
            None
        }
        Want::Nested => {
            let mark = d.layers.len();
            d.layers.push(name.to_string());
            match peel(read()?, d, depth + 1) {
                Ok(b) => Some(b),
                Err(_) => {
                    d.layers.truncate(mark);
                    None
                }
            }
        }
        Want::Skip => None,
    })
}

struct ZipEntry {
    name: String,
    method: u16,
    size: usize,
    local: usize,
}

fn u16le(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn u32le(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize
}

/// Central directory records running from `start` up to exactly `end`,
/// walking no more than `budget` records (taken from it as they're read).
fn central_run(b: &[u8], start: usize, end: usize, budget: &mut usize) -> Option<Vec<ZipEntry>> {
    let mut out = Vec::new();
    let mut p = start;
    while p < end {
        if *budget == 0 || b.get(p..p + 46)?.get(..4)? != ZIP_CENTRAL {
            return None;
        }
        *budget -= 1;
        let (n, x, c) = (u16le(b, p + 28), u16le(b, p + 30), u16le(b, p + 32));
        out.push(ZipEntry {
            name: String::from_utf8_lossy(b.get(p + 46..p + 46 + n)?).into_owned(),
            method: u16le(b, p + 10) as u16,
            size: u32le(b, p + 20),
            local: u32le(b, p + 42),
        });
        p += 46 + n + x + c;
    }
    (p == end).then_some(out)
}

/// Most directory records the search for a damaged zip's directory reads.
const DIRECTORY_STEPS: usize = 200_000;

/// The zip's directory. Decrypted .unf files carry zero padding and can have
/// a stale end record, so the directory is found by walking back from the
/// last end record rather than trusting its offset.
fn zip_entries(b: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let end = b
        .windows(4)
        .rposition(|w| w == ZIP_END)
        .ok_or("zip: no central directory (file truncated?)")?;
    // Every candidate start is walked to the end, so a crafted file with
    // many chained records that overshoot it would cost the square of
    // their number: the walking is capped for the whole search. A real
    // backup's directory is a few dozen records.
    let mut budget = DIRECTORY_STEPS;
    if let Some(e) = b
        .get(end + 16..end + 20)
        .and_then(|_| central_run(b, u32le(b, end + 16), end, &mut budget))
    {
        return Ok(e);
    }
    b[..end]
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == ZIP_CENTRAL)
        .find_map(|(p, _)| central_run(b, p, end, &mut budget))
        .ok_or_else(|| "zip: central directory damaged".into())
}

fn zip_read(b: &[u8], e: &ZipEntry) -> Result<Vec<u8>, String> {
    zip_read_upto(b, e, MAX_UNPACKED)
}

/// An entry's bytes, unpacked to no more than `max`.
fn zip_read_upto(b: &[u8], e: &ZipEntry, max: usize) -> Result<Vec<u8>, String> {
    let bad = || format!("zip: {} damaged", e.name);
    let h = b.get(e.local..e.local + 30).ok_or_else(bad)?;
    if &h[..4] != ZIP_LOCAL {
        return Err(bad());
    }
    let start = e.local + 30 + u16le(h, 26) + u16le(h, 28);
    let raw = b.get(start..start + e.size).ok_or_else(bad)?;
    match e.method {
        0 if raw.len() > max && max >= 1 << 20 => {
            Err(format!("zip: {} is more than {} MB", e.name, max >> 20))
        }
        0 if raw.len() > max => Err(format!("zip: {} is more than {max} bytes", e.name)),
        0 => Ok(raw.to_vec()),
        8 => read_upto(DeflateDecoder::new(raw), &format!("zip: {}", e.name), max),
        m => Err(format!("zip: {} uses unsupported compression {m}", e.name)),
    }
}

/// The first file in a zip whose name `pick` accepts, as (name, bytes);
/// one that unpacks past 64 MB is an error.
pub fn zip_find(
    b: &[u8],
    pick: impl Fn(&str) -> bool,
) -> Result<Option<(String, Vec<u8>)>, String> {
    if !b.starts_with(ZIP_LOCAL) {
        return Ok(None);
    }
    match zip_entries(b)?.into_iter().find(|e| pick(&e.name)) {
        Some(e) => zip_read_upto(b, &e, FIND_MAX).map(|d| Some((e.name, d))),
        None => Ok(None),
    }
}

/// Every file in a zip whose name `pick` accepts, as (name, bytes); at most
/// `max` of them, unpacking to 512 MB together.
pub fn zip_all(
    b: &[u8],
    pick: impl Fn(&str) -> bool,
    max: usize,
    budget: usize,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !b.starts_with(ZIP_LOCAL) {
        return Ok(Vec::new());
    }
    // A budget for them all, so many small entries that each inflate a
    // long way can't add up to all the memory.
    let mut left = budget.min(ALL_MAX);
    zip_entries(b)?
        .into_iter()
        .filter(|e| pick(&e.name))
        .take(max)
        .map(|e| {
            let d = zip_read_upto(b, &e, left)?;
            left -= d.len();
            Ok((e.name, d))
        })
        .collect()
}

fn from_zip(b: &[u8], d: &mut Dump, depth: u8) -> Result<Vec<u8>, String> {
    let entries = zip_entries(b)?;
    // Version first so it's known even when the dump sits in a nested
    // backup; only these and skipped entries are left, so it's a line at most.
    for e in entries
        .iter()
        .filter(|e| !matches!(want(&e.name), Want::Dump | Want::Nested))
    {
        take_entry(&e.name, || zip_read_upto(b, e, VERSION_MAX), d, depth)?;
    }
    let mut dumps: Vec<_> = entries
        .iter()
        .filter(|e| matches!(want(&e.name), Want::Dump | Want::Nested))
        .collect();
    dumps.sort_by_key(|e| !matches!(want(&e.name), Want::Dump));
    for e in dumps {
        if let Some(bson) = take_entry(&e.name, || zip_read(b, e), d, depth)? {
            return Ok(bson);
        }
    }
    Err("zip has no db.gz (is this a UniFi Network backup?)".into())
}

fn octal(field: &[u8]) -> Result<usize, String> {
    let s = String::from_utf8_lossy(field);
    let s = s.trim_matches(|c: char| c == '\0' || c == ' ');
    usize::from_str_radix(if s.is_empty() { "0" } else { s }, 8)
        .map_err(|_| format!("tar: bad size field {s:?}"))
}

fn cfield(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

/// Stream the tar, keeping only the entries we want in memory.
fn from_tar(mut r: impl Read, d: &mut Dump, depth: u8) -> Result<Vec<u8>, String> {
    let io_err = |e: io::Error| format!("tar: {e}");
    let mut long_name = None;
    loop {
        let mut h = [0u8; 512];
        if read_full(&mut r, &mut h).map_err(io_err)? < 512 || h.iter().all(|&c| c == 0) {
            break;
        }
        let size = octal(&h[124..136])?;
        if size > MAX_UNPACKED {
            return Err(format!("tar: entry of {size} bytes: not a UniFi backup"));
        }
        let padded = size.div_ceil(512) * 512;
        // Read what's there rather than allocate what the header claims.
        let read_body = |r: &mut dyn Read| -> Result<Vec<u8>, String> {
            let mut v = Vec::new();
            r.take(padded as u64).read_to_end(&mut v).map_err(io_err)?;
            if v.len() < padded {
                return Err("tar: truncated".into());
            }
            v.truncate(size);
            Ok(v)
        };
        if h[156] == b'L' {
            if size > 64 * 1024 {
                return Err("tar: long file name of more than 64 KB".into());
            }
            long_name = Some(cfield(&read_body(&mut r)?));
            continue;
        }
        let name = long_name.take().unwrap_or_else(|| {
            let prefix = cfield(&h[345..500]);
            let name = cfield(&h[..100]);
            if &h[257..262] == b"ustar" && !prefix.is_empty() {
                format!("{prefix}/{name}")
            } else {
                name
            }
        });
        // Only the network app's files: the console's own config and
        // database never hold a Mongo dump.
        let file = h[156] == b'0' || h[156] == 0;
        let network = name.contains("network/") || !name.contains('/');
        let take = match want(&name) {
            Want::Dump | Want::Version => network,
            Want::Nested => true,
            Want::Skip => false,
        };
        if file && take && matches!(want(&name), Want::Version) && size > VERSION_MAX {
            return Err(format!(
                "tar: version entry of {size} bytes: not a UniFi backup"
            ));
        }
        if file && take {
            if let Some(bson) = take_entry(&name, || read_body(&mut r), d, depth)? {
                return Ok(bson);
            }
            continue;
        }
        io::copy(&mut (&mut r).take(padded as u64), &mut io::sink()).map_err(io_err)?;
    }
    Err("tar has no network/db.gz (backup without UniFi Network?)".into())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A directory of chained records that overshoot the end, tried from
    /// every start: refused at once rather than walked (the square of
    /// 20,000 records).
    #[test]
    fn a_crafted_directory_is_refused_quickly() {
        let n = 20_000;
        let mut b = Vec::new();
        for i in 0..n {
            let mut r = vec![0u8; 46];
            r[..4].copy_from_slice(ZIP_CENTRAL);
            if i == n - 1 {
                // The last one's name runs past the end.
                r[28..30].copy_from_slice(&0xffffu16.to_le_bytes());
            }
            b.extend(r);
        }
        let mut end = vec![0u8; 22];
        end[..4].copy_from_slice(ZIP_END);
        end[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        b.extend(end);
        let t = std::time::Instant::now();
        assert!(zip_entries(&b).is_err());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
    }
    use aes::cipher::BlockModeEncrypt;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    pub fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// Stored zip; `stale_end` writes a wrong directory offset like a
    /// padded .unf can carry.
    pub fn zip(files: &[(&str, &[u8])], stale_end: bool) -> Vec<u8> {
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for (name, data) in files {
            let local = out.len() as u32;
            let sizes = [
                &[0u8; 4][..], // crc, unchecked here
                &(data.len() as u32).to_le_bytes(),
                &(data.len() as u32).to_le_bytes(),
            ]
            .concat();
            out.extend(ZIP_LOCAL);
            out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend(&sizes);
            out.extend((name.len() as u16).to_le_bytes());
            out.extend([0, 0]);
            out.extend(name.as_bytes());
            out.extend(*data);
            cd.extend(ZIP_CENTRAL);
            cd.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            cd.extend(&sizes);
            cd.extend((name.len() as u16).to_le_bytes());
            cd.extend([0; 12]);
            cd.extend(local.to_le_bytes());
            cd.extend(name.as_bytes());
        }
        let cd_at = if stale_end { 7 } else { out.len() as u32 };
        let n = files.len() as u16;
        let cd_len = cd.len() as u32;
        out.extend(cd);
        out.extend(ZIP_END);
        out.extend([0, 0, 0, 0]);
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend(cd_len.to_le_bytes());
        out.extend(cd_at.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    pub fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[156] = b'0';
            h[257..263].copy_from_slice(b"ustar\0");
            out.extend(h);
            out.extend(*data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend([0; 1024]);
        out
    }

    fn pad(mut v: Vec<u8>) -> Vec<u8> {
        v.resize(v.len().div_ceil(16) * 16, 0);
        v
    }

    pub fn unf(plain: Vec<u8>) -> Vec<u8> {
        let mut b = pad(plain);
        let n = b.len();
        cbc::Encryptor::<aes::Aes128>::new(UNF_KEY.into(), UNF_IV.into())
            .encrypt_padded::<NoPadding>(&mut b, n)
            .unwrap();
        b
    }

    pub fn unifi(plain: Vec<u8>) -> Vec<u8> {
        let iv = [7u8; 16];
        let mut b = pad(plain);
        let n = b.len();
        cbc::Encryptor::<aes::Aes256>::new(&UNIFI_KEY.into(), &iv.into())
            .encrypt_padded::<NoPadding>(&mut b, n)
            .unwrap();
        [&iv[..], &b].concat()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Ciphertext from OpenSSL, not from the crates under test, so a
    /// cipher upgrade that breaks both directions the same way still fails
    /// here. The round-trip tests below can't catch that.
    ///   printf 'PK\x03\x04is-alive known answer, 2 blk' |
    ///     openssl enc -aes-128-cbc -nopad -K <UNF_KEY hex> -iv <UNF_IV hex>
    ///   printf '\x1f\x8b\x08\x00is-alive known answer, 2 blk' |
    ///     openssl enc -aes-256-cbc -nopad -K <UNIFI_KEY hex> -iv 0707…07
    /// (the IV `unifi` below writes at the head of its test files).
    #[test]
    fn aes_cbc_matches_openssl() {
        let tail = b"is-alive known answer, 2 blk";
        let unf_plain = [ZIP_LOCAL, tail].concat();
        let unf_ct = unhex("e39f29748a2d96b326fa66d385a0c42d2c1a1d0f5a0f53849b70a20b71fddc52");
        assert_eq!(unf(unf_plain.clone()), unf_ct);
        assert_eq!(
            decrypt::<aes::Aes128>(UNF_KEY, UNF_IV, unf_ct.clone()).unwrap(),
            unf_plain
        );
        assert!(starts_with_after::<aes::Aes128>(
            UNF_KEY, UNF_IV, &unf_ct, ZIP_LOCAL
        ));

        // A whole .unifi file as the console writes it: its IV, then the
        // ciphertext. Split where `peel` splits it, so the layout is checked
        // as well as the cipher.
        let unifi_plain = [&[0x1f, 0x8b, 0x08, 0x00][..], tail].concat();
        let file = unhex(concat!(
            "07070707070707070707070707070707",
            "be34318870a6ebe749545f743b0d72327e1ffd2edf95a1ac64c198f34f48de17",
        ));
        assert_eq!(unifi(unifi_plain.clone()), file);
        let (iv, body) = file.split_at(16);
        assert!(starts_with_after::<aes::Aes256>(&UNIFI_KEY, iv, body, GZIP));
        assert_eq!(
            decrypt::<aes::Aes256>(&UNIFI_KEY, iv, body.to_vec()).unwrap(),
            unifi_plain
        );
    }

    const BSON: &[u8] = b"\x05\0\0\0\0";

    #[test]
    fn unf_with_stale_directory() {
        let z = zip(&[("version", b"9.0.114\n"), ("db.gz", &gz(BSON))], true);
        let d = open(unf(z)).unwrap();
        assert_eq!(d.bson, BSON);
        assert_eq!(d.version.as_deref(), Some("9.0.114"));
        assert_eq!(d.layers, [".unf (AES-128)", "zip", "db.gz"]);
    }

    #[test]
    fn zip_wrapping_unf() {
        let inner = unf(zip(&[("db.gz", &gz(BSON))], false));
        let outer = zip(&[("readme.txt", b"hi"), ("site_unf.unf", &inner)], false);
        let d = open(outer).unwrap();
        assert_eq!(d.bson, BSON);
        assert_eq!(
            d.layers,
            ["zip", "site_unf.unf", ".unf (AES-128)", "zip", "db.gz"]
        );
    }

    #[test]
    fn unifi_console_backup() {
        let t = tar(&[
            ("backup/metadata.json", b"{}"),
            ("backup/ucore/config/x.key", b"secret"),
            ("backup/network/version", b"9.1.0"),
            ("backup/network/db.gz", &gz(BSON)),
        ]);
        let d = open(unifi(gz(&t))).unwrap();
        assert_eq!(d.bson, BSON);
        assert_eq!(d.version.as_deref(), Some("9.1.0"));
        assert_eq!(
            d.layers,
            [".unifi (AES-256)", "gzip", "tar", "backup/network/db.gz"]
        );
    }

    #[test]
    fn bare_db_gz() {
        let d = open(gz(BSON)).unwrap();
        assert_eq!(
            (d.bson.as_slice(), d.layers.as_slice()),
            (BSON, &["db.gz".to_string()][..])
        );
    }

    #[test]
    fn rejects_other_files() {
        assert!(open(b"[{\"__cmd\":\"select\"}]".to_vec()).is_err());
        assert!(open(vec![0; 64]).is_err());
        assert!(open(zip(&[("a.txt", b"x")], false)).is_err());
    }

    #[test]
    fn inflating_past_the_cap_stops() {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), Compression::best());
        e.write_all(&vec![0u8; 8 << 20]).unwrap();
        let bomb = e.finish().unwrap();
        assert!(bomb.len() < 64 << 10);
        let read = |max| read_upto(DeflateDecoder::new(&bomb[..]), "x", max);
        assert!(read(1 << 20).unwrap_err().contains("1 MB"));
        assert_eq!(read(8 << 20).unwrap().len(), 8 << 20);
    }

    #[test]
    fn versions_are_one_clean_line() {
        let z = zip(
            &[("version", b"9.0.114\n\x1b[2J"), ("db.gz", &gz(BSON))],
            false,
        );
        assert_eq!(open(z).unwrap().version.as_deref(), Some("9.0.114 [2J"));
    }

    #[test]
    fn a_huge_version_is_not_read() {
        let big = vec![b'9'; VERSION_MAX + 1];
        let z = zip(&[("version", &big), ("db.gz", &gz(BSON))], false);
        assert!(open(z).err().unwrap().contains("version"));
        let t = tar(&[("network/version", &big), ("network/db.gz", &gz(BSON))]);
        assert!(open(unifi(gz(&t))).err().unwrap().contains("version"));
    }
}
