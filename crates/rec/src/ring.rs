//! The replay buffer on disk: one-second pieces, numbered as ffmpeg writes
//! them. Which pieces to delete and which make up a clip is decided here
//! from their numbers alone.

use std::path::Path;

/// How long ffmpeg makes each piece.
pub const PIECE_SECS: u32 = 1;
/// Pieces kept beyond the buffer: the one being written, and room for a
/// save to wait for it without the oldest piece being deleted meanwhile.
const SPARE: usize = 3;

/// What the pieces are made of. MPEG-TS needs no finishing, so a piece is
/// whole even if ffmpeg is killed while writing it; it cannot hold AV1,
/// which goes into fragmented MP4, as safe to cut off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    MpegTs,
    FragmentedMp4,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Self::MpegTs => "ts",
            Self::FragmentedMp4 => "mp4",
        }
    }
}

/// The file name ffmpeg is told to number: `piece_%09d.ts`.
pub fn pattern(container: Container) -> String {
    format!("piece_%09d.{}", container.extension())
}

pub fn piece_name(index: u64, container: Container) -> String {
    format!("piece_{index:09}.{}", container.extension())
}

/// The number in a piece's file name, whichever container it is.
pub fn parse_index(name: &str) -> Option<u64> {
    let (stem, ext) = name.strip_prefix("piece_")?.split_once('.')?;
    (matches!(ext, "ts" | "mp4") && !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())).then(|| stem.parse().ok())?
}

/// The pieces to delete: all but the newest `buffer_secs` and a few spare.
/// `indices` are in rising order.
pub fn expired(indices: &[u64], buffer_secs: u32) -> &[u64] {
    let keep = (buffer_secs / PIECE_SECS) as usize + SPARE;
    &indices[..indices.len().saturating_sub(keep)]
}

/// The pieces of a clip: those up to and including `last`, reaching back
/// `secs` seconds. `indices` are in rising order.
pub fn covering(indices: &[u64], last: u64, secs: u32) -> &[u64] {
    let end = indices.partition_point(|&i| i <= last);
    let want = secs.div_ceil(PIECE_SECS) as usize;
    &indices[end.saturating_sub(want)..end]
}

/// The number the next ffmpeg starts at, so that pieces written before a
/// restart stay in front of those written after it.
pub fn next_start(indices: &[u64]) -> u64 {
    indices.last().map_or(0, |last| last + 1)
}

/// The numbers of the pieces in `dir`, in rising order.
pub fn indices(dir: &Path) -> Vec<u64> {
    let mut found: Vec<u64> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| parse_index(&entry.file_name().to_string_lossy()))
        .collect();
    found.sort_unstable();
    found
}

/// A ring nobody has written to for this long belongs to no running recorder.
pub const FORGOTTEN_AFTER: std::time::Duration = std::time::Duration::from_secs(120);

/// The rings in `rings` other than `mine` that no recorder is writing to
/// any more: a running one adds a piece every second, so one untouched
/// for two minutes was left behind when its recorder was ended.
pub fn forgotten(rings: &Path, mine: &Path, now: std::time::SystemTime) -> Vec<std::path::PathBuf> {
    let last_written = |dir: &Path| {
        let inside = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()?.modified().ok());
        inside.chain(std::fs::metadata(dir).ok().and_then(|m| m.modified().ok())).max()
    };
    std::fs::read_dir(rings)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir() && dir != mine)
        .filter(|dir| last_written(dir).is_some_and(|at| now.duration_since(at).is_ok_and(|idle| idle > FORGOTTEN_AFTER)))
        .collect()
}

/// Delete every piece (and joining list) in `dir`: what a run before this
/// one left behind is not part of this run's buffer.
pub fn clear(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if parse_index(&name).is_some() || name == LIST_NAME {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The name of the list ffmpeg joins a clip from, kept beside the pieces.
pub const LIST_NAME: &str = "clip.txt";

/// The list of pieces for ffmpeg's concat reader. The names are relative to
/// the list itself, so nothing about the folder's own name (a user name
/// with an apostrophe in it, say) has to be quoted.
pub fn concat_list(pieces: &[u64], container: Container) -> String {
    pieces.iter().map(|&i| format!("file '{}'\n", piece_name(i, container))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_names_go_both_ways() {
        assert_eq!(piece_name(42, Container::MpegTs), "piece_000000042.ts");
        assert_eq!(parse_index("piece_000000042.ts"), Some(42));
        assert_eq!(parse_index("piece_1234567890.mp4"), Some(1234567890));
        assert_eq!(pattern(Container::FragmentedMp4), "piece_%09d.mp4");
        for other in ["clip.txt", "piece_.ts", "piece_12.txt", "piece_1x.ts", "Replay_2026.mp4", "piece_12"] {
            assert_eq!(parse_index(other), None, "{other}");
        }
    }

    #[test]
    fn only_pieces_older_than_the_buffer_are_deleted() {
        let all: Vec<u64> = (0..70).collect();
        // A 60 second buffer keeps 60 pieces and the spare ones.
        assert_eq!(expired(&all, 60), (0..7).collect::<Vec<_>>());
        assert_eq!(expired(&all[..63], 60), [] as [u64; 0]);
        assert_eq!(expired(&all[..64], 60), [0]);
        assert_eq!(expired(&[], 60), [] as [u64; 0]);
    }

    #[test]
    fn a_clip_is_the_last_seconds_before_the_piece_being_written() {
        let all: Vec<u64> = (100..170).collect();
        // Piece 169 is still open: the clip ends with 168.
        let clip = covering(&all, 168, 60);
        assert_eq!((clip.first(), clip.last(), clip.len()), (Some(&109), Some(&168), 60));
        // Early on there is less than a buffer's worth.
        assert_eq!(covering(&all[..5], 103, 60), [100, 101, 102, 103]);
        assert_eq!(covering(&all, 99, 60), [] as [u64; 0]);
        assert_eq!(covering(&[], 5, 60), [] as [u64; 0]);
    }

    #[test]
    fn deleting_never_reaches_into_a_clip_being_saved() {
        // Whatever the moment, the pieces of a clip that ends one or two
        // pieces back are all still there.
        for len in 1..200u64 {
            let all: Vec<u64> = (0..len).collect();
            let kept = &all[expired(&all, 30).len()..];
            for back in 1..=2 {
                let Some(last) = len.checked_sub(1 + back) else { continue };
                let clip = covering(&all, last, 30);
                assert!(clip.iter().all(|i| kept.contains(i)), "len {len} back {back}");
            }
        }
    }

    #[test]
    fn numbering_goes_on_after_a_restart() {
        assert_eq!(next_start(&[]), 0);
        assert_eq!(next_start(&[7, 8, 9]), 10);
        // A gap where pieces were deleted changes nothing.
        assert_eq!(covering(&[3, 4, 9, 10], 10, 3), [4, 9, 10]);
    }

    #[test]
    fn the_folder_is_read_and_cleared() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["piece_000000002.ts", "piece_000000010.ts", "piece_000000001.ts", "clip.txt", "keep.me"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert_eq!(indices(dir.path()), [1, 2, 10]);
        clear(dir.path());
        assert!(indices(dir.path()).is_empty());
        assert!(dir.path().join("keep.me").exists(), "only its own files are deleted");
        assert!(!dir.path().join("clip.txt").exists());
        assert!(indices(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn the_joining_list_names_the_pieces_in_order() {
        assert_eq!(concat_list(&[9, 10], Container::MpegTs), "file 'piece_000000009.ts'\nfile 'piece_000000010.ts'\n");
        assert_eq!(concat_list(&[], Container::MpegTs), "");
    }

    #[test]
    fn only_rings_long_untouched_are_cleared_away() {
        let dir = tempfile::tempdir().unwrap();
        let (mine, other) = (dir.path().join("100"), dir.path().join("200"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("piece"), b"x").unwrap();
        let now = std::time::SystemTime::now();
        assert!(forgotten(dir.path(), &mine, now).is_empty(), "one written to just now is someone's");
        let later = now + FORGOTTEN_AFTER + std::time::Duration::from_secs(5);
        assert_eq!(forgotten(dir.path(), &mine, later), [other], "never this run's own");
        assert!(forgotten(&dir.path().join("none"), &mine, later).is_empty());
    }
}
