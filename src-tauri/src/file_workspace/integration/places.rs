//! Small navigation preferences in the existing SQLite app_settings store.
//! Paths are restore/routing hints only, re-admitted by BrowseService on use.
use super::FileWorkspaceRuntime;
use crate::file_workspace::BrowsePathRef;
use crate::fs_safety::VerifiedDirectory;
use rusqlite::{params, OptionalExtension};
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
        let mut conn = self.inner.database.conn().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
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
        let changed = visit.is_some() || request.remove_id.is_some();
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
