//! Explicit, current-user-only application-data cleanup used by native
//! uninstall helpers. Safe Trash contains user files and is always retained.

use std::{fs, path::Path};

#[cfg(any(
    test,
    all(
        any(target_os = "windows", target_os = "macos"),
        not(feature = "native-qa")
    )
))]
use std::path::PathBuf;

const APP_IDENTIFIER: &str = "com.startlan.fylura";
const DATABASE_NAME: &str = "fylura.sqlite3";
const SAFE_TRASH_NAMES: &[&str] = &["safe-trash", ".fylura-trash", ".zen-canvas-trash"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct CleanupResult {
    preserved_safe_trash_roots: usize,
}

fn safe_trash_name(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy();
    SAFE_TRASH_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn clean_owned_root(root: &Path) -> Result<CleanupResult, String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CleanupResult {
                preserved_safe_trash_roots: 0,
            });
        }
        Err(_) => return Err("could not inspect a Fylura data directory".to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(
            "a Fylura data directory is not a real directory; it was preserved".to_string(),
        );
    }

    let mut preserved = 0;
    clean_directory_contents(root, &mut preserved)?;
    if preserved == 0 {
        fs::remove_dir(root).map_err(|_| "could not remove a Fylura data directory".to_string())?;
    }
    verify_only_safe_trash_remains(root)?;
    Ok(CleanupResult {
        preserved_safe_trash_roots: preserved,
    })
}

fn clean_directory_contents(directory: &Path, preserved: &mut usize) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|_| "could not inspect a Fylura data directory".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "could not inspect a Fylura data item".to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if safe_trash_name(&name) {
            *preserved += 1;
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not inspect a Fylura data item".to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            clean_directory_contents(&path, preserved)?;
            match fs::remove_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(_) => return Err("could not remove a Fylura data item".to_string()),
            }
        } else {
            fs::remove_file(&path)
                .map_err(|_| "could not remove a Fylura data item".to_string())?;
        }
    }
    Ok(())
}

fn verify_only_safe_trash_remains(root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("could not verify Fylura data cleanup".to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("could not verify Fylura data cleanup".to_string());
    }
    let entries =
        fs::read_dir(root).map_err(|_| "could not verify Fylura data cleanup".to_string())?;
    for entry in entries {
        let entry = entry.map_err(|_| "could not verify Fylura data cleanup".to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        if safe_trash_name(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| "could not verify Fylura data cleanup".to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            verify_only_safe_trash_remains(&path)?;
        } else {
            return Err("Fylura data cleanup left an unexpected item".to_string());
        }
    }
    Ok(())
}

#[cfg(all(
    any(target_os = "windows", target_os = "macos"),
    not(feature = "native-qa")
))]
fn application_data_roots() -> Result<(PathBuf, PathBuf), String> {
    let data_root = dirs::data_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "Fylura user data directory is unavailable".to_string())?;
    let cache_root = dirs::cache_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "Fylura cache directory is unavailable".to_string())?;
    Ok((
        data_root.join(APP_IDENTIFIER),
        cache_root.join(APP_IDENTIFIER),
    ))
}

#[cfg(all(
    any(target_os = "windows", target_os = "macos"),
    not(feature = "native-qa")
))]
pub fn clean_current_user_data() -> Result<String, String> {
    use crate::ai::settings::{
        clear_current_ai_credentials_for_profiles, current_custom_profile_ids,
        SystemCredentialStore,
    };

    let (data_root, cache_root) = application_data_roots()?;
    let database_path = data_root.join(DATABASE_NAME);
    if let Ok(metadata) = fs::symlink_metadata(&database_path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Fylura database is not a regular file; cleanup was stopped".to_string());
        }
    }
    let profile_ids = current_custom_profile_ids(&database_path)?;

    #[cfg(target_os = "macos")]
    let logs_result = clean_macos_preferences_and_launch_agent()?;

    clear_current_ai_credentials_for_profiles(&SystemCredentialStore, &profile_ids)?;
    let data_result = clean_owned_root(&data_root)?;
    let cache_result = clean_owned_root(&cache_root)?;
    #[cfg(not(target_os = "macos"))]
    let logs_result = CleanupResult {
        preserved_safe_trash_roots: 0,
    };

    #[cfg(target_os = "macos")]
    let removed_scopes = "database/cache/logs/preferences/login-item=removed";
    #[cfg(target_os = "windows")]
    let removed_scopes = "database/cache/logs=removed";
    Ok(format!(
        "Fylura data cleanup verified; credentials=removed; {removed_scopes}; Safe Trash roots preserved={}",
        data_result.preserved_safe_trash_roots
            + cache_result.preserved_safe_trash_roots
            + logs_result.preserved_safe_trash_roots
    ))
}

#[cfg(target_os = "macos")]
fn clean_macos_preferences_and_launch_agent() -> Result<CleanupResult, String> {
    let home =
        dirs::home_dir().ok_or_else(|| "Fylura user home directory is unavailable".to_string())?;
    let preferences = home
        .join("Library")
        .join("Preferences")
        .join("com.startlan.fylura.plist");
    for name in ["Fylura.plist", "com.startlan.fylura.plist"] {
        remove_current_macos_launch_agent(&home, name)?;
    }
    remove_exact_regular_file(&preferences, "Fylura preferences")?;
    let logs = home.join("Library").join("Logs").join(APP_IDENTIFIER);
    clean_owned_root(&logs)
}

#[cfg(any(target_os = "macos", test))]
fn remove_exact_regular_file(path: &Path, item: &str) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(format!("could not inspect {item}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{item} was not a regular file and was preserved"));
    }
    fs::remove_file(path).map_err(|_| format!("could not remove {item}"))?;
    if path.exists() {
        return Err(format!("{item} remains after cleanup"));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn is_owned_macos_launch_agent(name: &str, document: &plist::Value, executable: &Path) -> bool {
    let Some(dictionary) = document.as_dictionary() else {
        return false;
    };
    let label = dictionary.get("Label").and_then(plist::Value::as_string);
    let expected_label = match name {
        "Fylura.plist" => "Fylura",
        "com.startlan.fylura.plist" => "com.startlan.fylura",
        _ => return false,
    };
    let Some(arguments) = dictionary
        .get("ProgramArguments")
        .and_then(plist::Value::as_array)
    else {
        return false;
    };
    let Some(program) = arguments.first().and_then(plist::Value::as_string) else {
        return false;
    };
    label == Some(expected_label)
        && fs::canonicalize(program)
            .map(|program| program == executable)
            .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn remove_current_macos_launch_agent(home: &Path, name: &str) -> Result<(), String> {
    use std::process::Command;

    let path = home.join("Library").join("LaunchAgents").join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("could not inspect Fylura login item".to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Fylura login item was not a regular file and was preserved".to_string());
    }
    let document = plist::Value::from_file(&path)
        .map_err(|_| "could not read Fylura login item".to_string())?;
    let current_executable = std::env::current_exe()
        .map_err(|_| "could not verify the Fylura login item executable".to_string())?;
    let current_executable = fs::canonicalize(&current_executable)
        .map_err(|_| "could not verify the Fylura login item executable".to_string())?;
    if !is_owned_macos_launch_agent(name, &document, &current_executable) {
        return Err("Fylura login item was not recognized and was preserved".to_string());
    }

    let label = document
        .as_dictionary()
        .and_then(|dictionary| dictionary.get("Label"))
        .and_then(plist::Value::as_string)
        .ok_or_else(|| "Fylura login item was not recognized and was preserved".to_string())?;
    let loaded = Command::new("/bin/launchctl")
        .arg("list")
        .output()
        .map_err(|_| "could not inspect the Fylura login item state".to_string())?;
    if !loaded.status.success() {
        return Err("could not determine whether the Fylura login item is loaded".to_string());
    }
    let is_loaded = String::from_utf8_lossy(&loaded.stdout)
        .lines()
        .any(|line| line.split_whitespace().last() == Some(label));
    if is_loaded {
        let uid = unsafe { libc::geteuid() };
        let bootout = Command::new("/bin/launchctl")
            .args([
                "bootout",
                &format!("gui/{uid}"),
                path.to_str()
                    .ok_or_else(|| "Fylura login item path is invalid".to_string())?,
            ])
            .output()
            .map_err(|_| "could not stop the Fylura login item".to_string())?;
        if !bootout.status.success() {
            return Err("could not stop the Fylura login item; it was preserved".to_string());
        }
        let still_loaded = Command::new("/bin/launchctl")
            .arg("list")
            .output()
            .map_err(|_| "could not verify the Fylura login item stopped".to_string())?;
        if !still_loaded.status.success() {
            return Err(
                "could not verify the Fylura login item stopped; its file was preserved"
                    .to_string(),
            );
        }
        if String::from_utf8_lossy(&still_loaded.stdout)
            .lines()
            .any(|line| line.split_whitespace().last() == Some(label))
        {
            return Err("Fylura login item is still loaded; its file was preserved".to_string());
        }
    }
    remove_exact_regular_file(&path, "Fylura login item")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct TestCredentials {
        main: Mutex<Option<String>>,
        profiles: Mutex<std::collections::HashMap<String, String>>,
    }

    impl crate::ai::settings::CredentialStore for TestCredentials {
        fn set(&self, value: &str) -> Result<(), String> {
            *self.main.lock().unwrap() = Some(value.to_string());
            Ok(())
        }
        fn get(&self) -> Result<Option<String>, String> {
            Ok(self.main.lock().unwrap().clone())
        }
        fn delete(&self) -> Result<(), String> {
            *self.main.lock().unwrap() = None;
            Ok(())
        }
        fn set_profile(&self, profile_id: &str, value: &str) -> Result<(), String> {
            self.profiles
                .lock()
                .unwrap()
                .insert(profile_id.to_string(), value.to_string());
            Ok(())
        }
        fn get_profile(&self, profile_id: &str) -> Result<Option<String>, String> {
            Ok(self.profiles.lock().unwrap().get(profile_id).cloned())
        }
        fn delete_profile(&self, profile_id: &str) -> Result<(), String> {
            self.profiles.lock().unwrap().remove(profile_id);
            Ok(())
        }
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "fylura-uninstall-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn explicit_cleanup_removes_only_current_app_data_and_preserves_safe_trash() {
        let profile = test_root("profile");
        let data_root = profile.join("Application Support").join(APP_IDENTIFIER);
        let cache_root = profile.join("Caches").join(APP_IDENTIFIER);
        let unrelated = profile.join("Documents").join("keep.txt");
        fs::create_dir_all(data_root.join("safe-trash").join("batch")).unwrap();
        fs::create_dir_all(cache_root.join("logs")).unwrap();
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(data_root.join(DATABASE_NAME), b"database sentinel").unwrap();
        fs::write(
            data_root
                .join("safe-trash")
                .join("batch")
                .join("real-file.txt"),
            b"user file",
        )
        .unwrap();
        fs::write(cache_root.join("logs").join("app.log"), b"log sentinel").unwrap();
        fs::write(&unrelated, b"unrelated sentinel").unwrap();

        let result = clean_owned_root(&data_root).unwrap();
        assert_eq!(result.preserved_safe_trash_roots, 1);
        clean_owned_root(&cache_root).unwrap();

        assert!(!data_root.join(DATABASE_NAME).exists());
        assert!(data_root
            .join("safe-trash")
            .join("batch")
            .join("real-file.txt")
            .exists());
        assert!(!cache_root.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"unrelated sentinel");
        let _ = fs::remove_dir_all(profile);
    }

    #[test]
    fn cleanup_refuses_a_symlinked_app_data_root() {
        let parent = test_root("symlink");
        let target = parent.join("target");
        let root = parent.join(APP_IDENTIFIER);
        fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &root).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&target, &root).unwrap();

        assert!(clean_owned_root(&root).is_err());
        assert!(target.is_dir());
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn cleanup_reads_all_profile_ids_before_deleting_current_credentials() {
        use crate::ai::settings::{
            clear_current_ai_credentials_for_profiles, current_custom_profile_ids, CredentialStore,
        };

        let root = test_root("credentials");
        fs::create_dir_all(&root).unwrap();
        let database = root.join(DATABASE_NAME);
        let conn = rusqlite::Connection::open(&database).unwrap();
        conn.execute(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![
                crate::ai::settings::AI_SETTINGS_KEY,
                r#"{"customProfiles":[{"id":"alpha"},{"id":"beta"}]}"#,
            ],
        )
        .unwrap();
        drop(conn);

        let ids =
            current_custom_profile_ids(&database).expect("read profiles before database removal");
        assert_eq!(ids, vec!["alpha", "beta"]);
        let credentials = TestCredentials::default();
        credentials.set("main-key").unwrap();
        credentials.set_profile("alpha", "alpha-key").unwrap();
        credentials.set_profile("beta", "beta-key").unwrap();
        clear_current_ai_credentials_for_profiles(&credentials, &ids).unwrap();
        assert!(credentials.get().unwrap().is_none());
        assert!(credentials.get_profile("alpha").unwrap().is_none());
        assert!(credentials.get_profile("beta").unwrap().is_none());

        credentials.set("main-key").unwrap();
        assert!(clear_current_ai_credentials_for_profiles(
            &credentials,
            &["../foreign".to_string()]
        )
        .is_err());
        assert_eq!(credentials.get().unwrap().as_deref(), Some("main-key"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn macos_cleanup_matches_only_the_current_executable_launch_agent() {
        let executable = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "Label".to_string(),
            plist::Value::String("Fylura".to_string()),
        );
        dictionary.insert(
            "ProgramArguments".to_string(),
            plist::Value::Array(vec![
                plist::Value::String(executable.to_string_lossy().into_owned()),
                plist::Value::String("--background".to_string()),
            ]),
        );
        let document = plist::Value::Dictionary(dictionary.clone());
        assert!(is_owned_macos_launch_agent(
            "Fylura.plist",
            &document,
            &executable
        ));
        assert!(!is_owned_macos_launch_agent(
            "com.startlan.fylura.plist",
            &document,
            &executable
        ));

        dictionary.insert(
            "Label".to_string(),
            plist::Value::String("Other App".to_string()),
        );
        let foreign_label = plist::Value::Dictionary(dictionary);
        assert!(!is_owned_macos_launch_agent(
            "Fylura.plist",
            &foreign_label,
            &executable
        ));

        let other_executable = executable.with_file_name("other-app");
        let mut foreign = plist::Dictionary::new();
        foreign.insert(
            "Label".to_string(),
            plist::Value::String("Fylura".to_string()),
        );
        foreign.insert(
            "ProgramArguments".to_string(),
            plist::Value::Array(vec![plist::Value::String(
                other_executable.to_string_lossy().into_owned(),
            )]),
        );
        assert!(!is_owned_macos_launch_agent(
            "Fylura.plist",
            &plist::Value::Dictionary(foreign),
            &executable,
        ));
    }

    #[test]
    fn macos_exact_file_cleanup_removes_only_the_requested_regular_file() {
        let root = test_root("macos-exact-file");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("com.startlan.fylura.plist");
        fs::write(&target, b"Fylura plist fixture").unwrap();
        remove_exact_regular_file(&target, "Fylura preferences").unwrap();
        assert!(!target.exists());
        let _ = fs::remove_dir_all(root);
    }
}
