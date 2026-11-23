//! `zenith-sign`: the release side of updates.
//!
//! ```text
//! zenith-sign keygen <key file>            make the update key; prints its public half
//! zenith-sign sign <file>...               write <file>.minisig beside each
//! zenith-sign manifest <version> <notes file> <folder>
//!                                          write latest.json for the release files in <folder>
//! ```
//!
//! The key is read from `ZENITH_SIGNING_KEY` (its text) or, failing that,
//! from the file `ZENITH_SIGNING_KEY_FILE` names. `ZENITH_SIGNING_PASSWORD`
//! is its password, when it has one. Installed copies accept updates signed
//! with this key and nothing else: keep it, and keep it to yourself.
//!
//! Which key that is, is written in `app/update.pub`, and the app is built
//! with that file. So is this program: what it signs it checks against
//! the same file, and a release signed with any other key (which no
//! installed copy could install) is refused here, not found out later.

use minisign::{KeyPair, PublicKey, SecretKey, SecretKeyBox};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use zenith_core::update::{self, File, Manifest};

/// The kinds of computer a release may have a file for.
const TARGETS: [&str; 2] = ["linux-x86_64", "windows-x86_64"];

/// The keys installed copies accept updates from: what the app is built with.
const TRUSTED: &str = include_str!("../../../app/update.pub");

type Outcome<T> = Result<T, String>;

fn password() -> Option<String> {
    std::env::var("ZENITH_SIGNING_PASSWORD").ok().filter(|p| !p.is_empty())
}

fn keygen(file: &Path) -> Outcome<String> {
    use std::io::Write;
    let taken = || format!("{} is already there; a new key would lock every installed copy out of updates", file.display());
    if file.exists() {
        return Err(taken());
    }
    let pair = match password() {
        Some(password) => KeyPair::generate_encrypted_keypair(Some(password)),
        None => {
            // Said where it cannot be missed: whoever reads the file has the key.
            eprintln!("zenith-sign: WARNING: ZENITH_SIGNING_PASSWORD is not set, so the key in {} is NOT ENCRYPTED.", file.display());
            eprintln!("zenith-sign: WARNING: anyone who can read that file can sign updates. Set a password and make the key again, unless that is meant.");
            KeyPair::generate_unencrypted_keypair()
        }
    }
    .map_err(|e| e.to_string())?;
    let text = pair.sk.to_box(Some("Zenith update key")).map_err(|e| e.to_string())?.into_string();
    // Made new, and the owner's alone from the start: it is never, even
    // for a moment, a file others can read, and never written over one.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut made = options.open(file).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => taken(),
        _ => format!("{} could not be written: {e}", file.display()),
    })?;
    made.write_all(text.as_bytes()).and_then(|_| made.sync_all()).map_err(|e| format!("{} could not be written: {e}", file.display()))?;
    Ok(pair.pk.to_base64())
}

fn secret_key() -> Outcome<SecretKey> {
    let text = match std::env::var("ZENITH_SIGNING_KEY").ok().filter(|k| !k.trim().is_empty()) {
        Some(text) => text,
        None => {
            let file = std::env::var("ZENITH_SIGNING_KEY_FILE").map_err(|_| "set ZENITH_SIGNING_KEY or ZENITH_SIGNING_KEY_FILE".to_string())?;
            std::fs::read_to_string(&file).map_err(|e| format!("{file} could not be read: {e}"))?
        }
    };
    let boxed = SecretKeyBox::from_string(text.trim()).map_err(|e| format!("the key does not parse: {e}"))?;
    match password() {
        Some(password) => boxed.into_secret_key(Some(password)),
        None => boxed.into_unencrypted_secret_key(),
    }
    .map_err(|e| format!("the key could not be opened: {e}"))
}

/// Sign `file`: the text of its `.minisig`, which names the file. The
/// signature has to be one that `trusted` (the text of an `update.pub`)
/// accepts, or there is none.
fn sign(key: &SecretKey, file: &Path, trusted: &str) -> Outcome<String> {
    let name = file.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{} has no name", file.display()))?;
    let data = std::fs::read(file).map_err(|e| format!("{} could not be read: {e}", file.display()))?;
    let public = PublicKey::from_secret_key(key).map_err(|e| e.to_string())?;
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let signature = minisign::sign(Some(&public), key, Cursor::new(&data), Some(&update::trusted_comment(name, seconds)), Some("signed with the Zenith update key"))
        .map_err(|e| format!("signing failed: {e}"))?
        .into_string();
    // Checked the way the app will check it, against the keys the app is
    // built with, before it goes out: signed with another key, it would
    // be a release that nobody can install.
    update::verify(trusted, &signature, name, &data).map_err(|e| match e {
        update::UpdateError::NotOurs | update::UpdateError::NoKey => format!(
            "the signing key is not one that app/update.pub names, so no installed copy would accept {name}. Its public half is {}",
            public.to_base64()
        ),
        other => other.to_string(),
    })?;
    Ok(signature)
}

/// What `latest.json` says for the release files of `version` in `folder`.
fn manifest(version: &str, notes: &str, folder: &Path) -> Outcome<Manifest> {
    let mut files = BTreeMap::new();
    for target in TARGETS {
        let name = update::file_name(version, target);
        if let Ok(meta) = std::fs::metadata(folder.join(&name)) {
            files.insert(target.to_string(), File { name, size: meta.len() });
        }
    }
    if files.is_empty() {
        return Err(format!("no release file for {version} in {}", folder.display()));
    }
    let published = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_default();
    Ok(Manifest { version: version.to_string(), notes: notes.trim().to_string(), published, files })
}

fn run(args: &[String]) -> Outcome<()> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", file] => {
            let public = keygen(Path::new(file))?;
            println!("The key is in {file}. Its public half, for app/update.pub:\n{public}");
            Ok(())
        }
        ["sign", files @ ..] if !files.is_empty() => {
            let key = secret_key()?;
            for file in files {
                let signature = sign(&key, Path::new(file), TRUSTED)?;
                let out = format!("{file}.minisig");
                std::fs::write(&out, signature).map_err(|e| format!("{out} could not be written: {e}"))?;
                println!("signed {file}");
            }
            Ok(())
        }
        ["manifest", version, notes, folder] => {
            let notes = std::fs::read_to_string(notes).map_err(|e| format!("{notes} could not be read: {e}"))?;
            let version = version.trim_start_matches('v');
            let made = manifest(version, &notes, Path::new(folder))?;
            let out = Path::new(folder).join(update::MANIFEST);
            let text = serde_json::to_string_pretty(&made).map_err(|e| e.to_string())?;
            std::fs::write(&out, text).map_err(|e| format!("{} could not be written: {e}", out.display()))?;
            println!("wrote {}", out.display());
            Ok(())
        }
        _ => Err("usage: zenith-sign keygen <key file> | sign <file>... | manifest <version> <notes file> <folder>".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(why) = run(&args) {
        eprintln!("zenith-sign: {why}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_signed_here_is_what_the_app_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let pair = KeyPair::generate_unencrypted_keypair().unwrap();
        let name = update::file_name("0.2.0", "linux-x86_64");
        std::fs::write(dir.path().join(&name), b"the programs").unwrap();
        let trusted = format!("# the keys\n{}\n", pair.pk.to_base64());
        let signature = sign(&pair.sk, &dir.path().join(&name), &trusted).unwrap();
        assert_eq!(update::verify(&trusted, &signature, &name, b"the programs"), Ok(()));

        let made = manifest("0.2.0", "Faster.\n", dir.path()).unwrap();
        assert_eq!(made.files["linux-x86_64"], File { name, size: 12 });
        assert_eq!((made.version.as_str(), made.notes.as_str(), made.files.len()), ("0.2.0", "Faster.", 1));
        assert!(manifest("0.3.0", "", dir.path()).is_err(), "a release with no files is not one");
    }

    #[test]
    fn a_key_the_app_does_not_trust_signs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (ours, another) = (KeyPair::generate_unencrypted_keypair().unwrap(), KeyPair::generate_unencrypted_keypair().unwrap());
        let file = dir.path().join(update::file_name("0.2.0", "linux-x86_64"));
        std::fs::write(&file, b"the programs").unwrap();
        let trusted = format!("{}\n", ours.pk.to_base64());
        let why = sign(&another.sk, &file, &trusted).unwrap_err();
        assert!(why.contains("app/update.pub") && why.contains(&another.pk.to_base64()), "{why}");
        assert!(sign(&another.sk, &file, "# no key at all\n").is_err());
        assert!(sign(&ours.sk, &file, &trusted).is_ok());
        // A new key is not the one this build trusts either, until its
        // public half is put in app/update.pub.
        assert!(sign(&another.sk, &file, TRUSTED).is_err());
    }

    #[test]
    fn a_key_is_made_once_and_is_the_owners_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("zenith-updates.key");
        let public = keygen(&file).unwrap();
        assert!(PublicKey::from_base64(&public).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let before = std::fs::read(&file).unwrap();
        assert!(keygen(&file).is_err(), "an existing key is never written over");
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
