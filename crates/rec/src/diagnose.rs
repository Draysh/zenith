//! What this machine can do: the short answer the daemon asks for
//! (`--info`), and the long one a person sends in when recording does not
//! work (`--diagnose`), which tries everything for a moment and writes
//! down what happened.

use crate::args::Codec;
use crate::audio::{self, Tap};
use crate::ffmpeg::{self, Abilities, Encoder, Family, Frames, Grab, Video};
use crate::{monitors, sys};
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// Whether each encoder works, all tried at once.
fn try_encoders(ffmpeg: &Path, list: Vec<Encoder>) -> Vec<(Encoder, Result<(), String>)> {
    std::thread::scope(|scope| {
        let tries: Vec<_> = list.into_iter().map(|e| (e, scope.spawn(move || ffmpeg::run_quietly(ffmpeg, &ffmpeg::encoder_test_args(e), ffmpeg::PROBE_LIMIT)))).collect();
        tries.into_iter().map(|(e, t)| (e, t.join().unwrap_or_else(|_| Err("the test itself failed".into())))).collect()
    })
}

/// The codecs to offer, from which encoders work: a codec with a hardware
/// encoder, and H.264 also when only the software one is there.
pub fn offered(results: &[(Encoder, bool)]) -> Vec<Codec> {
    Codec::ALL
        .into_iter()
        .filter(|codec| {
            results.iter().any(|(e, works)| *works && e.codec == *codec && (e.family != Family::Software || *codec == Codec::H264))
        })
        .collect()
}

/// The text of `--info`.
pub fn info() -> String {
    let ffmpeg = ffmpeg::locate();
    let has = ffmpeg::abilities(&ffmpeg);
    let known: Vec<Encoder> = Codec::ALL
        .into_iter()
        .flat_map(ffmpeg::encoders)
        .filter(|e| has.encoders.contains(e.name))
        // A test recording only ever uses the software encoder.
        .filter(|e| !monitors::test_source() || e.name == "libx264")
        .collect();
    let results: Vec<(Encoder, bool)> = try_encoders(&ffmpeg, known).into_iter().map(|(e, r)| (e, r.is_ok())).collect();
    let mut text = String::from("section=video_codecs\n");
    for codec in offered(&results) {
        let _ = writeln!(text, "{}", codec.as_str());
    }
    let app_audio = if sys::build_has_app_audio(sys::windows_build()) { "yes" } else { "no" };
    let _ = write!(text, "section=system_info\nsupports_app_audio|{app_audio}\n");
    text
}

fn verdict(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "works".into(),
        Err(err) => format!("FAILS: {err}"),
    }
}

/// Listen to `tap` for a moment: how much came, and how loud.
fn listen(tap: &Tap) -> String {
    let mut opener = audio::opener(0);
    let mut stream = match opener.open(tap) {
        Ok(stream) => stream,
        Err(err) => return format!("FAILS to open: {err}"),
    };
    let (mut samples, began) = (Vec::new(), Instant::now());
    while began.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(10));
        if let Err(err) = stream.read(&mut samples) {
            return format!("opened, then FAILED: {err}");
        }
    }
    let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
    format!("opened; {} frames in 1.5 s (72000 when sound is playing), peak level {peak:.3}", samples.len() / 2)
}

fn video(grab: Grab, frames: Frames, encoder: Encoder) -> Video {
    Video { grab, frames, encoder, fps: 60, cursor: true, scale: None, bitrate_kbps: 12000 }
}

fn report(ffmpeg: &Path, has: &Abilities) -> String {
    let mut text = String::new();
    let mut say = |line: String| {
        crate::log::stderr(&line);
        let _ = writeln!(text, "{line}");
    };
    say(format!("zenith-rec {} diagnosis, {}", env!("CARGO_PKG_VERSION"), chrono::Local::now().format("%Y-%m-%d %H:%M:%S")));
    say(format!("system: {}", sys::os_description()));
    say(format!("ffmpeg: {} ({})", ffmpeg.display(), if has.version.is_empty() { "CANNOT BE RUN" } else { &has.version }));
    say(format!("capture filters in this ffmpeg: ddagrab {}, gfxcapture {}", has.filters.contains("ddagrab"), has.filters.contains("gfxcapture")));

    say("\n== monitors ==".into());
    let all = monitors::list();
    for m in &all {
        say(format!("{} {}x{} primary={} ddagrab output={:?} handle={}", m.name, m.size.0, m.size.1, m.primary, m.output_idx, m.handle));
    }
    if all.is_empty() {
        say("none found".into());
    }

    say("\n== encoders (a second of a generated picture) ==".into());
    let known: Vec<Encoder> = Codec::ALL.into_iter().flat_map(ffmpeg::encoders).collect();
    let (built, missing): (Vec<Encoder>, Vec<Encoder>) = known.into_iter().partition(|e| has.encoders.contains(e.name));
    for e in missing {
        say(format!("{}: not in this ffmpeg", e.name));
    }
    let results = try_encoders(ffmpeg, built);
    for (e, result) in &results {
        say(format!("{}: {}", e.name, verdict(result)));
    }
    let working: Vec<Encoder> = results.iter().filter(|(_, r)| r.is_ok()).map(|(e, _)| *e).collect();

    say("\n== capture (a second of each monitor, each way) ==".into());
    let software = working.iter().copied().find(|e| e.name == "libx264");
    for m in all.iter().filter(|_| !monitors::test_source()) {
        for grab in monitors::grabs(m, &has.filters, None) {
            // First with the plainest encoder, so a failure is the capture's.
            if let Some(encoder) = software {
                let v = video(grab.clone(), Frames::Memory, encoder);
                say(format!("{} {}: {}", m.name, v.describe(), verdict(&ffmpeg::run_quietly(ffmpeg, &ffmpeg::probe_args(&v), ffmpeg::PROBE_LIMIT))));
            }
            for encoder in working.iter().copied().filter(|e| e.family != Family::Software && e.codec == Codec::H264) {
                for frames in [Frames::Gpu, Frames::Memory] {
                    let v = video(grab.clone(), frames, encoder);
                    say(format!("{} {}: {}", m.name, v.describe(), verdict(&ffmpeg::run_quietly(ffmpeg, &ffmpeg::probe_args(&v), ffmpeg::PROBE_LIMIT))));
                }
            }
        }
    }
    if monitors::test_source()
        && let Some(encoder) = software
    {
        let v = video(Grab::Test { size: (1280, 720) }, Frames::Memory, encoder);
        say(format!("{}: {}", v.describe(), verdict(&ffmpeg::run_quietly(ffmpeg, &ffmpeg::probe_args(&v), ffmpeg::PROBE_LIMIT))));
    }

    say("\n== sound (1.5 s of each) ==".into());
    for (name, label) in audio::devices() {
        say(format!("device {name} | {label}"));
    }
    say(format!("default speakers: {}", listen(&Tap::Output(None))));
    say(format!("default microphone: {}", listen(&Tap::Input(None))));
    say(format!("one program's sound is supported by this Windows: {}", sys::build_has_app_audio(sys::windows_build())));
    let me = std::process::id();
    say(format!("only this program's sound (should open, and be silent): {}", listen(&Tap::Tree { pid: me, include: true })));
    say(format!("everything but this program's sound: {}", listen(&Tap::Tree { pid: me, include: false })));
    text
}

/// Try everything, write the report beside the log, and print where it is.
pub fn run() -> ExitCode {
    let ffmpeg = ffmpeg::locate();
    let has = ffmpeg::abilities(&ffmpeg);
    let text = report(&ffmpeg, &has);
    let dir = sys::logs_dir();
    let path = dir.join(format!("zenith-rec-diagnose-{}.txt", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")));
    if let Err(err) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, text)) {
        crate::log::stderr(&format!("zenith-rec: the report could not be written to {}: {err}", path.display()));
        return ExitCode::FAILURE;
    }
    println!("{}", std::path::absolute(&path).unwrap_or(path).display());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, works: bool) -> (Encoder, bool) {
        let encoder = Codec::ALL.into_iter().flat_map(ffmpeg::encoders).find(|e| e.name == name).unwrap();
        (encoder, works)
    }

    #[test]
    fn a_codec_is_offered_when_the_graphics_card_encodes_it() {
        let results = [result("h264_nvenc", true), result("hevc_nvenc", true), result("av1_nvenc", false), result("libx264", true)];
        assert_eq!(offered(&results), [Codec::H264, Codec::Hevc]);
    }

    #[test]
    fn without_a_hardware_encoder_there_is_still_h264() {
        let results = [result("h264_amf", false), result("libx264", true), result("libx265", true), result("libsvtav1", true)];
        assert_eq!(offered(&results), [Codec::H264], "software H.265 and AV1 are too slow to offer");
        assert!(offered(&[result("libx264", false)]).is_empty());
    }

    #[test]
    fn info_has_the_two_sections_the_app_reads() {
        if !monitors::test_source() || ffmpeg::abilities(&ffmpeg::locate()).version.is_empty() {
            return;
        }
        let text = info();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"section=video_codecs"));
        assert_eq!(lines[lines.len() - 2..], ["section=system_info", "supports_app_audio|no"]);
    }
}
