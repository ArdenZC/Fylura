//! Clipboard export/reuse of the existing Transfer ticket registry.
use super::*;

impl FileWorkspaceRuntime {
    pub(crate) fn clipboard_owned_tickets(
        &self,
        ids: &[String],
        paths: &[String],
    ) -> Result<Option<Vec<TransferTicket>>, String> {
        if ids.is_empty() {
            return Ok(None);
        }
        let tickets = {
            let state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            let Some(tickets) = ids
                .iter()
                .map(|id| state.sources.get(id).map(|source| source.ticket.clone()))
                .collect::<Option<Vec<_>>>()
            else {
                return Ok(None);
            };
            tickets
        };
        let expected = tickets
            .iter()
            .map(|ticket| ticket.path.replace('\\', "/"))
            .collect::<HashSet<_>>();
        let actual = paths
            .iter()
            .map(|path| path.replace('\\', "/"))
            .collect::<HashSet<_>>();
        if expected != actual {
            return Ok(None);
        }
        self.clipboard_transfer_paths(ids)?;
        Ok(Some(tickets))
    }

    /// Export only live tickets after namespace identity revalidation. No
    /// transfer registry lock is held across filesystem or clipboard work.
    pub(crate) fn clipboard_transfer_paths(&self, ids: &[String]) -> Result<Vec<String>, String> {
        self.ensure_live()?;
        let sources = {
            let state = self
                .inner
                .transfers
                .lock()
                .map_err(|_| "transfer_state_unavailable")?;
            ids.iter()
                .map(|id| {
                    state
                        .sources
                        .get(id)
                        .cloned()
                        .ok_or_else(|| "transfer_ticket_expired".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut paths = Vec::new();
        for source in sources {
            if file_namespace_fingerprint(Path::new(&source.ticket.path))? != source.identity {
                return Err("transfer_source_changed".into());
            }
            if !paths.contains(&source.ticket.path) {
                paths.push(source.ticket.path);
            }
        }
        Ok(paths)
    }
}
