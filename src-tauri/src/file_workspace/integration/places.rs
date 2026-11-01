//! Small navigation preferences in the existing SQLite app_settings store.
//! Paths are restore/routing hints only, re-admitted by BrowseService on use.
use super::FileWorkspaceRuntime;
use crate::file_workspace::BrowsePathRef;
use crate::fs_safety::VerifiedDirectory;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
const KEY: &str = "file_library_places_v1";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPlace {
    pub id: String,
    pub name: String,
    pub routing_hint: String,
    pub favorite: bool,
    pub visited_at: i64,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub current: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlacesRequest {
    pub session_id: Option<String>,
    pub path_ref: Option<BrowsePathRef>,
    pub favorite: Option<bool>,
    pub remove_id: Option<String>,
}
impl FileWorkspaceRuntime {
    pub(crate) fn places(&self, request: PlacesRequest) -> Result<Vec<SavedPlace>, String> {
        self.ensure_live()?;
        let visit =
            if let (Some(session), Some(reference)) = (&request.session_id, &request.path_ref) {
                let path = self
                    .inner
                    .browse
                    .resolve_watch_target(session, reference)
                    .map_err(|e| e.to_string())?;
                let path = path.as_path().canonicalize().map_err(|e| e.to_string())?;
                Some((
                    crate::path_identity::normalize_path(&path),
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                ))
            } else {
                None
            };
        let current_path = visit.as_ref().map(|(path, _)| path.clone());
        let changed = visit.is_some() || request.remove_id.is_some();
        let mut conn = self.inner.database.conn().map_err(|e| e.to_string())?;
        // Browse panes can record their recent locations at the same time.
        // Acquire the write reservation before reading app_settings so those
        // writers serialize instead of racing a deferred read-to-write upgrade
        // and surfacing SQLITE_BUSY to the user.
        let tx = conn
            .transaction_with_behavior(if changed {
                TransactionBehavior::Immediate
            } else {
                TransactionBehavior::Deferred
            })
            .map_err(|e| e.to_string())?;
        let json: Option<String> = tx
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [KEY],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let mut places: Vec<SavedPlace> = match json {
            Some(json) => serde_json::from_str(&json).map_err(|e| e.to_string())?,
            None => Vec::new(),
        };
        if let Some(id) = request.remove_id {
            places.retain(|p| p.id != id);
        }
        if let Some((path, name)) = visit {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs() as i64;
            if let Some(place) = places.iter_mut().find(|p| p.routing_hint == path) {
                place.visited_at = now;
                if let Some(favorite) = request.favorite {
                    place.favorite = favorite;
                }
            } else {
                places.push(SavedPlace {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    routing_hint: path,
                    favorite: request.favorite.unwrap_or(false),
                    visited_at: now,
                    unavailable: false,
                    current: false,
                });
            }
            places.sort_by_key(|p| std::cmp::Reverse(p.visited_at));
            let mut recent = 0;
            places.retain(|p| {
                if p.favorite {
                    true
                } else {
                    recent += 1;
                    recent <= 20
                }
            });
            if places.len() > 100 {
                return Err("places_limit_100".into());
            }
        }
        if changed {
            let value = serde_json::to_string(&places).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO app_settings(key, value, revision) VALUES (?1, ?2, 1) ON CONFLICT(key) DO UPDATE SET value = excluded.value, revision = revision + 1", params![KEY, value]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        // Explicit listing only; no monitoring/polling of favorites or recent paths.
        for place in &mut places {
            place.current = current_path.as_deref() == Some(place.routing_hint.as_str());
            place.unavailable =
                VerifiedDirectory::open_existing(std::path::Path::new(&place.routing_hint))
                    .is_err();
        }
        Ok(places)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        file_workspace::integration::{types::BrowseOpenRequest, FileWorkspaceRuntime},
        platform::macos::quick_look::MacThumbnailService,
    };
    use std::{
        collections::HashSet,
        fs,
        path::PathBuf,
        sync::{Arc, Barrier},
        thread,
    };

    #[test]
    fn parallel_browse_panes_can_record_recent_locations() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("file-workspace-places-{}", uuid::Uuid::new_v4()));
        let pane_paths = [root.join("left-pane"), root.join("right-pane")];
        for path in &pane_paths {
            fs::create_dir_all(path).expect("pane fixture");
        }
        let runtime = FileWorkspaceRuntime::new(
            Database::open(root.join("zen-canvas.sqlite3")).expect("places database"),
            MacThumbnailService::new(root.join("legacy-thumbnail-cache")),
            root.join("thumbnail-cache"),
        )
        .expect("workspace runtime");
        let platform = if cfg!(target_os = "windows") {
            crate::file_workspace::contracts::WorkspacePlatform::Windows
        } else {
            crate::file_workspace::contracts::WorkspacePlatform::Macos
        };
        let opened = pane_paths
            .iter()
            .map(|path| {
                runtime
                    .open_browse(BrowseOpenRequest {
                        platform,
                        routing_hint: path.to_string_lossy().into_owned(),
                        display_hint: Some(
                            path.file_name().unwrap().to_string_lossy().into_owned(),
                        ),
                        global_entry_id: None,
                    })
                    .expect("admit pane fixture")
            })
            .collect::<Vec<_>>();
        let start = Arc::new(Barrier::new(opened.len() + 1));
        let workers = opened
            .into_iter()
            .map(|browse| {
                let runtime = runtime.clone();
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    runtime.places(PlacesRequest {
                        session_id: Some(browse.session_id),
                        path_ref: Some(browse.root_path_ref),
                        favorite: None,
                        remove_id: None,
                    })
                })
            })
            .collect::<Vec<_>>();
        start.wait();
        for worker in workers {
            worker
                .join()
                .expect("places worker")
                .expect("concurrent recent location update");
        }

        let stored = runtime
            .places(PlacesRequest {
                session_id: None,
                path_ref: None,
                favorite: None,
                remove_id: None,
            })
            .expect("read saved places");
        let paths = stored
            .into_iter()
            .map(|place| place.routing_hint)
            .collect::<HashSet<_>>();
        for path in &pane_paths {
            let canonical = path.canonicalize().expect("canonical fixture path");
            assert!(paths.contains(&crate::path_identity::normalize_path(&canonical)));
        }

        drop(runtime);
        fs::remove_dir_all(root).expect("remove places fixture");
    }
}
