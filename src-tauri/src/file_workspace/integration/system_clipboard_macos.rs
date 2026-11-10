//! Native file URLs and per-item action metadata, under Fylura's app identity.
use super::ClipboardFiles;
use objc2::{rc::autoreleasepool, runtime::ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSString};

const ACTION_TYPE: &str = "org.fylura.file-transfer.action";

pub(super) fn read(_owner: Option<usize>) -> Result<ClipboardFiles, String> {
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let revision = board.changeCount();
        let file_type = unsafe { NSPasteboardTypeFileURL };
        let action_type = NSString::from_str(ACTION_TYPE);
        let mut paths = Vec::new();
        let mut move_files = None;
        if let Some(items) = board.pasteboardItems() {
            if items.len() > super::MAX_PATHS {
                return Err("workspace_clipboard_invalid_files".into());
            }
            for index in 0..items.len() {
                let item = items.objectAtIndex(index);
                let Some(value) = item.stringForType(file_type) else {
                    continue;
                };
                let url = url::Url::parse(&value.to_string())
                    .map_err(|_| "workspace_clipboard_invalid_files")?;
                let path = url
                    .to_file_path()
                    .map_err(|_| "workspace_clipboard_invalid_files")?;
                let path = path
                    .into_os_string()
                    .into_string()
                    .map_err(|_| "workspace_clipboard_invalid_files")?;
                if path.len() > super::MAX_PATH_LENGTH {
                    return Err("workspace_clipboard_invalid_files".into());
                }
                let action = item
                    .stringForType(&action_type)
                    .is_some_and(|value| value.to_string() == "move");
                if move_files.is_some_and(|previous| previous != action) {
                    return Err("workspace_clipboard_invalid_files".into());
                }
                move_files = Some(action);
                paths.push(path);
            }
        }
        if revision != board.changeCount() {
            return Err("workspace_clipboard_changed".into());
        }
        Ok(ClipboardFiles {
            paths,
            move_files: move_files.unwrap_or(false),
        })
    })
}

pub(super) fn write(files: &ClipboardFiles, _owner: Option<usize>) -> Result<(), String> {
    autoreleasepool(|_| {
        let file_type = unsafe { NSPasteboardTypeFileURL };
        let action_type = NSString::from_str(ACTION_TYPE);
        let action = NSString::from_str(if files.move_files { "move" } else { "copy" });
        let mut items = Vec::with_capacity(files.paths.len());
        for path in &files.paths {
            let url =
                url::Url::from_file_path(path).map_err(|_| "workspace_clipboard_invalid_files")?;
            let url = NSString::from_str(url.as_str());
            let item = NSPasteboardItem::new();
            if !item.setString_forType(&url, file_type)
                || !item.setString_forType(&action, &action_type)
            {
                return Err("workspace_clipboard_unavailable".into());
            }
            items.push(ProtocolObject::<dyn NSPasteboardWriting>::from_retained(
                item,
            ));
        }
        let items = NSArray::from_retained_slice(&items);
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        if !board.writeObjects(&items) {
            return Err("workspace_clipboard_unavailable".into());
        }
        Ok(())
    })
}
