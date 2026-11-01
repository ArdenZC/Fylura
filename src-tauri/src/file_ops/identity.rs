use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileIdentityFingerprint {
    pub(crate) size: u64,
    pub(crate) modified_ns: Option<i128>,
    pub(crate) platform_volume_id: Option<String>,
    pub(crate) platform_file_id: Option<String>,
    pub(crate) quick_hash: Option<String>,
    pub(crate) full_hash: Option<String>,
}

pub(crate) fn file_identity_fingerprint(path: &Path) -> Result<FileIdentityFingerprint, String> {
    let identity =
        crate::fs_safety::capture_identity(path, None).map_err(|error| error.to_string())?;
    Ok(FileIdentityFingerprint {
        size: identity.size,
        modified_ns: identity.modified_ns,
        platform_volume_id: identity.platform_volume_id,
        platform_file_id: identity.platform_file_id,
        quick_hash: identity.sample_hash,
        full_hash: identity.full_hash,
    })
}

pub(crate) fn file_namespace_fingerprint(path: &Path) -> Result<FileIdentityFingerprint, String> {
    let identity = crate::fs_safety::capture_namespace_identity_only(path, None)
        .map_err(|error| error.to_string())?;
    Ok(FileIdentityFingerprint {
        size: identity.size,
        modified_ns: identity.modified_ns,
        platform_volume_id: identity.platform_volume_id,
        platform_file_id: identity.platform_file_id,
        quick_hash: None,
        full_hash: None,
    })
}

pub(crate) fn file_operation_fingerprint(
    path: &Path,
    operation_type: &str,
) -> Result<FileIdentityFingerprint, String> {
    let _ = operation_type;
    if path
        .symlink_metadata()
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return file_namespace_fingerprint(path);
    }
    // macOS clone/copy/replace selection must not pre-read a sparse or
    // provider-backed source just to manufacture a preview hash. The native
    // copy policy chooses a physical clone from namespace identity and falls
    // back to a single streaming pass only when the clone syscall is not
    // available. Other platforms retain the existing content-fingerprint
    // contract.
    if cfg!(target_os = "macos") {
        file_namespace_fingerprint(path)
    } else {
        file_identity_fingerprint(path)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        file_identity_fingerprint, file_namespace_fingerprint, file_operation_fingerprint,
    };
    use std::fs;

    #[test]
    fn namespace_fingerprint_has_no_content_hash() {
        let root = std::env::temp_dir().join(format!(
            "zen-canvas-file-op-identity-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture");
        let path = root.join("file.txt");
        fs::write(&path, b"content fingerprint fixture").expect("file");

        let namespace = file_namespace_fingerprint(&path).expect("namespace fingerprint");
        assert!(namespace.quick_hash.is_none());
        assert!(namespace.full_hash.is_none());

        let content = file_identity_fingerprint(&path).expect("content fingerprint");
        assert!(content.quick_hash.is_some());
        assert!(content.full_hash.is_some());

        let operation = file_operation_fingerprint(&path, "move").expect("operation fingerprint");
        if cfg!(target_os = "macos") {
            assert!(operation.quick_hash.is_none());
            assert!(operation.full_hash.is_none());
        } else {
            assert!(operation.quick_hash.is_some());
            assert!(operation.full_hash.is_some());
        }

        let copy_operation =
            file_operation_fingerprint(&path, "copy").expect("copy operation fingerprint");
        if cfg!(target_os = "macos") {
            assert!(copy_operation.quick_hash.is_none());
            assert!(copy_operation.full_hash.is_none());
        } else {
            assert!(copy_operation.quick_hash.is_some());
            assert!(copy_operation.full_hash.is_some());
        }

        let _ = fs::remove_dir_all(root);
    }
}
