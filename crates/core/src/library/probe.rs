//! What a video file holds, asked of ffprobe.

use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub audio_tracks: u32,
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("ffprobe could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("not a playable video: {0}")]
    Invalid(String),
}

#[derive(Deserialize)]
struct Output {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
    size: Option<String>,
}

/// `"60/1"` or `"60000/1001"` as frames a second.
fn rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

/// Read ffprobe's JSON (`-show_format -show_streams`).
pub fn parse(json: &str) -> Result<Meta, ProbeError> {
    let out: Output = serde_json::from_str(json).map_err(|e| ProbeError::Invalid(e.to_string()))?;
    let is = |s: &Stream, kind: &str| s.codec_type.as_deref() == Some(kind);
    let video = out.streams.iter().find(|s| is(s, "video")).ok_or_else(|| ProbeError::Invalid("it has no picture".into()))?;
    let format = out.format.as_ref();
    let seconds = format
        .and_then(|f| f.duration.as_deref())
        .or(video.duration.as_deref())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| *d > 0.0)
        .ok_or_else(|| ProbeError::Invalid("it has no length".into()))?;
    let (Some(width), Some(height)) = (video.width, video.height) else {
        return Err(ProbeError::Invalid("its picture has no size".into()));
    };
    Ok(Meta {
        seconds,
        width,
        height,
        // The average is right for a variable rate; the nominal one is the fallback.
        fps: video.avg_frame_rate.as_deref().and_then(rate).or_else(|| video.r_frame_rate.as_deref().and_then(rate)).unwrap_or(60.0),
        codec: video.codec_name.clone().unwrap_or_default(),
        audio_tracks: out.streams.iter().filter(|s| is(s, "audio")).count() as u32,
        bytes: format.and_then(|f| f.size.as_deref()).and_then(|s| s.parse().ok()).unwrap_or(0),
    })
}

/// What `file` holds. An error means it is not a finished, playable video.
pub fn probe(file: &Path) -> Result<Meta, ProbeError> {
    let out = crate::paths::command("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .args(crate::media::FILES_ONLY)
        .arg(file)
        .output()
        .map_err(ProbeError::Spawn)?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        return Err(ProbeError::Invalid(text.lines().last().unwrap_or("ffprobe refused it").trim().to_string()));
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_with_three_tracks_is_read() {
        let json = r#"{"streams":[
            {"codec_type":"video","codec_name":"h264","width":2560,"height":1440,"avg_frame_rate":"60000/1001","r_frame_rate":"60/1"},
            {"codec_type":"audio","codec_name":"aac"},{"codec_type":"audio"},{"codec_type":"audio"}],
            "format":{"duration":"59.983000","size":"165000000"}}"#;
        let m = parse(json).unwrap();
        assert_eq!((m.width, m.height, m.audio_tracks, m.bytes), (2560, 1440, 3, 165_000_000));
        assert!((m.fps - 59.94).abs() < 0.01);
        assert!((m.seconds - 59.983).abs() < 1e-9);
        assert_eq!(m.codec, "h264");
    }

    #[test]
    fn what_is_not_a_video_is_refused() {
        assert!(parse(r#"{"streams":[{"codec_type":"audio"}],"format":{"duration":"3"}}"#).is_err());
        assert!(parse(r#"{"streams":[{"codec_type":"video","width":1,"height":1}],"format":{}}"#).is_err());
        assert!(parse("{}").is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn a_zero_rate_falls_back() {
        let json = r#"{"streams":[{"codec_type":"video","width":2,"height":2,"avg_frame_rate":"0/0","r_frame_rate":"30/1"}],"format":{"duration":"1"}}"#;
        assert_eq!(parse(json).unwrap().fps, 30.0);
    }
}
