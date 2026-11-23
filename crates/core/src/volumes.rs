//! What the editor remembers from one clip to the next: how loud it plays.
//! (How loud a clip's own tracks are is part of that clip's edit, and
//! where they start is in the settings.)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Volumes {
    /// How loud the editor plays, 0 to 1. Not part of any export.
    pub listening: f64,
}

impl Default for Volumes {
    fn default() -> Self {
        Self { listening: 1.0 }
    }
}

impl Volumes {
    /// Where it is kept.
    pub fn file() -> PathBuf {
        crate::paths::data_dir().join("volumes.json")
    }

    /// What is kept in `path`; nothing remembered when it cannot be read.
    pub fn load(path: &Path) -> Self {
        let mut kept: Self = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        kept.listening = if kept.listening.is_finite() { kept.listening.clamp(0.0, 1.0) } else { 1.0 };
        kept
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playback_volume_comes_back() {
        let dir = std::env::temp_dir().join(format!("zenith-volumes-{}", std::process::id()));
        let path = dir.join("volumes.json");
        assert_eq!(Volumes::load(&path), Volumes::default());

        Volumes { listening: 0.5 }.save(&path).unwrap();
        assert_eq!(Volumes::load(&path).listening, 0.5);

        // As 0.6.0 wrote it, with the tracks' volumes beside.
        std::fs::write(&path, r#"{"tracks":{"Game":0.4},"listening":0.25}"#).unwrap();
        assert_eq!(Volumes::load(&path).listening, 0.25);

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Volumes::load(&path), Volumes::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
