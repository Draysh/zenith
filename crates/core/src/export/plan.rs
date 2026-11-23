//! Fitting a clip under a file size: how many bits there are to spend, and
//! the picture size and frame rate that look best with them.

use serde::{Deserialize, Serialize};

const MIB: u64 = 1024 * 1024;

/// Kept clear of the limit for the container's own bytes and the encoder's
/// drift from the bitrate it is asked for.
const MARGIN: f64 = 0.96;

/// Below this many bits per pixel per frame the picture falls apart, so a
/// smaller size or a lower rate is used instead.
const MIN_BITS_PER_PIXEL: f64 = 0.06;
/// More than this is wasted: a short clip is not padded up to the limit.
const MAX_BITS_PER_PIXEL: f64 = 0.20;
/// Under this the result is not worth making.
const MIN_VIDEO_KBPS: u32 = 120;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Discord without Nitro.
    Discord,
    NitroBasic,
    Nitro,
    Custom { megabytes: u32 },
}

impl Preset {
    pub fn label(self) -> String {
        match self {
            Self::Discord => "Discord".into(),
            Self::NitroBasic => "Nitro Basic".into(),
            Self::Nitro => "Nitro".into(),
            Self::Custom { megabytes } => format!("{megabytes} MB"),
        }
    }

    pub fn megabytes(self) -> u32 {
        match self {
            Self::Discord => 20,
            Self::NitroBasic => 50,
            Self::Nitro => 500,
            Self::Custom { megabytes } => megabytes.clamp(1, 4000),
        }
    }

    /// The most bytes the file may be.
    pub fn max_bytes(self) -> u64 {
        self.megabytes() as u64 * MIB
    }
}

/// What is being exported: the picture as recorded, and the length of the
/// edit's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub seconds: f64,
    pub has_audio: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_kbps: u32,
    /// 0 when the result has no sound.
    pub audio_kbps: u32,
}

impl Plan {
    /// The size the file should come out at, in bytes.
    pub fn expected_bytes(&self, seconds: f64) -> u64 {
        ((self.video_kbps + self.audio_kbps) as f64 * 1000.0 / 8.0 * seconds) as u64
    }

    /// The same plan with less video, for another try after a file came out
    /// `actual` bytes when `limit` was allowed.
    pub fn reduced(&self, actual: u64, limit: u64) -> Self {
        let ratio = (limit as f64 / actual as f64).min(1.0) * 0.97;
        Self { video_kbps: ((self.video_kbps as f64 * ratio) as u32).max(1), ..*self }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlanError {
    #[error("this clip is too long to fit in {megabytes} MB: at most about {max_seconds} seconds would")]
    TooLong { megabytes: u32, max_seconds: u32 },
}

/// A height's size for a picture shaped like `src`, with even sides.
fn scaled(src: &Source, height: u32) -> (u32, u32) {
    if height >= src.height {
        return (src.width & !1, src.height & !1);
    }
    let w = (src.width as f64 * height as f64 / src.height as f64).round() as u32;
    (w.max(2) & !1, height & !1)
}

/// The plan that fits `src` under `max_bytes`.
pub fn plan(src: Source, preset: Preset) -> Result<Plan, PlanError> {
    let max_bytes = preset.max_bytes();
    let seconds = src.seconds.max(0.1);
    let total_kbps = max_bytes as f64 * 8.0 / 1000.0 / seconds * MARGIN;
    let audio_kbps = match (src.has_audio, total_kbps) {
        (false, _) => 0,
        (true, t) if t >= 1500.0 => 160,
        (true, t) if t >= 600.0 => 96,
        (true, _) => 64,
    };
    let budget = total_kbps - audio_kbps as f64;
    if budget < MIN_VIDEO_KBPS as f64 {
        let floor = (MIN_VIDEO_KBPS + audio_kbps) as f64;
        let max_seconds = (max_bytes as f64 * 8.0 / 1000.0 * MARGIN / floor) as u32;
        return Err(PlanError::TooLong { megabytes: preset.megabytes(), max_seconds });
    }

    let high = src.fps.min(60.0);
    let low = src.fps.min(30.0);
    // From best to worst: size is given up before smoothness down to 720p,
    // then smoothness, then size again.
    let steps = [(u32::MAX, high), (1440, high), (1080, high), (720, high), (720, low), (540, low), (480, low), (360, low)];
    let mut chosen = None;
    for (height, fps) in steps {
        let (w, h) = scaled(&src, height);
        let per_kbit = w as f64 * h as f64 * fps / 1000.0;
        chosen = Some((w, h, fps, per_kbit));
        if budget >= per_kbit * MIN_BITS_PER_PIXEL {
            break;
        }
    }
    let (width, height, fps, per_kbit) = chosen.expect("there are steps");
    let video_kbps = budget.min(per_kbit * MAX_BITS_PER_PIXEL).floor() as u32;
    Ok(Plan { width, height, fps, video_kbps, audio_kbps })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(seconds: f64) -> Source {
        Source { width: 2560, height: 1440, fps: 60.0, seconds, has_audio: true }
    }

    #[test]
    fn every_plan_fits_its_limit() {
        for preset in [Preset::Discord, Preset::NitroBasic, Preset::Nitro, Preset::Custom { megabytes: 8 }] {
            for seconds in [3.0, 15.0, 30.0, 60.0, 120.0, 300.0] {
                if let Ok(p) = plan(src(seconds), preset) {
                    assert!(
                        p.expected_bytes(seconds) <= (preset.max_bytes() as f64 * MARGIN) as u64 + 1,
                        "{preset:?} {seconds}s {p:?}"
                    );
                    assert!(p.width % 2 == 0 && p.height % 2 == 0);
                }
            }
        }
    }

    #[test]
    fn a_short_clip_keeps_its_size_and_is_not_padded() {
        let p = plan(src(5.0), Preset::Nitro).unwrap();
        assert_eq!((p.width, p.height, p.fps), (2560, 1440, 60.0));
        // Capped by quality, far below the 500 MB it could use.
        assert!(p.expected_bytes(5.0) < 30 * MIB, "{p:?}");
    }

    #[test]
    fn longer_clips_step_down() {
        let short = plan(src(15.0), Preset::Discord).unwrap();
        let minute = plan(src(60.0), Preset::Discord).unwrap();
        let long = plan(src(240.0), Preset::Discord).unwrap();
        assert_eq!((short.height, short.fps), (1080, 60.0), "{short:?}");
        assert_eq!((minute.height, minute.fps), (720, 30.0), "{minute:?}");
        assert!(long.height <= 480 && long.fps == 30.0, "{long:?}");
        assert!(short.video_kbps > minute.video_kbps && minute.video_kbps > long.video_kbps);
    }

    #[test]
    fn a_clip_that_cannot_fit_says_how_long_would() {
        let err = plan(src(3600.0), Preset::Discord).unwrap_err();
        let PlanError::TooLong { megabytes: 20, max_seconds } = err else { panic!("{err:?}") };
        assert!((800..1000).contains(&max_seconds), "{max_seconds}");
        assert!(plan(src(max_seconds as f64 - 5.0), Preset::Discord).is_ok());
    }

    #[test]
    fn shape_and_rate_are_never_raised() {
        let wide = Source { width: 3440, height: 1440, fps: 144.0, seconds: 20.0, has_audio: false };
        let p = plan(wide, Preset::Discord).unwrap();
        assert_eq!(p.audio_kbps, 0);
        assert!(p.fps <= 60.0);
        assert!((p.width as f64 / p.height as f64 - 3440.0 / 1440.0).abs() < 0.01, "{p:?}");
        let small = Source { width: 1280, height: 720, fps: 30.0, seconds: 5.0, has_audio: true };
        let p = plan(small, Preset::Nitro).unwrap();
        assert_eq!((p.width, p.height, p.fps), (1280, 720, 30.0));
    }

    #[test]
    fn a_retry_asks_for_less() {
        let p = plan(src(60.0), Preset::Discord).unwrap();
        let again = p.reduced(22 * MIB, 20 * MIB);
        assert!(again.video_kbps < p.video_kbps * 20 / 22);
        assert_eq!((again.width, again.audio_kbps), (p.width, p.audio_kbps));
    }

    #[test]
    fn presets_are_megabytes_of_1024() {
        assert_eq!(Preset::Discord.max_bytes(), 20_971_520);
        assert_eq!(Preset::Custom { megabytes: 0 }.megabytes(), 1);
        assert_eq!(Preset::NitroBasic.label(), "Nitro Basic");
    }
}
