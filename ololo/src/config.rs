use crate::error::OlolError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SERVICE: &str = "ololo";

/// A single set of credentials (token + server URL) for one profile.
#[derive(Debug, Clone)]
pub struct Config {
    pub token: String,
    pub server_url: String,
}

// ── File format ───────────────────────────────────────────────────────────────
//
// ~/.config/ololo/credentials.toml
//
//   [default]
//   server_url = "https://ololo.dev"
//   token      = "ololo_abc..."
//
//   [staging]
//   server_url = "https://staging.ololo.dev"
//   token      = "ololo_xyz..."

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    // Flatten so each profile key becomes a top-level TOML table.
    #[serde(flatten)]
    profiles: BTreeMap<String, ProfileEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ProfileEntry {
    server_url: String,
    token: String,
}

// ── Keyring keys ──────────────────────────────────────────────────────────────
//
// Service is always "ololo". Account names are namespaced by profile so
// multiple profiles can coexist in the same keychain.

fn keyring_account_token(profile: &str) -> String {
    format!("token:{profile}")
}

fn keyring_account_server(profile: &str) -> String {
    format!("server_url:{profile}")
}

// ── Config impl ───────────────────────────────────────────────────────────────

impl Config {
    /// Persist credentials for `profile`.
    ///
    /// The file is written first (reliable source of truth). The keyring is
    /// then attempted as a bonus — failures there are silently ignored.
    pub fn save(&self, profile: &str) -> Result<(), OlolError> {
        self.save_to_file(profile)?;
        self.try_save_to_keyring(profile);
        Ok(())
    }

    /// Attempt a keyring write and immediately read back to verify it stuck.
    /// Returns `true` only when both values are confirmed persisted.
    fn try_save_to_keyring(&self, profile: &str) -> bool {
        let key_t = keyring_account_token(profile);
        let key_s = keyring_account_server(profile);
        let Ok(te) = keyring::Entry::new(SERVICE, &key_t) else {
            return false;
        };
        let Ok(se) = keyring::Entry::new(SERVICE, &key_s) else {
            return false;
        };
        if te.set_password(&self.token).is_err() {
            return false;
        }
        if se.set_password(&self.server_url).is_err() {
            return false;
        }
        // Read back to confirm the write actually persisted (some macOS
        // configurations return Ok from set_password but silently drop the
        // write — unsigned dev binaries are the common culprit).
        te.get_password().ok().as_deref() == Some(&self.token)
            && se.get_password().ok().as_deref() == Some(&self.server_url)
    }

    fn save_to_file(&self, profile: &str) -> Result<(), OlolError> {
        let path = credentials_path().map_err(OlolError::StorageError)?;

        // Load existing file so we don't clobber other profiles.
        let mut creds: CredentialsFile = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| OlolError::StorageError(e.to_string()))?;
            toml::from_str(&raw).unwrap_or_default()
        } else {
            CredentialsFile::default()
        };

        creds.profiles.insert(
            profile.to_owned(),
            ProfileEntry {
                server_url: self.server_url.clone(),
                token: self.token.clone(),
            },
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OlolError::StorageError(e.to_string()))?;
        }

        let content =
            toml::to_string(&creds).map_err(|e| OlolError::StorageError(e.to_string()))?;
        std::fs::write(&path, content).map_err(|e| OlolError::StorageError(e.to_string()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| OlolError::StorageError(e.to_string()))?;
        }
        Ok(())
    }

    /// Load credentials for `profile`. Tries keyring first, then file.
    pub fn load(profile: &str) -> Option<Self> {
        Self::load_from_keyring(profile).or_else(|| Self::load_from_file(profile))
    }

    fn load_from_keyring(profile: &str) -> Option<Self> {
        let key_t = keyring_account_token(profile);
        let key_s = keyring_account_server(profile);
        let te = keyring::Entry::new(SERVICE, &key_t).ok()?;
        let se = keyring::Entry::new(SERVICE, &key_s).ok()?;
        let token = te.get_password().ok().filter(|s| !s.is_empty())?;
        let server_url = se.get_password().ok().filter(|s| !s.is_empty())?;
        Some(Self { token, server_url })
    }

    fn load_from_file(profile: &str) -> Option<Self> {
        let path = credentials_path().ok()?;
        let raw = std::fs::read_to_string(path).ok()?;
        let creds: CredentialsFile = toml::from_str(&raw).ok()?;
        let entry = creds.profiles.get(profile)?;
        if entry.token.is_empty() || entry.server_url.is_empty() {
            return None;
        }
        Some(Self {
            token: entry.token.clone(),
            server_url: entry.server_url.clone(),
        })
    }
}

/// Resolve the Arena server URL using the priority chain:
/// 1. `--server` flag (if `Some`)
/// 2. `OLOLO_URL` environment variable (if set and non-empty)
/// 3. Stored `server_url` from the active profile (if present)
/// 4. Default `https://ololo.dev`
pub fn resolve_server_url(flag: Option<String>, profile: &str) -> String {
    if let Some(url) = flag {
        return url;
    }
    if let Ok(url) = std::env::var("OLOLO_URL")
        && !url.is_empty()
    {
        return url;
    }
    if let Some(cfg) = Config::load(profile)
        && !cfg.server_url.is_empty()
    {
        return cfg.server_url;
    }
    "https://ololo.dev".to_string()
}

fn credentials_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "cannot determine home directory".to_string())?;
    Ok(std::path::Path::new(&home)
        .join(".config")
        .join("ololo")
        .join("credentials.toml"))
}

#[cfg(test)]
pub(crate) fn env_lock() -> &'static std::sync::Mutex<()> {
    // The same lock snapshot tests take (HOME_LOCK): the process env is one
    // global, so two lock instances would only pretend to serialize it. A
    // config test dropping HOME between another thread's HomeGuard::set and
    // its first HOME read was exactly the CI flake this prevents.
    &crate::test_util::test_util::HOME_LOCK
}

/// Summary of a stored profile (name + server URL, no secrets).
pub struct ProfileSummary {
    pub name: String,
    pub server_url: String,
}

/// Return all profiles stored in the credentials file, sorted alphabetically.
/// Returns an empty Vec when the credentials file does not exist.
pub fn list_profiles() -> Result<Vec<ProfileSummary>, OlolError> {
    let path = credentials_path().map_err(OlolError::StorageError)?;
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| OlolError::StorageError(e.to_string()))?;
    let creds: CredentialsFile =
        toml::from_str(&raw).map_err(|e| OlolError::StorageError(e.to_string()))?;
    // BTreeMap guarantees alphabetical order.
    Ok(creds
        .profiles
        .into_iter()
        .map(|(name, entry)| ProfileSummary {
            name,
            server_url: entry.server_url,
        })
        .collect())
}

impl Config {
    /// Delete the credentials for `profile`.
    ///
    /// Atomicity contract (see spec):
    /// 1. Profile must exist in the TOML file.
    /// 2. Delete keyring token entry (NoEntry → success).
    /// 3. Delete keyring server_url entry (NoEntry → success).
    /// 4. Remove from TOML and write.
    pub fn delete(profile: &str) -> Result<(), OlolError> {
        let path = credentials_path().map_err(OlolError::StorageError)?;

        // 1. Load file and verify profile exists.
        let raw =
            std::fs::read_to_string(&path).map_err(|e| OlolError::StorageError(e.to_string()))?;
        let mut creds: CredentialsFile =
            toml::from_str(&raw).map_err(|e| OlolError::StorageError(e.to_string()))?;
        if !creds.profiles.contains_key(profile) {
            return Err(OlolError::StorageError(format!(
                "profile '{profile}' not found"
            )));
        }

        // 2. Delete keyring token entry.
        let key_t = keyring_account_token(profile);
        if let Ok(entry) = keyring::Entry::new(SERVICE, &key_t) {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(OlolError::StorageError(e.to_string())),
            }
        }

        // 3. Delete keyring server_url entry.
        let key_s = keyring_account_server(profile);
        if let Ok(entry) = keyring::Entry::new(SERVICE, &key_s) {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(OlolError::StorageError(e.to_string())),
            }
        }

        // 4. Remove from TOML and persist.
        creds.profiles.remove(profile);
        let content =
            toml::to_string(&creds).map_err(|e| OlolError::StorageError(e.to_string()))?;
        std::fs::write(&path, content).map_err(|e| OlolError::StorageError(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_overrides_all() {
        let _g = env_lock().lock().unwrap();
        unsafe { std::env::set_var("OLOLO_URL", "http://env") };
        let result = resolve_server_url(Some("http://custom".into()), "default");
        unsafe { std::env::remove_var("OLOLO_URL") };
        assert_eq!(result, "http://custom");
    }

    #[test]
    fn env_used_when_no_flag() {
        let _g = env_lock().lock().unwrap();
        unsafe { std::env::remove_var("OLOLO_URL") };
        unsafe { std::env::set_var("OLOLO_URL", "http://env-test") };
        let result = resolve_server_url(None, "default");
        unsafe { std::env::remove_var("OLOLO_URL") };
        assert_eq!(result, "http://env-test");
    }

    #[test]
    fn multi_profile_file_roundtrip() {
        let mut creds = CredentialsFile::default();
        creds.profiles.insert(
            "default".into(),
            ProfileEntry {
                server_url: "https://ololo.dev".into(),
                token: "tok_default".into(),
            },
        );
        creds.profiles.insert(
            "staging".into(),
            ProfileEntry {
                server_url: "https://staging.example.com".into(),
                token: "tok_staging".into(),
            },
        );

        // Serialize to TOML and parse back.
        let content = toml::to_string(&creds).unwrap();
        let loaded: CredentialsFile = toml::from_str(&content).unwrap();

        let def = loaded.profiles.get("default").unwrap();
        assert_eq!(def.token, "tok_default");
        assert_eq!(def.server_url, "https://ololo.dev");

        let sta = loaded.profiles.get("staging").unwrap();
        assert_eq!(sta.token, "tok_staging");
        assert_eq!(sta.server_url, "https://staging.example.com");
    }

    #[test]
    fn profiles_are_sorted_in_output() {
        // BTreeMap ensures stable alphabetical ordering in the TOML file.
        let mut creds = CredentialsFile::default();
        add_sample_profiles(&mut creds, &["zzz", "aaa", "mmm"]);
        let content = toml::to_string(&creds).unwrap();
        let aaa_pos = content.find("[aaa]").unwrap();
        let mmm_pos = content.find("[mmm]").unwrap();
        let zzz_pos = content.find("[zzz]").unwrap();
        assert!(aaa_pos < mmm_pos && mmm_pos < zzz_pos);
    }

    // ── New tests ─────────────────────────────────────────────────────────────

    /// Insert `name` → `https://{name}.example.com` / `tok_{name}` for each
    /// entry. Shared by the alphabetical-ordering and list-roundtrip tests so
    /// the fixture construction stays in one place.
    fn add_sample_profiles(creds: &mut CredentialsFile, names: &[&str]) {
        for name in names {
            creds.profiles.insert(
                (*name).into(),
                ProfileEntry {
                    server_url: format!("https://{name}.example.com"),
                    token: format!("tok_{name}"),
                },
            );
        }
    }

    fn write_credentials(home: &std::path::Path, creds: &CredentialsFile) {
        let dir = home.join(".config").join("ololo");
        std::fs::create_dir_all(&dir).unwrap();
        let content = toml::to_string(creds).unwrap();
        std::fs::write(dir.join("credentials.toml"), content).unwrap();
    }

    #[test]
    fn list_profiles_from_toml_content() {
        let _g = env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap().to_owned();

        let mut creds = CredentialsFile::default();
        add_sample_profiles(&mut creds, &["zzz", "aaa", "mmm"]);
        write_credentials(tmp.path(), &creds);

        let _h = crate::test_util::test_util::HomeGuard::set(&home);
        let result = list_profiles();

        let profiles = result.unwrap();
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0].name, "aaa");
        assert_eq!(profiles[0].server_url, "https://aaa.example.com");
        assert_eq!(profiles[1].name, "mmm");
        assert_eq!(profiles[2].name, "zzz");
    }

    #[test]
    fn delete_nonexistent_profile_returns_error() {
        let _g = env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap().to_owned();

        let mut creds = CredentialsFile::default();
        creds.profiles.insert(
            "prod".into(),
            ProfileEntry {
                server_url: "https://prod.example.com".into(),
                token: "tok_prod".into(),
            },
        );
        write_credentials(tmp.path(), &creds);

        let _h = crate::test_util::test_util::HomeGuard::set(&home);
        let result = Config::delete("staging");

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not found") || msg.contains("staging"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn delete_profile_toml_only() {
        let _g = env_lock().lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap().to_owned();

        let mut creds = CredentialsFile::default();
        creds.profiles.insert(
            "dev".into(),
            ProfileEntry {
                server_url: "https://dev.example.com".into(),
                token: "tok_dev".into(),
            },
        );
        write_credentials(tmp.path(), &creds);

        let _h = crate::test_util::test_util::HomeGuard::set(&home);
        let delete_result = Config::delete("dev");
        let profiles = list_profiles().unwrap();

        assert!(delete_result.is_ok(), "delete returned: {delete_result:?}");
        assert!(
            profiles.iter().all(|p| p.name != "dev"),
            "dev profile still present after delete"
        );
    }
}
