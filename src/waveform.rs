//! The seek bar's waveform: the loudness of a file's audio along its length,
//! as `PEAKS` levels. A hidden libVLC player transcodes the audio to a small
//! WAV (`Vlc::extract_audio`, about 100× real time); this reduces it and
//! caches the levels in the cache folder, keyed by the file's path, size and
//! modification time.

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Levels per file; the seek bar resamples them to its bar count.
pub const PEAKS: usize = 1024;

/// Where the levels for `file` are (or will be) cached, and the scratch WAV
/// next to them. `None` when there is no cache folder or the file is gone.
pub fn cache_paths(file: &Path) -> Option<(PathBuf, PathBuf)> {
    let dirs = directories::ProjectDirs::from("io.github", "nobledoodle", "ferricone")?;
    let meta = fs::metadata(file).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    let path = file.canonicalize().ok()?;
    let mut key = fnv1a(path.as_os_str().as_encoded_bytes());
    key = fnv1a_more(key, &meta.len().to_le_bytes());
    key = fnv1a_more(key, &modified.as_nanos().to_le_bytes());
    let dir = dirs.cache_dir().join("waveforms");
    Some((
        dir.join(format!("{key:016x}.peaks")),
        dir.join(format!("{key:016x}.wav")),
    ))
}

/// Cached levels, if they are there and whole.
pub fn load(cache: &Path) -> Option<Vec<u8>> {
    fs::read(cache).ok().filter(|levels| levels.len() == PEAKS)
}

/// Reduces the extracted WAV to levels, caches them and deletes the WAV.
pub fn finish(wav: &Path, cache: &Path) -> Option<Vec<u8>> {
    let levels = fs::read(wav).ok().and_then(|bytes| levels(&bytes));
    let _ = fs::remove_file(wav);
    let levels = levels?;
    if let Some(dir) = cache.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(cache, &levels) {
        eprintln!("ferricone: caching the waveform: {e}");
    }
    Some(levels)
}

/// The peak of each of `PEAKS` equal stretches of 16-bit mono WAV audio,
/// scaled so the loudest is 255.
pub fn levels(wav: &[u8]) -> Option<Vec<u8>> {
    let samples = data_chunk(wav)?;
    let count = samples.len() / 2;
    if count == 0 {
        return None;
    }
    let sample = |i: usize| i16::from_le_bytes([samples[2 * i], samples[2 * i + 1]]).unsigned_abs();
    let peaks: Vec<u16> = (0..PEAKS)
        .map(|bucket| {
            let (start, end) = (
                bucket * count / PEAKS,
                ((bucket + 1) * count / PEAKS).max(bucket * count / PEAKS + 1),
            );
            (start..end.min(count)).map(sample).max().unwrap_or(0)
        })
        .collect();
    let loudest = u32::from(*peaks.iter().max()?).max(1);
    Some(
        peaks
            .into_iter()
            .map(|p| (u32::from(p) * 255 / loudest) as u8)
            .collect(),
    )
}

/// The samples of a RIFF/WAVE file's `data` chunk. A size left at 0 or
/// past the end (a stream that never went back to write it) reads to the end.
fn data_chunk(wav: &[u8]) -> Option<&[u8]> {
    if wav.get(..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    while let Some(header) = wav.get(at..at + 8) {
        let size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        let body = at + 8;
        if &header[..4] == b"data" {
            let end = if size == 0 {
                wav.len()
            } else {
                (body + size).min(wav.len())
            };
            return wav.get(body..end);
        }
        at = body + size + size % 2;
    }
    None
}

fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_more(0xcbf2_9ce4_8422_2325, bytes)
}

/// FNV-1a: stable across Rust versions, unlike the standard hasher.
fn fnv1a_more(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend(b"fmt \x10\0\0\0");
        out.extend([1, 0, 1, 0, 0xa0, 0x0f, 0, 0, 0x40, 0x1f, 0, 0, 2, 0, 16, 0]);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn scales_peaks_to_the_loudest() {
        // Quiet first half, loud second half.
        let samples: Vec<i16> = (0..PEAKS * 4)
            .map(|i| if i < PEAKS * 2 { 100 } else { -20000 })
            .collect();
        let levels = levels(&wav(&samples)).unwrap();
        assert_eq!(levels.len(), PEAKS);
        assert_eq!(levels[0], 1);
        assert_eq!(levels[PEAKS - 1], 255);
    }

    #[test]
    fn reads_short_and_unsized_data() {
        let short = levels(&wav(&[1000, -2000, 500])).unwrap();
        assert_eq!(short.len(), PEAKS);
        assert_eq!(*short.iter().max().unwrap(), 255);
        let mut no_size = wav(&[300; 64]);
        let at = no_size.len() - 128 - 4;
        no_size[at..at + 4].copy_from_slice(&[0; 4]);
        assert!(levels(&no_size).is_some());
        assert!(levels(b"not a wav").is_none());
    }
}
