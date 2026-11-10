//! Optional controlled destination for the existing copy profiles. Every
//! comparison uses one generated directory on the same selected target.
use fylura_tauri::fs_safety::VerifiedDirectory;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct NativeCopyTarget {
    path: PathBuf,
    directory: VerifiedDirectory,
}
impl NativeCopyTarget {
    pub fn configured(source: &Path) -> Option<Self> {
        let parent = std::env::var_os("ZC_NATIVE_COPY_TARGET_ROOT")?;
        let parent = PathBuf::from(parent);
        assert!(
            parent.is_absolute() && parent.is_dir(),
            "copy target must be an existing absolute mounted directory"
        );
        let kind =
            std::env::var("ZC_NATIVE_COPY_TARGET_KIND").expect("label controlled target kind");
        assert!(
            matches!(
                kind.as_str(),
                "usb_flash"
                    | "mobile_ssd"
                    | "mobile_hdd"
                    | "sd_card"
                    | "smb"
                    | "cloud_client"
                    | "local"
            ),
            "unsupported copy target label"
        );
        let path = parent.join(format!(
            "fylura-native-copy-profile-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).expect("create exclusive owned copy target");
        let directory = VerifiedDirectory::open_existing(&path).expect("capture target identity");
        let source_directory =
            VerifiedDirectory::open_existing(source).expect("capture source identity");
        println!("native copy storage context kind={kind} sourceVolume={} targetVolume={} crossVolume={} labelOrigin=operator remoteSync=UNVERIFIED cache=uncontrolled",
            source_directory.identity().volume_id, directory.identity().volume_id,
            source_directory.identity().volume_id != directory.identity().volume_id);
        Some(Self { path, directory })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for NativeCopyTarget {
    fn drop(&mut self) {
        // An unplug/replacement must not turn cleanup into deletion on a new
        // mounted volume. Leave the owned fixture behind if identity changed.
        match VerifiedDirectory::open_existing(&self.path) {
            Ok(current) if current.identity() == self.directory.identity() => {
                if let Err(error) = fs::remove_dir_all(&self.path) {
                    eprintln!("native copy fixture cleanup pending: {error}");
                }
            }
            _ => eprintln!(
                "native copy fixture cleanup pending: target identity unavailable or changed"
            ),
        }
    }
}
