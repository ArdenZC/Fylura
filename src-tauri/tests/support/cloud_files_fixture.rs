//! Task-owned, fully cached Cloud Files objects. No remote account/client,
//! network endpoint or user sync root is used or presented as qualified.
use fylura_tauri::{
    db::Database,
    file_ops::{
        execute_moves_with_persistence, native_qa_transfer_copy_preview,
        native_qa_transfer_move_preview, restore_moves_with_persistence, ExecuteMovesRequest,
        RestoreMovesRequest,
    },
};
use std::{
    fs,
    mem::size_of,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::Storage::CloudFilters::*;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
struct Root {
    path: PathBuf,
    connection: Option<CF_CONNECTION_KEY>,
}
impl Drop for Root {
    fn drop(&mut self) {
        if let Some(connection) = self.connection {
            unsafe {
                CfDisconnectSyncRoot(connection);
            }
        }
        let result = unsafe { CfUnregisterSyncRoot(wide(&self.path).as_ptr()) };
        assert!(
            result >= 0,
            "task-owned Cloud Files root must unregister: {result:#x}"
        );
    }
}
fn capability_unavailable(result: i32) -> bool {
    matches!((result as u32) & 0xffff, 1 | 5 | 50 | 379 | 1062)
}

pub fn verify_cached_copy(db: &Database, source_parent: &Path, target_parent: &Path) {
    let root = source_parent.join("native-cloud-files");
    fs::create_dir(&root).expect("create owned Cloud Files root");
    let provider = "Fylura Native QA\0".encode_utf16().collect::<Vec<_>>();
    let version = "1.0\0".encode_utf16().collect::<Vec<_>>();
    let registration = CF_SYNC_REGISTRATION {
        StructSize: size_of::<CF_SYNC_REGISTRATION>() as u32,
        ProviderName: provider.as_ptr(),
        ProviderVersion: version.as_ptr(),
        ProviderId: windows_sys::core::GUID::from_u128(0xb3bc3b2e_91b1_40c3_b5a6_8be343f0093d),
        ..Default::default()
    };
    let policies = CF_SYNC_POLICIES {
        StructSize: size_of::<CF_SYNC_POLICIES>() as u32,
        Hydration: CF_HYDRATION_POLICY {
            Primary: CF_HYDRATION_POLICY_FULL,
            Modifier: 0,
        },
        Population: CF_POPULATION_POLICY {
            Primary: CF_POPULATION_POLICY_FULL,
            Modifier: 0,
        },
        InSync: CF_INSYNC_POLICY_NONE,
        HardLink: CF_HARDLINK_POLICY_NONE,
        PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
    };
    let result = unsafe {
        CfRegisterSyncRoot(
            wide(&root).as_ptr(),
            &registration,
            &policies,
            CF_REGISTER_FLAG_NONE,
        )
    };
    if result < 0 && capability_unavailable(result) {
        println!("Windows Cloud Files cached Copy=UNVERIFIED reason=driver_or_permission_unavailable hresult={result:#x}");
        return;
    }
    assert!(
        result >= 0,
        "register owned Cloud Files fixture: {result:#x}"
    );
    let mut owner = Root {
        path: root.clone(),
        connection: None,
    };
    let callbacks = [CF_CALLBACK_REGISTRATION {
        Type: CF_CALLBACK_TYPE_NONE,
        Callback: None,
    }];
    let mut key = 0;
    let result = unsafe {
        CfConnectSyncRoot(
            wide(&root).as_ptr(),
            callbacks.as_ptr(),
            ptr::null(),
            CF_CONNECT_FLAG_NONE,
            &mut key,
        )
    };
    assert!(result >= 0, "connect owned no-network fixture: {result:#x}");
    owner.connection = Some(key);
    let source = root.join("cached-cloud-file.txt");
    let payload = b"cached cloud bytes; no provider callback or download";
    fs::write(&source, payload).expect("write cached payload");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&source)
        .expect("open fixture for conversion");
    let identity = b"fylura-native-cached";
    let mut usn = 0;
    let result = unsafe {
        CfConvertToPlaceholder(
            file.as_raw_handle(),
            identity.as_ptr().cast(),
            identity.len() as u32,
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            &mut usn,
            ptr::null_mut(),
        )
    };
    assert!(
        result >= 0,
        "convert cached Cloud Files fixture: {result:#x}"
    );
    drop(file);
    use std::os::windows::fs::MetadataExt;
    let attributes = fs::symlink_metadata(&source).unwrap().file_attributes();
    assert_ne!(attributes & 0x400, 0, "exercise a real reparse object");
    assert_eq!(
        attributes & (0x1000 | 0x40000 | 0x400000),
        0,
        "fixture must be fully cached"
    );
    let destination = target_parent.join("native-cloud-copy-target");
    fs::create_dir(&destination).expect("create copy target");
    let operation =
        native_qa_transfer_copy_preview(&source, &destination).expect("cached cloud Copy preview");
    let result = execute_moves_with_persistence(
        db,
        ExecuteMovesRequest {
            operations: vec![operation],
        },
    )
    .expect("cached cloud Copy");
    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].status, "success", "{:?}", result.logs);
    assert_eq!(fs::read(&source).unwrap(), payload);
    assert_eq!(
        fs::read(destination.join("cached-cloud-file.txt")).unwrap(),
        payload
    );
    assert_eq!(result.logs[0].operation_phase, "completed");
    let incoming = root.join("incoming");
    fs::create_dir(&incoming).expect("create incoming Cloud Files directory");
    use std::os::windows::fs::OpenOptionsExt;
    let directory = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
        .open(&incoming)
        .expect("open directory for conversion");
    let result = unsafe {
        CfConvertToPlaceholder(
            directory.as_raw_handle(),
            identity.as_ptr().cast(),
            identity.len() as u32,
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            &mut usn,
            ptr::null_mut(),
        )
    };
    assert!(result >= 0, "convert cached directory: {result:#x}");
    drop(directory);
    let plain = source_parent.join("plain-to-cloud.txt");
    fs::write(&plain, payload).unwrap();
    let operation =
        native_qa_transfer_copy_preview(&plain, &incoming).expect("incoming Copy preview");
    let result = execute_moves_with_persistence(
        db,
        ExecuteMovesRequest {
            operations: vec![operation],
        },
    )
    .unwrap();
    assert_eq!(result.logs[0].status, "success", "{:?}", result.logs);
    assert_eq!(
        fs::read(incoming.join("plain-to-cloud.txt")).unwrap(),
        payload
    );

    // Same-volume move/restore exercises cached provider namespace changes.
    // Cross-device provider retirement and real client sync stay separately
    // UNVERIFIED unless their controlled native fixtures are available.
    let moved_parent = source_parent.join("native-cloud-move-target");
    fs::create_dir(&moved_parent).unwrap();
    let operation =
        native_qa_transfer_move_preview(&source, &moved_parent).expect("cached cloud Move preview");
    let result = execute_moves_with_persistence(
        db,
        ExecuteMovesRequest {
            operations: vec![operation],
        },
    )
    .unwrap();
    assert_eq!(result.logs[0].status, "success", "{:?}", result.logs);
    assert!(!source.exists());
    let restored =
        restore_moves_with_persistence(db, RestoreMovesRequest { logs: result.logs }).unwrap();
    assert_eq!(restored.restored, 1);
    assert_eq!(fs::read(&source).unwrap(), payload);
    println!("Windows Cloud Files cached Copy/Move=PASS incomingCopy=true sourceRetained=true contentVerified=true journalCompleted=true restore=true remoteClient=UNVERIFIED");
    drop(owner);
}
