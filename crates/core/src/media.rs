//! Pictures and sound shapes of a clip, made with ffmpeg and kept in the
//! cache: the thumbnail on a card, the strip of frames along the timeline,
//! and each audio track's waveform. Everything here is made again when the
//! clip's file changes, and can be deleted at any time.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Put before a file handed to ffmpeg or ffprobe to look at: what it reads
/// is that file and nothing a file can point to. A clip is a file on this
/// computer; one that names an address on the network, or another file,
/// is not followed there. (Every video dropped into the clip folder is
/// looked at without anyone asking.)
pub const FILES_ONLY: [&str; 2] = ["-protocol_whitelist", "file"];

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("ffmpeg could not be started: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("ffmpeg failed: {0}")]
    Failed(String),
}

/// A name for what is cached about a file: it changes when the file's
/// path, size or modification time does.
pub fn cache_key(file: &Path) -> String {
    let meta = std::fs::metadata(file).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let modified = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    // FNV-1a: the same on every machine and every build.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in file.as_os_str().as_encoded_bytes().iter().copied().chain(size.to_le_bytes()).chain(modified.to_le_bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn ffmpeg(args: &[&str], input: &Path, tail: &[&str], out: &Path) -> Result<(), MediaError> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written under another name and renamed: a reader never sees half a file.
    // The name is this process's and this call's own: two makers of the same
    // picture never write the same file.
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("tmp");
    let partial = out.with_extension(format!("{}-{call}.part.{ext}", std::process::id()));
    let done = crate::paths::command("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error"])
        .args(args)
        .args(FILES_ONLY)
        .arg("-i")
        .arg(input)
        .args(tail)
        .arg(&partial)
        .stdin(Stdio::null())
        .output()?;
    if !done.status.success() || !partial.is_file() {
        let _ = std::fs::remove_file(&partial);
        let text = String::from_utf8_lossy(&done.stderr);
        return Err(MediaError::Failed(text.lines().last().unwrap_or("no message").trim().to_string()));
    }
    std::fs::rename(&partial, out)?;
    Ok(())
}

/// The frame at `at` seconds, `width` pixels wide, as a JPEG at `out`.
pub fn thumbnail(file: &Path, at: f64, width: u32, out: &Path) -> Result<(), MediaError> {
    let at = format!("{:.3}", at.max(0.0));
    let scale = format!("scale={width}:-2:flags=bilinear");
    ffmpeg(&["-ss", &at], file, &["-frames:v", "1", "-vf", &scale, "-q:v", "4", "-f", "mjpeg"], out)
}

/// `count` frames spread over a clip of `seconds`, side by side in one JPEG,
/// each `tile_width` wide. Only keyframes are decoded, so this is quick.
pub fn filmstrip(file: &Path, seconds: f64, count: u32, tile_width: u32, out: &Path) -> Result<(), MediaError> {
    let count = count.clamp(1, 120);
    let rate = format!("{:.6}", count as f64 / seconds.max(0.1));
    let filter = format!("fps={rate},scale={tile_width}:-2:flags=bilinear,tile={count}x1");
    ffmpeg(&["-skip_frame", "nokey"], file, &["-frames:v", "1", "-vf", &filter, "-q:v", "5", "-f", "mjpeg"], out)
}

/// The loudest moment in each of `buckets` equal stretches of samples, 0 to 1.
pub fn peaks(samples: &[i16], buckets: usize) -> Vec<f32> {
    if buckets == 0 || samples.is_empty() {
        return vec![0.0; buckets];
    }
    (0..buckets)
        .map(|b| {
            let start = b * samples.len() / buckets;
            let end = ((b + 1) * samples.len() / buckets).max(start + 1).min(samples.len());
            let peak = samples[start..end].iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            peak as f32 / 32768.0
        })
        .collect()
}

/// The shape of each of a file's first `tracks` audio tracks in `buckets`
/// steps, read in one pass over the file. `scratch` is a folder the sound
/// is written to on the way.
pub fn waveforms(file: &Path, tracks: u32, buckets: usize, scratch: &Path) -> Result<Vec<Vec<f32>>, MediaError> {
    std::fs::create_dir_all(scratch)?;
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let raw: Vec<PathBuf> = (0..tracks).map(|t| scratch.join(format!("{}-{call}-{t}.part.raw", std::process::id()))).collect();
    let mut cmd = crate::paths::command("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error"]).args(FILES_ONLY).arg("-i").arg(file);
    for (track, out) in raw.iter().enumerate() {
        cmd.args(["-map", &format!("0:a:{track}"), "-ac", "1", "-ar", "4000", "-f", "s16le"]).arg(out);
    }
    let done = cmd.stdin(Stdio::null()).output();
    let read = |out: &PathBuf| {
        let bytes = std::fs::read(out)?;
        let samples: Vec<i16> = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        Ok::<_, MediaError>(peaks(&samples, buckets))
    };
    let waves = match done {
        Ok(done) if done.status.success() => raw.iter().map(read).collect(),
        Ok(done) => {
            let text = String::from_utf8_lossy(&done.stderr);
            Err(MediaError::Failed(text.lines().last().unwrap_or("no message").trim().to_string()))
        }
        Err(e) => Err(MediaError::Spawn(e)),
    };
    for out in &raw {
        let _ = std::fs::remove_file(out);
    }
    waves
}

/// The shape of audio track `track` (0 is the first) in `buckets` steps.
pub fn waveform(file: &Path, track: u32, buckets: usize) -> Result<Vec<f32>, MediaError> {
    let map = format!("0:a:{track}");
    let done = crate::paths::command("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(FILES_ONLY)
        .arg("-i")
        .arg(file)
        .args(["-map", &map, "-ac", "1", "-ar", "4000", "-f", "s16le", "-"])
        .stdin(Stdio::null())
        .output()?;
    if !done.status.success() {
        let text = String::from_utf8_lossy(&done.stderr);
        return Err(MediaError::Failed(text.lines().last().unwrap_or("no message").trim().to_string()));
    }
    let samples: Vec<i16> = done.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    Ok(peaks(&samples, buckets))
}

/// Where a clip's cached pictures and waveforms are kept.
pub struct Cache {
    dir: PathBuf,
}

/// The folders of the cache: thumbnails, filmstrips, waveforms.
const KINDS: [&str; 3] = ["thumbs", "strips", "waves"];

/// The most the cache holds after it is tidied. What is kept is named after
/// its clip's path, size and time, so every rename, move and new export of
/// a clip leaves a set behind that nothing asks for again; the oldest go
/// first, and one that was still wanted is made again when it is.
pub const CACHE_BYTES: u64 = 512 * 1024 * 1024;

/// How often a running app tidies the cache, at most.
const TIDY_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// A half-made file this old was left by a Zenith that is not running any
/// more: no picture takes this long to make.
const LEFT_BEHIND: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Held by whoever is making a file's waveforms (see `Cache::waveform`).
struct Making<'a>(&'a Path);

static MAKING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
static MADE: std::sync::Condvar = std::sync::Condvar::new();

impl<'a> Making<'a> {
    /// Wait until nobody else is making this file's waveforms, then be
    /// the one who is.
    fn of(file: &'a Path) -> Self {
        let mut making = MAKING.lock().unwrap_or_else(|e| e.into_inner());
        while making.iter().any(|other| other == file) {
            making = MADE.wait(making).unwrap_or_else(|e| e.into_inner());
        }
        making.push(file.to_path_buf());
        Self(file)
    }
}

impl Drop for Making<'_> {
    fn drop(&mut self) {
        MAKING.lock().unwrap_or_else(|e| e.into_inner()).retain(|other| other != self.0);
        MADE.notify_all();
    }
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn thumbnail_path(&self, file: &Path) -> PathBuf {
        self.dir.join("thumbs").join(format!("{}.jpg", cache_key(file)))
    }

    pub fn filmstrip_path(&self, file: &Path, count: u32) -> PathBuf {
        self.dir.join("strips").join(format!("{}-{count}.jpg", cache_key(file)))
    }

    fn waveform_path(&self, file: &Path, track: u32, buckets: usize) -> PathBuf {
        self.dir.join("waves").join(format!("{}-{track}-{buckets}.json", cache_key(file)))
    }

    /// The card's picture: a frame a third of the way in, where a clip is
    /// more telling than at its first frame.
    pub fn thumbnail(&self, file: &Path, seconds: f64) -> Result<PathBuf, MediaError> {
        let out = self.thumbnail_path(file);
        if !out.is_file() {
            thumbnail(file, seconds / 3.0, 640, &out)?;
            self.tidy_now_and_then();
        }
        Ok(out)
    }

    pub fn filmstrip(&self, file: &Path, seconds: f64, count: u32) -> Result<PathBuf, MediaError> {
        let out = self.filmstrip_path(file, count);
        if !out.is_file() {
            filmstrip(file, seconds, count, 160, &out)?;
            self.tidy_now_and_then();
        }
        Ok(out)
    }

    fn cached_waveform(&self, file: &Path, track: u32, buckets: usize) -> Option<Vec<f32>> {
        let path = self.waveform_path(file, track, buckets);
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Vec<f32>>(&b).ok())
    }

    /// The shape of one audio track. The first one asked for of a file
    /// makes every track's, in a single pass over the file, so the others
    /// are there when they are asked for.
    pub fn waveform(&self, file: &Path, track: u32, buckets: usize) -> Result<Vec<f32>, MediaError> {
        if let Some(cached) = self.cached_waveform(file, track, buckets) {
            return Ok(cached);
        }
        // One at a time for each file: whoever asks for the next track of
        // the same file meanwhile waits here, then finds it made. Another
        // file's waveforms are not kept waiting behind this one's.
        let _making = Making::of(file);
        if let Some(cached) = self.cached_waveform(file, track, buckets) {
            return Ok(cached);
        }
        let tracks = crate::library::probe(file).map(|m| m.audio_tracks).unwrap_or(0);
        if track >= tracks {
            // Not a track the file is known to have: ask ffmpeg for it alone.
            return waveform(file, track, buckets);
        }
        let waves = waveforms(file, tracks, buckets, &self.dir.join("waves"))?;
        for (t, wave) in waves.iter().enumerate() {
            if let Ok(json) = serde_json::to_vec(wave) {
                let _ = std::fs::write(self.waveform_path(file, t as u32, buckets), json);
            }
        }
        Ok(waves.into_iter().nth(track as usize).unwrap_or_default())
    }

    /// Tidy the cache when something new was put in it, the first time in
    /// a run of the app and then every few hours: often enough to keep it
    /// in bounds, and never in the way of a screen that is waiting.
    fn tidy_now_and_then(&self) {
        static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
        {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|at| at.elapsed() < TIDY_EVERY) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        self.tidy(CACHE_BYTES);
    }

    /// Bring the cache under `most` bytes by taking away what was made
    /// longest ago, and take away half-made files a Zenith that fell over
    /// left behind. Returns how many bytes were freed. Anything taken away
    /// that is still wanted is made again when it is asked for.
    pub fn tidy(&self, most: u64) -> u64 {
        let now = std::time::SystemTime::now();
        let mut freed = 0;
        let mut kept: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
        for kind in KINDS {
            let Ok(entries) = std::fs::read_dir(self.dir.join(kind)) else { continue };
            for entry in entries.flatten() {
                let Ok(meta) = entry.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let made = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                if entry.file_name().to_string_lossy().contains(".part.") {
                    // Someone may be writing it at this moment.
                    if now.duration_since(made).is_ok_and(|age| age > LEFT_BEHIND) && std::fs::remove_file(entry.path()).is_ok() {
                        freed += meta.len();
                    }
                    continue;
                }
                kept.push((made, meta.len(), entry.path()));
            }
        }
        let mut total: u64 = kept.iter().map(|(_, bytes, _)| bytes).sum();
        if total <= most {
            return freed;
        }
        // Down to a little under the most, so the next few pictures do not
        // each call for another tidying.
        let enough = most / 10 * 9;
        kept.sort();
        for (_, bytes, path) in kept {
            if total <= enough {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                freed += bytes;
            }
            // Counted as gone either way: one that cannot be removed is
            // not a reason to remove everything after it.
            total -= bytes;
        }
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_find_the_loudest_sample_of_each_stretch() {
        let samples = [0, 100, -16384, 3, 32767, 0, 0, -32768];
        assert_eq!(peaks(&samples, 4), [100.0 / 32768.0, 0.5, 32767.0 / 32768.0, 1.0]);
        assert_eq!(peaks(&samples, 1), [1.0]);
        assert_eq!(peaks(&[], 3), [0.0, 0.0, 0.0]);
        assert_eq!(peaks(&[1, 2], 4).len(), 4, "more buckets than samples still gives every bucket");
    }

    #[test]
    fn a_tidied_cache_keeps_what_was_made_last() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().join("cache"));
        assert_eq!(cache.tidy(1000), 0, "no cache yet");
        // Ten pictures of 100 bytes, made an hour apart, the first one longest ago.
        let now = std::time::SystemTime::now();
        let hours = |n: u64| now - std::time::Duration::from_secs(n * 3600);
        let put = |name: &str, bytes: usize, made: std::time::SystemTime| {
            let path = dir.path().join("cache").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, vec![0u8; bytes]).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(made).unwrap();
        };
        for n in 0..10 {
            put(&format!("{}/{n}.jpg", KINDS[n % 3]), 100, hours(20 - n as u64));
        }
        // Half-made files: one from a Zenith that fell over yesterday, one being written now.
        put("thumbs/a.1-2.part.jpg", 50, hours(24));
        put("waves/1-2-0.part.raw", 50, now);
        let left = || {
            let mut names: Vec<String> = KINDS
                .iter()
                .flat_map(|kind| std::fs::read_dir(dir.path().join("cache").join(kind)).unwrap())
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        assert_eq!(cache.tidy(1000), 50, "within bounds: only what was left behind goes");
        assert_eq!(left().len(), 11);
        // Over: the oldest go, until it is a little under.
        assert_eq!(cache.tidy(500), 600);
        assert_eq!(left(), ["1-2-0.part.raw", "6.jpg", "7.jpg", "8.jpg", "9.jpg"]);
        assert_eq!(cache.tidy(500), 0, "and then there is nothing more to do");
    }

    #[test]
    fn waveforms_of_one_file_are_made_one_at_a_time_and_of_others_meanwhile() {
        let (a, b) = (PathBuf::from("/c/a.mp4"), PathBuf::from("/c/b.mp4"));
        let making_a = Making::of(&a);
        // Another file is not kept waiting.
        drop(Making::of(&b));
        // The same file is, until the first is done.
        let (tx, rx) = std::sync::mpsc::channel();
        let second = std::thread::spawn({
            let a = a.clone();
            move || {
                let _making = Making::of(&a);
                tx.send(()).unwrap();
            }
        });
        assert!(rx.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "it waits");
        drop(making_a);
        assert!(rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "and then goes on");
        second.join().unwrap();
    }

    #[test]
    fn the_cache_key_follows_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.mp4");
        std::fs::write(&a, b"one").unwrap();
        let first = cache_key(&a);
        assert_eq!(first, cache_key(&a));
        assert_eq!(first.len(), 16);
        std::fs::write(&a, b"other").unwrap();
        assert_ne!(first, cache_key(&a), "another size is another key");
        assert_ne!(cache_key(&a), cache_key(&dir.path().join("b.mp4")));
    }
}
