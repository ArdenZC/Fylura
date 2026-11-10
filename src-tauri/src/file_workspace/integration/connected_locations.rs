//! OS/client-exposed paths only; no account login or remote enumeration.
use std::path::PathBuf;

pub(super) fn cloud_roots() -> Vec<(PathBuf, String)> {
    let mut roots: Vec<(PathBuf, String)> = Vec::new();
    #[cfg(windows)]
    {
        for name in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(path) = std::env::var_os(name) {
                roots.push((PathBuf::from(path), "OneDrive".into()));
            }
        }
        // Dropbox's documented local client configuration contains mounted
        // paths. Read at most 64 KiB; do not read credentials or user content.
        for name in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(name) {
                let config = PathBuf::from(root).join("Dropbox/info.json");
                if let Ok(file) = std::fs::File::open(config) {
                    use std::io::Read;
                    let mut bytes = Vec::new();
                    if file.take(65537).read_to_end(&mut bytes).is_ok() && bytes.len() <= 65536 {
                        roots.extend(dropbox_paths(&bytes));
                    }
                }
            }
        }
        // Google Drive virtual drives already come from logical-drive
        // discovery. Custom mirrored folders remain user-selected Browse paths.
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            if let Ok(entries) = std::fs::read_dir(home.join("Library/CloudStorage")) {
                let mut paths = entries
                    .flatten()
                    .take(32)
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>();
                paths.sort();
                roots.extend(paths.into_iter().filter(|path| path.is_dir()).map(|path| {
                    let label = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    (path, label)
                }));
            }
            let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
            if icloud.is_dir() {
                roots.push((icloud, "iCloud Drive".into()));
            }
        }
    }
    roots.retain(|(path, _)| path.is_absolute() && path.to_string_lossy().len() <= 4096);
    roots.truncate(8);
    roots
}

#[cfg(any(windows, test))]
fn dropbox_paths(bytes: &[u8]) -> Vec<(PathBuf, String)> {
    let Ok(serde_json::Value::Object(accounts)) = serde_json::from_slice(bytes) else {
        return Vec::new();
    };
    ["personal", "business"]
        .iter()
        .filter_map(|account| {
            let path = PathBuf::from(accounts.get(*account)?.get("path")?.as_str()?);
            (path.is_absolute()
                && !path.to_string_lossy().contains('\0')
                && path.to_string_lossy().len() <= 4096)
                .then_some((path, "Dropbox".into()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_configuration_is_bounded_to_account_paths_not_credentials_or_arbitrary_fields() {
        let path = std::env::temp_dir().join("fylura-dropbox-projection");
        let bytes = serde_json::to_vec(&serde_json::json!({
            "personal":{"path":path,"token":"never_projected"},
            "business":{"path":"relative"},"unrelated":{"path":"/not-a-client-root"}
        }))
        .unwrap();
        assert_eq!(dropbox_paths(&bytes), vec![(path, "Dropbox".into())]);
        assert!(dropbox_paths(b"not-json").is_empty());
    }
}
