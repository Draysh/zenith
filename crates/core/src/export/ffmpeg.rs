//! The ffmpeg commands that render an edit, and running them.

use super::plan::Plan;
use crate::edit::Edit;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the sound of the result is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// One track, the audible ones mixed: what plays everywhere.
    Mixed,
    /// Every track kept apart, so the result can be edited again.
    Tracks,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Picture {
    /// Sized and encoded in two passes to land on a file size.
    Fit(Plan),
    /// As recorded, at a quality that shows no loss.
    Full,
}

#[derive(Clone, Debug)]
pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
    pub edit: Edit,
    pub picture: Picture,
    pub sound: Sound,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("ffmpeg could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("ffmpeg failed: {0}")]
    Failed(String),
    #[error("the export was cancelled")]
    Cancelled,
    #[error("the result could not be brought under {limit} bytes (it came out at {actual})")]
    TooBig { actual: u64, limit: u64 },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Put after sound that was made louder, or several tracks added up: what
/// would go over full scale is held under it instead of being cut off.
const LIMITER: &str = ",alimiter=level=0:limit=0.98";

fn limiter(gain: f64) -> &'static str {
    if gain > 1.0 { LIMITER } else { "" }
}

fn secs(t: f64) -> String {
    format!("{t:.3}")
}

/// Where the sound of a kept stretch is read from when the edit moves the
/// sound against the picture: the stretch's own times, earlier by the
/// offset. Returns where in the file to start, how long to read, and how
/// much silence goes before it (sound wanted from before the file begins).
fn shifted(start: f64, len: f64, offset_ms: i32) -> (f64, f64, f64) {
    let from = start - offset_ms as f64 / 1000.0;
    let silence = (-from).clamp(0.0, len);
    (from.max(0.0), len - silence, silence)
}

/// The filter graph for an edit: every kept stretch is an input of its own
/// (`[0]`, `[1]`, …), joined end to end. Where the sound is moved against
/// the picture, each stretch's sound is an input of its own after those
/// (see `inputs`). Returns the graph and the labels of its audio outputs;
/// the picture comes out as `[vo]`.
pub fn graph(edit: &Edit, sound: Option<Sound>, picture: &Picture) -> (String, Vec<String>) {
    let n = edit.segments.len();
    // The input a stretch's sound comes from, and what is done to it first.
    let heard = |i: usize, j: usize| -> String {
        if edit.audio_offset_ms == 0 {
            return format!("[{i}:a:{j}]");
        }
        let s = edit.segments[i];
        let (_, _, silence) = shifted(s.start, s.len(), edit.audio_offset_ms);
        let lead = if silence > 0.0 { format!("adelay={}:all=1,", (silence * 1000.0).round() as u64) } else { String::new() };
        format!("[{}:a:{j}]{lead}", n + i)
    };
    let audible: Vec<usize> = (0..edit.gains.len()).filter(|j| edit.gains[*j].factor() > 0.0).collect();
    let outs = match sound {
        None => 0,
        Some(Sound::Mixed) => usize::from(!audible.is_empty()),
        Some(Sound::Tracks) => edit.gains.len(),
    };
    let mut g = String::new();
    let mut joined = String::new();
    for i in 0..n {
        joined += &format!("[{i}:v]");
        match sound {
            Some(Sound::Mixed) if outs == 1 => {
                if let [j] = audible[..] {
                    let gain = edit.gains[j].factor();
                    g += &format!("{}volume={gain:.3}{}[a{i}];", heard(i, j), limiter(gain));
                } else {
                    for j in &audible {
                        g += &format!("{}volume={:.3}[m{i}_{j}];", heard(i, *j), edit.gains[*j].factor());
                    }
                    let ins: String = audible.iter().map(|j| format!("[m{i}_{j}]")).collect();
                    let sum: f64 = audible.iter().map(|j| edit.gains[*j].factor()).sum();
                    g += &format!("{ins}amix=inputs={}:normalize=0:duration=longest{}[a{i}];", audible.len(), limiter(sum));
                }
                joined += &format!("[a{i}]");
            }
            Some(Sound::Tracks) => {
                for (j, gain) in edit.gains.iter().enumerate().take(outs) {
                    let gain = gain.factor();
                    g += &format!("{}volume={gain:.3}{}[t{i}_{j}];", heard(i, j), limiter(gain));
                    joined += &format!("[t{i}_{j}]");
                }
            }
            _ => {}
        }
    }
    let labels: Vec<String> = (0..outs).map(|j| format!("[ao{j}]")).collect();
    g += &format!("{joined}concat=n={n}:v=1:a={outs}[vc]{};", labels.concat());
    match picture {
        Picture::Fit(p) => g += &format!("[vc]scale={}:{}:flags=lanczos,fps={:.3},format=yuv420p[vo]", p.width, p.height, p.fps),
        // The encoder takes even sides only; a clip made elsewhere may have odd ones.
        Picture::Full => g += "[vc]scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p[vo]",
    }
    (g, labels)
}

/// The file once for every kept stretch; and, with sound that is moved
/// against the picture, once more for each stretch's sound, read from
/// where that sound is in the file.
fn inputs(job: &Job, with_sound: bool) -> Vec<String> {
    let file = job.input.to_string_lossy().into_owned();
    let mut v = Vec::new();
    for s in &job.edit.segments {
        v.extend(["-ss".into(), secs(s.start), "-t".into(), secs(s.len()), "-i".into(), file.clone()]);
    }
    if with_sound && job.edit.audio_offset_ms != 0 {
        for s in &job.edit.segments {
            let (from, len, _) = shifted(s.start, s.len(), job.edit.audio_offset_ms);
            // The picture of these is not used; only their sound is read.
            v.extend(["-vn".into(), "-ss".into(), secs(from), "-t".into(), secs(len.max(0.001)), "-i".into(), file.clone()]);
        }
    }
    v
}

fn head() -> Vec<String> {
    ["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-progress", "pipe:1", "-nostats"].map(String::from).into()
}

/// The ffmpeg command lines for a job, to run one after the other. `passlog`
/// is where the first pass leaves its notes for the second.
pub fn commands(job: &Job, passlog: &Path) -> Vec<Vec<String>> {
    let s = |x: &str| x.to_string();
    let out = job.output.to_string_lossy().into_owned();
    let with_sound = |cmd: &mut Vec<String>, audio_kbps: u32| {
        let (g, labels) = graph(&job.edit, Some(job.sound), &job.picture);
        cmd.extend([s("-filter_complex"), g, s("-map"), s("[vo]")]);
        for l in &labels {
            cmd.extend([s("-map"), l.clone()]);
        }
        if labels.is_empty() {
            cmd.push(s("-an"));
        } else {
            cmd.extend([s("-c:a"), s("aac"), s("-b:a"), format!("{audio_kbps}k"), s("-ac"), s("2")]);
        }
    };
    match job.picture {
        Picture::Full => {
            let mut cmd = head();
            cmd.extend(inputs(job, true));
            with_sound(&mut cmd, 192);
            cmd.extend(["-c:v", "libx264", "-preset", "fast", "-crf", "16", "-movflags", "+faststart"].map(String::from));
            cmd.push(out);
            vec![cmd]
        }
        Picture::Fit(p) => {
            // The plan's sound is one track's worth. Tracks kept apart share
            // it, and what they need beyond it comes out of the picture.
            let tracks = match job.sound {
                Sound::Tracks => job.edit.gains.len().max(1) as u32,
                Sound::Mixed => 1,
            };
            let planned = p.audio_kbps.max(64);
            let each = if tracks > 1 { (planned / tracks).max(48) } else { planned };
            let video_kbps = p.video_kbps.saturating_sub((each * tracks).saturating_sub(planned)).max(1);
            let rate = |cmd: &mut Vec<String>, pass: &str| {
                cmd.extend([s("-c:v"), s("libx264"), s("-preset"), s("medium"), s("-profile:v"), s("high")]);
                cmd.extend([s("-b:v"), format!("{video_kbps}k")]);
                cmd.extend([s("-maxrate"), format!("{}k", video_kbps * 3 / 2), s("-bufsize"), format!("{}k", video_kbps * 3)]);
                // For the picture only: a newer ffmpeg otherwise looks for a
                // pass log for the sound too, which the first pass never made.
                cmd.extend([s("-pass:v"), s(pass), s("-passlogfile:v"), passlog.to_string_lossy().into_owned()]);
            };
            let mut first = head();
            first.extend(inputs(job, false));
            let (g, _) = graph(&job.edit, None, &job.picture);
            first.extend([s("-filter_complex"), g, s("-map"), s("[vo]"), s("-an")]);
            rate(&mut first, "1");
            // Thrown away: into the system's nowhere.
            first.extend([s("-f"), s("null"), s(if cfg!(windows) { "NUL" } else { "/dev/null" })]);

            let mut second = head();
            second.extend(inputs(job, true));
            with_sound(&mut second, each);
            rate(&mut second, "2");
            second.extend([s("-movflags"), s("+faststart"), out]);
            vec![first, second]
        }
    }
}

/// The position ffmpeg reports on a line of its `-progress` output, in seconds.
pub fn progress_seconds(line: &str) -> Option<f64> {
    let us = line.trim().strip_prefix("out_time_us=").or_else(|| line.trim().strip_prefix("out_time_ms="))?;
    us.parse::<i64>().ok().filter(|v| *v >= 0).map(|v| v as f64 / 1_000_000.0)
}

fn run_one(args: &[String], cancel: &AtomicBool, mut on_seconds: impl FnMut(f64)) -> Result<(), ExportError> {
    let mut child = crate::paths::command("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ExportError::Spawn)?;
    let stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
        text
    });
    // Its progress is read on a thread of its own, so a cancel is noticed
    // at once and not only when ffmpeg next has something to say.
    let stdout = child.stdout.take().expect("piped");
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ExportError::Cancelled);
        }
        match lines.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) => {
                if let Some(t) = progress_seconds(&line) {
                    on_seconds(t);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    let text = errors.join().unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err(ExportError::Cancelled);
    }
    if !status.success() {
        // Its last words; the line before often names the file or the
        // setting the last one is about.
        let mut last: Vec<&str> = text.lines().rev().filter(|l| !l.trim().is_empty()).take(3).collect();
        last.reverse();
        return Err(ExportError::Failed(if last.is_empty() { "no message".into() } else { last.join(" / ") }));
    }
    Ok(())
}

/// The files beside a pass log that the encoder makes.
fn passlog_files(passlog: &Path) -> Vec<PathBuf> {
    ["-0.log", "-0.log.mbtree", "-0.log.temp", "-0.log.mbtree.temp"]
        .iter()
        .map(|end| {
            let mut name = passlog.as_os_str().to_owned();
            name.push(end);
            PathBuf::from(name)
        })
        .collect()
}

/// Encode into `partial` until it is under the limit, or the tries run out.
fn attempts(
    job: &Job,
    partial: &Path,
    passlog: &Path,
    limit: Option<u64>,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(f64),
) -> Result<u64, ExportError> {
    const TRIES: usize = 3;
    let total = job.edit.kept().max(0.1);
    let mut attempt = Job { output: partial.to_path_buf(), ..job.clone() };
    let mut last = 0;
    for round in 0..TRIES {
        let mut cmds = commands(&attempt, passlog);
        // What the first pass learned about the picture still holds for
        // another try at a lower rate: only the second pass is run again.
        if round > 0 && cmds.len() > 1 {
            cmds.drain(..cmds.len() - 1);
        }
        let count = cmds.len() as f64;
        for (i, cmd) in cmds.iter().enumerate() {
            run_one(cmd, cancel, |t| progress(((i as f64 + (t / total).min(1.0)) / count).min(0.999)))?;
        }
        last = std::fs::metadata(partial)?.len();
        match (limit, attempt.picture) {
            (Some(limit), Picture::Fit(p)) if last > limit => attempt.picture = Picture::Fit(p.reduced(last, limit)),
            _ => return Ok(last),
        }
    }
    Err(ExportError::TooBig { actual: last, limit: limit.unwrap_or(0) })
}

/// Render a job. `progress` is told how far along it is, 0 to 1. With a size
/// limit the result is checked and made again smaller until it is under;
/// what is left at `job.output` is always a finished file under the limit,
/// or nothing.
pub fn run(job: &Job, limit: Option<u64>, cancel: &AtomicBool, mut progress: impl FnMut(f64)) -> Result<u64, ExportError> {
    if let Some(dir) = job.output.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside the result and renamed when done: a half-made file is
    // never at the result's name. The names are this process's and this
    // call's own, so two exports to one name do not write each other's files.
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let own = format!("{}-{}", std::process::id(), CALLS.fetch_add(1, Ordering::Relaxed));
    let partial = job.output.with_extension(format!("{own}.part.mp4"));
    let passlog = job.output.with_extension(format!("{own}.passlog"));
    let made = attempts(job, &partial, &passlog, limit, cancel, &mut progress)
        .and_then(|bytes| std::fs::rename(&partial, &job.output).map(|_| bytes).map_err(ExportError::from));
    // Whatever happened, nothing of the making is left behind.
    let _ = std::fs::remove_file(&partial);
    for file in passlog_files(&passlog) {
        let _ = std::fs::remove_file(file);
    }
    if made.is_ok() {
        progress(1.0);
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(edit: Edit, picture: Picture, sound: Sound) -> Job {
        Job { input: "/c/in.mp4".into(), output: "/c/out.mp4".into(), edit, picture, sound }
    }

    fn fit() -> Picture {
        Picture::Fit(Plan { width: 1280, height: 720, fps: 30.0, video_kbps: 2000, audio_kbps: 96 })
    }

    #[test]
    fn one_stretch_with_three_tracks_mixes_down() {
        let (g, labels) = graph(&Edit::new(10.0, 3), Some(Sound::Mixed), &fit());
        assert_eq!(labels, ["[ao0]"]);
        assert!(g.contains("[0:a:0]volume=1.000[m0_0];"), "{g}");
        assert!(g.contains("[m0_0][m0_1][m0_2]amix=inputs=3:normalize=0:duration=longest,alimiter=level=0:limit=0.98[a0];"), "{g}");
        assert!(g.contains("[0:v][a0]concat=n=1:v=1:a=1[vc][ao0];"), "{g}");
        assert!(g.ends_with("[vc]scale=1280:720:flags=lanczos,fps=30.000,format=yuv420p[vo]"), "{g}");
    }

    #[test]
    fn muted_tracks_are_left_out_and_volumes_applied() {
        let mut e = Edit::new(10.0, 3);
        e.set_muted(1, true);
        e.set_muted(2, true);
        e.set_volume(0, 0.5);
        let (g, labels) = graph(&e, Some(Sound::Mixed), &Picture::Full);
        assert_eq!(labels.len(), 1);
        assert!(g.contains("[0:a:0]volume=0.500[a0];") && !g.contains("amix") && !g.contains("a:1"), "{g}");
        e.set_muted(0, true);
        let (g, labels) = graph(&e, Some(Sound::Mixed), &Picture::Full);
        assert!(labels.is_empty());
        assert!(g.contains("[0:v]concat=n=1:v=1:a=0[vc];"), "{g}");
    }

    #[test]
    fn cuts_become_inputs_joined_in_order() {
        let mut e = Edit::new(60.0, 2);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        let j = job(e, fit(), Sound::Tracks);
        let cmds = commands(&j, Path::new("/c/log"));
        assert_eq!(cmds.len(), 2);
        let second = cmds[1].join(" ");
        assert!(second.contains("-ss 0.000 -t 10.000 -i /c/in.mp4 -ss 20.000 -t 40.000 -i /c/in.mp4"), "{second}");
        assert!(second.contains("[0:v][t0_0][t0_1][1:v][t1_0][t1_1]concat=n=2:v=1:a=2[vc][ao0][ao1];"), "{second}");
        assert!(second.contains("-map [vo] -map [ao0] -map [ao1]"), "{second}");
        assert!(second.contains("-pass:v 2") && second.contains("-b:v 2000k") && second.ends_with("-movflags +faststart /c/out.mp4"));
        let first = cmds[0].join(" ");
        assert!(first.contains("-pass:v 1") && first.contains("-an") && first.ends_with(if cfg!(windows) { "-f null NUL" } else { "-f null /dev/null" }), "{first}");
        assert!(!first.contains(":a:"), "the first pass reads no sound: {first}");
    }

    #[test]
    fn moved_sound_is_read_from_where_it_is_in_the_file() {
        let mut e = Edit::new(60.0, 2);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        e.set_muted(1, true);
        // Later by 250 ms: the sound for 0 s is silence, for 20 s it is what the file has at 19.75 s.
        e.set_audio_offset(250);
        let cmd = commands(&job(e.clone(), Picture::Full, Sound::Mixed), Path::new("/c/log"))[0].join(" ");
        assert!(cmd.contains("-ss 20.000 -t 40.000 -i /c/in.mp4 -vn -ss 0.000 -t 9.750 -i /c/in.mp4 -vn -ss 19.750 -t 40.000 -i /c/in.mp4"), "{cmd}");
        assert!(cmd.contains("[2:a:0]adelay=250:all=1,volume=1.000[a0];[3:a:0]volume=1.000[a1];"), "{cmd}");
        assert!(cmd.contains("[0:v][a0][1:v][a1]concat=n=2:v=1:a=1[vc][ao0];"), "{cmd}");
        // Earlier: read from further on, with nothing put before it.
        e.set_audio_offset(-250);
        let cmds = commands(&job(e, fit(), Sound::Mixed), Path::new("/c/log"));
        let second = cmds[1].join(" ");
        assert!(second.contains("-vn -ss 0.250 -t 10.000 -i /c/in.mp4 -vn -ss 20.250 -t 40.000 -i /c/in.mp4") && !second.contains("adelay"), "{second}");
        assert!(!cmds[0].join(" ").contains("-vn"), "the first pass reads no sound");
    }

    #[test]
    fn sound_is_limited_only_where_it_could_go_over() {
        let mut e = Edit::new(10.0, 2);
        e.set_volume(0, 0.4);
        e.set_volume(1, 0.5);
        let (g, _) = graph(&e, Some(Sound::Mixed), &Picture::Full);
        assert!(!g.contains("alimiter"), "0.4 + 0.5 stays under: {g}");
        e.set_volume(1, 2.0);
        let (g, _) = graph(&e, Some(Sound::Tracks), &Picture::Full);
        assert!(g.contains("[0:a:0]volume=0.400[t0_0];") && g.contains("[0:a:1]volume=2.000,alimiter=level=0:limit=0.98[t0_1];"), "{g}");
        assert!(g.ends_with("[vc]scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p[vo]"), "{g}");
    }

    #[test]
    fn tracks_kept_apart_share_the_sound_budget() {
        let cmds = commands(&job(Edit::new(30.0, 3), fit(), Sound::Tracks), Path::new("/c/log"));
        let second = cmds[1].join(" ");
        // 96 planned, three tracks at the least each can be: 48 more than planned.
        assert!(second.contains("-b:a 48k") && second.contains("-b:v 1952k"), "{second}");
        assert!(cmds[0].join(" ").contains("-b:v 1952k"), "both passes ask for the same picture");
        let mixed = commands(&job(Edit::new(30.0, 3), fit(), Sound::Mixed), Path::new("/c/log"))[1].join(" ");
        assert!(mixed.contains("-b:a 96k") && mixed.contains("-b:v 2000k"), "{mixed}");
    }

    #[test]
    fn full_quality_is_one_pass() {
        let cmds = commands(&job(Edit::new(5.0, 0), Picture::Full, Sound::Mixed), Path::new("/c/log"));
        assert_eq!(cmds.len(), 1);
        let cmd = cmds[0].join(" ");
        assert!(cmd.contains("-crf 16") && cmd.contains(" -an ") && !cmd.contains("-pass"), "{cmd}");
    }

    #[test]
    fn progress_lines_are_read() {
        assert_eq!(progress_seconds("out_time_us=1500000"), Some(1.5));
        assert_eq!(progress_seconds("out_time_ms=2000000\n"), Some(2.0));
        assert_eq!(progress_seconds("out_time_us=N/A"), None);
        assert_eq!(progress_seconds("frame=12"), None);
    }
}
