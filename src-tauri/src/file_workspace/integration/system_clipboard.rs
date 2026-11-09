//! Explicit user clipboard intent. OS paths are re-admitted through Transfer;
//! exporting accepts only existing identity-bound tickets, never renderer paths.
use super::FileWorkspaceRuntime;
use crate::file_ops::manual_transfer::{CaptureRequest, TransferAction, TransferTicket};
use serde::{Deserialize, Serialize};

const MAX_PATHS: usize = crate::file_ops::manual_transfer::LIMIT;
const MAX_PATH_LENGTH: usize = 4096;

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SystemClipboardRequest {
    Read {
        #[serde(default)]
        ticket_ids: Vec<String>,
    },
    Write {
        ticket_ids: Vec<String>,
        move_files: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemClipboardTransfer {
    tickets: Vec<TransferTicket>,
    action: TransferAction,
    owned: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClipboardFiles {
    paths: Vec<String>,
    move_files: bool,
}

impl ClipboardFiles {
    fn validate(&self) -> Result<(), String> {
        if self.paths.len() > MAX_PATHS
            || self.paths.iter().any(|path| {
                path.is_empty()
                    || path.len() > MAX_PATH_LENGTH
                    || path.contains('\0')
                    || !std::path::Path::new(path).is_absolute()
            })
        {
            return Err("workspace_clipboard_invalid_files".into());
        }
        Ok(())
    }
}

impl FileWorkspaceRuntime {
    pub(crate) fn system_clipboard(
        &self,
        request: SystemClipboardRequest,
        owner: Option<usize>,
    ) -> Result<SystemClipboardTransfer, String> {
        self.ensure_live()?;
        match request {
            SystemClipboardRequest::Read { ticket_ids } => {
                if ticket_ids.len() > MAX_PATHS {
                    return Err("workspace_clipboard_invalid_files".into());
                }
                let files = native::read(owner)?;
                files.validate()?;
                if let Some(tickets) = self.clipboard_owned_tickets(&ticket_ids, &files.paths)? {
                    return Ok(SystemClipboardTransfer {
                        tickets,
                        owned: true,
                        action: if files.move_files {
                            TransferAction::Move
                        } else {
                            TransferAction::Copy
                        },
                    });
                }
                if files.paths.len() > crate::file_ops::manual_transfer::EXTERNAL_PATH_LIMIT {
                    return Err("workspace_clipboard_external_limit".into());
                }
                let tickets = self.capture_transfer(CaptureRequest {
                    refs: Vec::new(),
                    selection: None,
                    release_ids: Vec::new(),
                    external_paths: files.paths,
                })?;
                Ok(SystemClipboardTransfer {
                    tickets,
                    owned: false,
                    action: if files.move_files {
                        TransferAction::Move
                    } else {
                        TransferAction::Copy
                    },
                })
            }
            SystemClipboardRequest::Write {
                ticket_ids,
                move_files,
            } => {
                if ticket_ids.is_empty() || ticket_ids.len() > MAX_PATHS {
                    return Err("workspace_clipboard_invalid_files".into());
                }
                let files = ClipboardFiles {
                    paths: self.clipboard_transfer_paths(&ticket_ids)?,
                    move_files,
                };
                files.validate()?;
                native::write(&files, owner)?;
                Ok(SystemClipboardTransfer {
                    tickets: Vec::new(),
                    owned: false,
                    action: if move_files {
                        TransferAction::Move
                    } else {
                        TransferAction::Copy
                    },
                })
            }
        }
    }
}

// Native adapters handle standard OS file formats without reading file content.
#[cfg(target_os = "windows")]
#[path = "system_clipboard_windows.rs"]
mod native;

#[cfg(target_os = "macos")]
#[path = "system_clipboard_macos.rs"]
mod native;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod native {
    use super::ClipboardFiles;
    pub(super) fn read(_owner: Option<usize>) -> Result<ClipboardFiles, String> {
        Err("workspace_clipboard_platform_unsupported".into())
    }
    pub(super) fn write(_: &ClipboardFiles, _owner: Option<usize>) -> Result<(), String> {
        Err("workspace_clipboard_platform_unsupported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_request_cannot_export_renderer_paths() {
        assert!(serde_json::from_value::<SystemClipboardRequest>(serde_json::json!({"kind":"write","ticketIds":[],"moveFiles":false,"paths":["/tmp/forbidden"]})).is_err());
    }
    #[test]
    fn validates_bounded_absolute_file_lists() {
        assert!(ClipboardFiles {
            paths: vec!["relative".into()],
            move_files: false
        }
        .validate()
        .is_err());
        assert!(ClipboardFiles {
            paths: vec!["x".repeat(4097)],
            move_files: false
        }
        .validate()
        .is_err());
        assert!(ClipboardFiles {
            paths: vec![std::env::temp_dir().to_string_lossy().into_owned(); 1001],
            move_files: false
        }
        .validate()
        .is_err());
        assert!(ClipboardFiles {
            paths: vec![std::env::temp_dir().to_string_lossy().into_owned()],
            move_files: false
        }
        .validate()
        .is_ok());
    }
}
