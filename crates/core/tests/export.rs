//! Exports run for real, through ffmpeg. Skipped where ffmpeg is not installed.

use zenith_core::edit::Edit;
use zenith_core::export::ffmpeg::{ExportError, Job, Picture, Sound, run};
use zenith_core::export::{Preset, Source, plan};
use zenith_core::library::probe;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// A 12 second 720p60 clip with moving noise (hard to compress) and three
/// audio tracks, as the recorder would write it.
fn sample(dir: &Path) -> std::path::PathBuf {
    let out = dir.join("sample.mp4");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=60,noise=alls=40:allf=t"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440", "-f", "lavfi", "-i", "sine=frequency=660", "-f", "lavfi", "-i", "sine=frequency=880"])
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:a", "-t", "12"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-b:v", "8M", "-g", "60", "-c:a", "aac"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    out
}

#[test]
fn an_edited_clip_is_exported_under_the_limit() {
    if !have_ffmpeg() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = sample(dir.path());
    let meta = probe(&input).unwrap();
    assert_eq!((meta.width, meta.height, meta.audio_tracks), (1280, 720, 3));

    // Keep 0-3 s and 6-12 s, mute the third track, halve the second.
    let mut edit = Edit::new(meta.seconds, 3);
    edit.split(3.0);
    edit.split(6.0);
    edit.remove(1);
    edit.set_muted(2, true);
    edit.set_volume(1, 0.5);
    let kept = edit.kept();
    assert!((kept - 9.0).abs() < 0.2, "{kept}");

    let preset = Preset::Custom { megabytes: 1 };
    let source = Source { width: meta.width, height: meta.height, fps: meta.fps, seconds: kept, has_audio: edit.audible() };
    let p = plan(source, preset).unwrap();
    let output = dir.path().join("out/for discord.mp4");
    let job = Job { input: input.clone(), output: output.clone(), edit: edit.clone(), picture: Picture::Fit(p), sound: Sound::Mixed };
    let mut seen = Vec::new();
    let bytes = run(&job, Some(preset.max_bytes()), &AtomicBool::new(false), |f| seen.push(f)).unwrap();

    assert!(bytes <= preset.max_bytes(), "{bytes} bytes for {p:?}");
    assert!(bytes > preset.max_bytes() / 2, "the budget is used: {bytes}");
    let result = probe(&output).unwrap();
    assert!((result.seconds - kept).abs() < 0.3, "{} for {kept}", result.seconds);
    assert_eq!(result.audio_tracks, 1);
    assert_eq!((result.width, result.height), (p.width, p.height));
    assert_eq!(result.codec, "h264");
    assert_eq!(seen.last(), Some(&1.0));
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "progress only moves forward");
    // Nothing is left behind beside the result.
    let left: Vec<_> = std::fs::read_dir(output.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["for discord.mp4"]);

    // The same edit at full quality keeps the tracks apart.
    let full = dir.path().join("out/full.mp4");
    let job = Job { input, output: full.clone(), edit, picture: Picture::Full, sound: Sound::Tracks };
    run(&job, None, &AtomicBool::new(false), |_| {}).unwrap();
    let result = probe(&full).unwrap();
    assert_eq!((result.audio_tracks, result.width), (3, 1280));
    assert!((result.seconds - kept).abs() < 0.3);
}

#[test]
fn sound_moved_against_the_picture_exports_at_the_same_length() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = sample(dir.path());
    let meta = probe(&input).unwrap();
    // Later runs into silence at the start, earlier past the end of the file.
    for (offset, sound, tracks) in [(400, Sound::Mixed, 1), (-400, Sound::Tracks, 3)] {
        let mut edit = Edit::new(meta.seconds, 3);
        edit.split(3.0);
        edit.split(6.0);
        edit.remove(1);
        edit.set_audio_offset(offset);
        let output = dir.path().join(format!("moved {offset}.mp4"));
        let job = Job { input: input.clone(), output: output.clone(), edit: edit.clone(), picture: Picture::Full, sound };
        run(&job, None, &AtomicBool::new(false), |_| {}).unwrap();
        let result = probe(&output).unwrap();
        assert_eq!(result.audio_tracks, tracks, "moved by {offset}");
        assert!((result.seconds - edit.kept()).abs() < 0.5, "{} for {} moved by {offset}", result.seconds, edit.kept());
    }
}

#[test]
fn a_cancelled_or_failed_export_leaves_nothing() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.mp4");
    let job = |input: &Path| Job { input: input.into(), output: output.clone(), edit: Edit::new(5.0, 0), picture: Picture::Full, sound: Sound::Mixed };
    let err = run(&job(&dir.path().join("missing.mp4")), None, &AtomicBool::new(false), |_| {}).unwrap_err();
    assert!(matches!(err, ExportError::Failed(_)), "{err}");
    let input = sample(dir.path());
    let err = run(&job(&input), None, &AtomicBool::new(true), |_| {}).unwrap_err();
    assert!(matches!(err, ExportError::Cancelled), "{err}");
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["sample.mp4"]);
}

#[test]
fn a_clip_gets_its_pictures_and_waveforms() {
    use zenith_core::media::Cache;
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = sample(dir.path());
    let cache = Cache::new(dir.path().join("cache"));

    let thumb = cache.thumbnail(&input, 12.0).unwrap();
    let meta = probe_image(&thumb);
    assert_eq!(meta, (640, 360));
    assert_eq!(cache.thumbnail(&input, 12.0).unwrap(), thumb, "the second time it is the cached file");

    let strip = cache.filmstrip(&input, 12.0, 8).unwrap();
    assert_eq!(probe_image(&strip), (160 * 8, 90));

    let wave = cache.waveform(&input, 1, 50).unwrap();
    assert_eq!(wave.len(), 50);
    assert!(wave.iter().all(|p| (0.05..=1.0).contains(p)), "a steady tone: {wave:?}");
    assert_eq!(cache.waveform(&input, 1, 50).unwrap(), wave);
    assert!(cache.waveform(&input, 9, 50).is_err(), "there is no tenth track");
    assert!(cache.thumbnail(&dir.path().join("missing.mp4"), 1.0).is_err());
}

fn probe_image(file: &Path) -> (u32, u32) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text.trim().split_once(',').unwrap();
    (w.parse().unwrap(), h.parse().unwrap())
}
