//! New versions: what a release says about itself, whether it is newer, and
//! whether a downloaded file really is the one that was published.
//!
//! A release is a few files in one place (a GitHub release): `latest.json`,
//! which names the version and the file for each kind of computer, the
//! files themselves, and beside each a `.minisig` signature. Nothing is
//! installed unless its signature was made with a key this build knows.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where the newest release is read from.
pub const RELEASES: &str = "https://github.com/Draysh/zenith-releases/releases/latest/download";

/// The name of the file that describes a release.
pub const MANIFEST: &str = "latest.json";

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum UpdateError {
    #[error("this copy has no update key, so it installs nothing")]
    NoKey,
    #[error("the signature could not be read")]
    BadSignature,
    #[error("the download was not signed with Zenith's key; it is not installed")]
    NotOurs,
    #[error("the signature is for another file ({0}); it is not installed")]
    OtherFile(String),
    #[error("the release does not say what it is: {0}")]
    BadManifest(String),
    #[error("version {offered} is not newer than this one ({current})")]
    NotNewer { offered: String, current: String },
}

/// What `latest.json` holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// What changed, as the release notes were written.
    #[serde(default)]
    pub notes: String,
    /// When it was published, as text.
    #[serde(default)]
    pub published: String,
    /// The file for each kind of computer, by [`target`].
    #[serde(default)]
    pub files: BTreeMap<String, File>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct File {
    pub name: String,
    #[serde(default)]
    pub size: u64,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, UpdateError> {
        serde_json::from_str(text).map_err(|e| UpdateError::BadManifest(e.to_string()))
    }

    /// The file for this computer, when the release has one.
    pub fn file(&self) -> Option<&File> {
        self.files.get(&target())
    }
}

/// The kind of computer this build is for: `linux-x86_64`, `windows-x86_64`.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The name a release's file for `target` goes by.
pub fn file_name(version: &str, target: &str) -> String {
    if target.starts_with("windows") {
        format!("Zenith-{version}-{target}-setup.exe")
    } else {
        format!("zenith-{version}-{target}.tar.gz")
    }
}

/// Where `file` of the newest release is. `ZENITH_UPDATE_URL` names
/// another place, for trying an update out.
pub fn url(file: &str) -> String {
    let base = std::env::var("ZENITH_UPDATE_URL").unwrap_or_else(|_| RELEASES.into());
    format!("{}/{file}", base.trim_end_matches('/'))
}

/// The patch notes of `version` in the text of `CHANGELOG.md`: its section,
/// without the heading. Nothing when it has none.
pub fn notes_for(changelog: &str, version: &str) -> String {
    let heading = format!("## {}", version.trim().trim_start_matches('v'));
    let section: Vec<&str> =
        changelog.lines().skip_while(|l| l.trim_end() != heading).skip(1).take_while(|l| !l.starts_with("## ")).collect();
    section.join("\n").trim().to_string()
}

/// Whether `candidate` is a later version than `than`. A version that does
/// not read as one is never newer.
pub fn newer(candidate: &str, than: &str) -> Option<bool> {
    let read = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).ok();
    Some(read(candidate)? > read(than)?)
}

/// The keys in an `update.pub`: one to a line, as minisign writes a public
/// key (`RW…`). Comment lines and blank ones are passed over.
fn keys(text: &str) -> Vec<minisign_verify::PublicKey> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("untrusted comment") && !l.starts_with('#'))
        .filter_map(|l| minisign_verify::PublicKey::from_base64(l).ok())
        .collect()
}

/// Whether `pubkeys` holds a key at all. A build without one installs
/// nothing, so it does not look either.
pub fn has_key(pubkeys: &str) -> bool {
    !keys(pubkeys).is_empty()
}

/// Check that `data` is the file `name` of a release: `signature` (the
/// text of its `.minisig`) was made over it by one of `pubkeys`, and says
/// it is for that name, so a signed file cannot stand in for another.
pub fn verify(pubkeys: &str, signature: &str, name: &str, data: &[u8]) -> Result<(), UpdateError> {
    let keys = keys(pubkeys);
    if keys.is_empty() {
        return Err(UpdateError::NoKey);
    }
    let sig = minisign_verify::Signature::decode(signature).map_err(|_| UpdateError::BadSignature)?;
    if !keys.iter().any(|key| key.verify(data, &sig, false).is_ok()) {
        return Err(UpdateError::NotOurs);
    }
    let signed = sig.trusted_comment().split('\t').find_map(|part| part.trim().strip_prefix("file:")).unwrap_or_default();
    if signed != name {
        return Err(UpdateError::OtherFile(signed.to_owned()));
    }
    Ok(())
}

/// The trusted comment a signature for `name` carries.
pub fn trusted_comment(name: &str, seconds: u64) -> String {
    format!("timestamp:{seconds}\tfile:{name}")
}

/// What a release offers this copy: its manifest, once it is known to be
/// ours, newer, and to have a file for this computer.
pub fn offer(pubkeys: &str, manifest: &str, signature: &str, current: &str) -> Result<Option<(Manifest, File)>, UpdateError> {
    verify(pubkeys, signature, MANIFEST, manifest.as_bytes())?;
    let manifest = Manifest::parse(manifest)?;
    if newer(&manifest.version, current) != Some(true) {
        return Ok(None);
    }
    let Some(file) = manifest.file().cloned() else { return Ok(None) };
    // The name is ours to decide, not the release's: nothing else is fetched.
    if file.name != file_name(&manifest.version, &target()) {
        return Err(UpdateError::BadManifest(format!("an unexpected file name, {}", file.name)));
    }
    Ok(Some((manifest, file)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minisign::KeyPair;
    use std::io::Cursor;

    fn sign(pair: &KeyPair, name: &str, data: &[u8]) -> String {
        minisign::sign(Some(&pair.pk), &pair.sk, Cursor::new(data), Some(&trusted_comment(name, 1)), None).unwrap().into_string()
    }

    fn manifest(version: &str) -> String {
        let name = file_name(version, &target());
        format!(r#"{{"version":"{version}","notes":"Faster.","files":{{"{}":{{"name":"{name}","size":3}}}}}}"#, target())
    }

    #[test]
    fn versions_compare_as_versions() {
        assert_eq!(newer("0.10.0", "0.9.9"), Some(true));
        assert_eq!(newer("v0.2.0", "0.2.0"), Some(false));
        assert_eq!(newer("0.2.0-rc.1", "0.2.0"), Some(false), "a release candidate is before its release");
        assert_eq!(newer("0.1.0", "0.2.0"), Some(false));
        assert_eq!(newer("latest", "0.2.0"), None);
    }

    #[test]
    fn a_versions_notes_are_its_section_of_the_changelog() {
        let log = "# What changed\n\nWords.\n\n## 0.2.0\n\n### New\n- A thing.\n\n### Fixed\n- Another.\n\n## 0.1.0\n\n- The first.\n";
        assert_eq!(notes_for(log, "0.2.0"), "### New\n- A thing.\n\n### Fixed\n- Another.");
        assert_eq!(notes_for(log, "v0.1.0"), "- The first.");
        assert_eq!(notes_for(log, "0.3.0"), "");
        assert_eq!(notes_for(log, "0.2"), "", "a part of a version is not that version");
    }

    #[test]
    fn files_are_named_by_version_and_computer() {
        assert_eq!(file_name("0.2.0", "linux-x86_64"), "zenith-0.2.0-linux-x86_64.tar.gz");
        assert_eq!(file_name("0.2.0", "windows-x86_64"), "Zenith-0.2.0-windows-x86_64-setup.exe");
    }

    #[test]
    fn only_our_signature_over_the_named_file_passes() {
        let ours = KeyPair::generate_unencrypted_keypair().unwrap();
        let theirs = KeyPair::generate_unencrypted_keypair().unwrap();
        let pubkeys = format!("untrusted comment: Zenith\n{}\n", ours.pk.to_base64());
        let data = b"the new version";
        let good = sign(&ours, "zenith-0.2.0-linux-x86_64.tar.gz", data);
        assert_eq!(verify(&pubkeys, &good, "zenith-0.2.0-linux-x86_64.tar.gz", data), Ok(()));
        assert_eq!(verify(&pubkeys, &good, "zenith-0.2.0-linux-x86_64.tar.gz", b"something else"), Err(UpdateError::NotOurs));
        assert_eq!(
            verify(&pubkeys, &good, "latest.json", data),
            Err(UpdateError::OtherFile("zenith-0.2.0-linux-x86_64.tar.gz".into())),
            "a signed file does not pass for another"
        );
        let forged = sign(&theirs, "zenith-0.2.0-linux-x86_64.tar.gz", data);
        assert_eq!(verify(&pubkeys, &forged, "zenith-0.2.0-linux-x86_64.tar.gz", data), Err(UpdateError::NotOurs));
        assert_eq!(verify(&pubkeys, "not a signature", "x", data), Err(UpdateError::BadSignature));
        assert_eq!(verify("", &good, "x", data), Err(UpdateError::NoKey));
        // Two keys, while one is being retired: either will do.
        let both = format!("{}\n{}\n", theirs.pk.to_base64(), ours.pk.to_base64());
        assert_eq!(verify(&both, &good, "zenith-0.2.0-linux-x86_64.tar.gz", data), Ok(()));
    }

    #[test]
    fn a_release_is_offered_only_when_ours_and_newer() {
        let pair = KeyPair::generate_unencrypted_keypair().unwrap();
        let pubkeys = pair.pk.to_base64();
        let text = manifest("0.2.0");
        let sig = sign(&pair, MANIFEST, text.as_bytes());
        let (found, file) = offer(&pubkeys, &text, &sig, "0.1.0").unwrap().unwrap();
        assert_eq!((found.version.as_str(), found.notes.as_str()), ("0.2.0", "Faster."));
        assert_eq!(file.name, file_name("0.2.0", &target()));
        assert_eq!(offer(&pubkeys, &text, &sig, "0.2.0").unwrap(), None, "the same version is not an update");
        assert_eq!(offer(&pubkeys, &text, &sig, "0.3.0").unwrap(), None, "nor is an older one");
        let tampered = text.replace("0.2.0", "9.9.9");
        assert_eq!(offer(&pubkeys, &tampered, &sig, "0.1.0"), Err(UpdateError::NotOurs));
        // A release with nothing for this computer offers nothing.
        let other = r#"{"version":"0.2.0","files":{"plan9-mips":{"name":"x","size":1}}}"#;
        let sig = sign(&pair, MANIFEST, other.as_bytes());
        assert_eq!(offer(&pubkeys, other, &sig, "0.1.0").unwrap(), None);
        // A file under another name is refused.
        let odd = format!(r#"{{"version":"0.2.0","files":{{"{}":{{"name":"evil.tar.gz"}}}}}}"#, target());
        let sig = sign(&pair, MANIFEST, odd.as_bytes());
        assert!(matches!(offer(&pubkeys, &odd, &sig, "0.1.0"), Err(UpdateError::BadManifest(_))));
    }
}
