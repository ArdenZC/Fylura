use super::format::{ascii_slot, CompactIndexData, StringTableBuilder};
use super::{
    FLAG_DIRECTORY, FLAG_HIDDEN, FLAG_STALE, FLAG_SYSTEM, MAX_INDEX_BYTES, NAME_BITMAP_WIDTH,
    NO_PARENT, NULL_MTIME,
};
use rusqlite::Connection;
use std::io;

impl CompactIndexData {
    pub(super) fn build(conn: &Connection) -> io::Result<Self> {
        let generation: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM compact_name_search_changes",
                [],
                |row| row.get(0),
            )
            .map_err(io::Error::other)?;

        let mut names = StringTableBuilder::default();
        {
            let mut statement = conn
                .prepare(
                    "SELECT DISTINCT name_normalized FROM global_entries
                     ORDER BY name_normalized COLLATE BINARY",
                )
                .map_err(io::Error::other)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(io::Error::other)?;
            for row in rows {
                names
                    .push_sorted(&row.map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            }
        }

        let mut extensions = StringTableBuilder::default();
        {
            let mut statement = conn
                .prepare(
                    "SELECT DISTINCT extension FROM global_entries
                     ORDER BY extension COLLATE BINARY",
                )
                .map_err(io::Error::other)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(io::Error::other)?;
            for row in rows {
                extensions
                    .push_sorted(&row.map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            }
        }

        let mut sources = StringTableBuilder::default();
        {
            let mut statement = conn
                .prepare("SELECT id FROM global_volumes ORDER BY id COLLATE BINARY")
                .map_err(io::Error::other)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(io::Error::other)?;
            for row in rows {
                sources
                    .push_sorted(&row.map_err(io::Error::other)?)
                    .map_err(io::Error::other)?;
            }
        }

        let mut data = Self {
            generation,
            path_relation_complete: true,
            names,
            extensions,
            sources,
            name_posting_offsets: Vec::new(),
            name_postings: Vec::new(),
            name_bitmaps: Vec::new(),
            extension_posting_offsets: Vec::new(),
            extension_postings: Vec::new(),
            id_offsets: Vec::new(),
            id_lengths: Vec::new(),
            id_bytes: Vec::new(),
            entry_name_ids: Vec::new(),
            entry_extension_ids: Vec::new(),
            entry_source_ids: Vec::new(),
            entry_parent_dirs: Vec::new(),
            entry_mtimes: Vec::new(),
            entry_flags: Vec::new(),
            dir_entry_indexes: Vec::new(),
            dir_parent_indexes: Vec::new(),
        };
        data.read_entries(conn)?;
        data.read_parent_links(conn)?;
        data.build_postings_and_bitmaps()?;
        Ok(data)
    }

    fn read_entries(&mut self, conn: &Connection) -> io::Result<()> {
        let sql = r#"
            SELECT id, volume_id, name_normalized, extension, modified_at_fs,
                   is_directory, is_hidden, is_system, is_stale
            FROM global_entries
            WHERE id IS NOT NULL
            ORDER BY id COLLATE BINARY
        "#;
        let mut statement = conn.prepare(sql).map_err(io::Error::other)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)? != 0,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, i64>(7)? != 0,
                    row.get::<_, i64>(8)? != 0,
                ))
            })
            .map_err(io::Error::other)?;
        let mut previous_id: Option<String> = None;
        for row in rows {
            let (id, volume, name, extension, mtime, directory, hidden, system, stale) =
                row.map_err(io::Error::other)?;
            if previous_id
                .as_deref()
                .is_some_and(|previous| previous >= id.as_str())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_entry_id_order_invalid",
                ));
            }
            previous_id = Some(id.clone());
            let name_id = self.names.find(&name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_name_dictionary_miss",
                )
            })?;
            let extension_id = self.extensions.find(&extension).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_extension_dictionary_miss",
                )
            })?;
            let source_id = self.sources.find(&volume).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_source_dictionary_miss",
                )
            })?;
            let id_offset = u32::try_from(self.id_bytes.len()).map_err(io::Error::other)?;
            let id_length = u32::try_from(id.len()).map_err(io::Error::other)?;
            self.id_offsets.push(id_offset);
            self.id_lengths.push(id_length);
            self.id_bytes.extend_from_slice(id.as_bytes());
            self.entry_name_ids.push(name_id);
            self.entry_extension_ids.push(extension_id);
            self.entry_source_ids.push(source_id);
            self.entry_parent_dirs.push(NO_PARENT);
            self.entry_mtimes.push(mtime.unwrap_or(NULL_MTIME));
            self.entry_flags.push(
                (if directory { FLAG_DIRECTORY } else { 0 })
                    | (if hidden { FLAG_HIDDEN } else { 0 })
                    | (if system { FLAG_SYSTEM } else { 0 })
                    | (if stale { FLAG_STALE } else { 0 }),
            );
            if directory {
                self.dir_entry_indexes
                    .push(u32::try_from(self.entry_name_ids.len() - 1).map_err(io::Error::other)?);
            }
        }
        if self.id_bytes.len() as u64 > MAX_INDEX_BYTES {
            return Err(io::Error::other("compact_name_search_id_section_too_large"));
        }
        Ok(())
    }

    fn read_parent_links(&mut self, conn: &Connection) -> io::Result<()> {
        let sql = r#"
            SELECT ge.id, ge.parent_platform_file_id, parent.id, ge.is_directory
            FROM global_entries ge
            LEFT JOIN global_entries parent
              ON ge.parent_platform_file_id <> ''
             AND parent.volume_id = ge.volume_id
             AND parent.platform_file_id = ge.parent_platform_file_id
             AND parent.is_directory = 1
            WHERE ge.id IS NOT NULL
            ORDER BY ge.id COLLATE BINARY
        "#;
        let mut statement = conn.prepare(sql).map_err(io::Error::other)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })
            .map_err(io::Error::other)?;
        let mut entry_index = 0usize;
        let mut directory_index = 0usize;
        let mut previous_id: Option<String> = None;
        for row in rows {
            let (id, parent_platform_id, parent_id, is_directory) =
                row.map_err(io::Error::other)?;
            if previous_id.as_deref() == Some(id.as_str()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_parent_identity_ambiguous",
                ));
            }
            previous_id = Some(id.clone());
            let expected_id = self.entry_id(entry_index);
            if expected_id != id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "compact_name_search_parent_pass_mismatch",
                ));
            }
            let parent_directory = if let Some(parent_id) = parent_id {
                Some(self.find_directory_index(&parent_id).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "compact_name_search_parent_directory_missing",
                    )
                })?)
            } else {
                if !parent_platform_id.is_empty() {
                    self.path_relation_complete = false;
                }
                None
            };
            let parent_index = parent_directory.unwrap_or(NO_PARENT);
            self.entry_parent_dirs[entry_index] = parent_index;
            if is_directory {
                if self.dir_entry_indexes.get(directory_index).copied() != Some(entry_index as u32)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "compact_name_search_directory_order_invalid",
                    ));
                }
                self.dir_parent_indexes.push(parent_index);
                directory_index += 1;
            }
            entry_index += 1;
        }
        if entry_index != self.entry_name_ids.len()
            || directory_index != self.dir_entry_indexes.len()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_parent_pass_incomplete",
            ));
        }
        Ok(())
    }

    pub(super) fn entry_id(&self, index: usize) -> &str {
        let start = self.id_offsets[index] as usize;
        let end = start + self.id_lengths[index] as usize;
        std::str::from_utf8(&self.id_bytes[start..end]).expect("SQLite ID is UTF-8")
    }

    fn find_directory_index(&self, id: &str) -> Option<u32> {
        self.dir_entry_indexes
            .binary_search_by(|entry_index| self.entry_id(*entry_index as usize).cmp(id))
            .ok()
            .and_then(|index| u32::try_from(index).ok())
    }

    fn build_postings_and_bitmaps(&mut self) -> io::Result<()> {
        let entry_count = self.entry_name_ids.len();
        let name_count = self.names.offsets.len();
        let extension_count = self.extensions.offsets.len();
        let mut name_counts = vec![0u32; name_count];
        let mut extension_counts = vec![0u32; extension_count];
        for index in 0..entry_count {
            name_counts[self.entry_name_ids[index] as usize] += 1;
            extension_counts[self.entry_extension_ids[index] as usize] += 1;
        }
        self.name_posting_offsets = prefix_offsets(&name_counts)?;
        self.extension_posting_offsets = prefix_offsets(&extension_counts)?;
        self.name_postings = vec![0; entry_count];
        self.extension_postings = vec![0; entry_count];
        let mut name_cursor = self.name_posting_offsets[..name_count].to_vec();
        let mut extension_cursor = self.extension_posting_offsets[..extension_count].to_vec();
        // Entry records are ID-ordered, so each posting list is stable-ID ordered.
        for entry_index in 0..entry_count {
            let name_id = self.entry_name_ids[entry_index] as usize;
            let name_at = name_cursor[name_id] as usize;
            self.name_postings[name_at] = entry_index as u32;
            name_cursor[name_id] += 1;
            let extension_id = self.entry_extension_ids[entry_index] as usize;
            let extension_at = extension_cursor[extension_id] as usize;
            self.extension_postings[extension_at] = entry_index as u32;
            extension_cursor[extension_id] += 1;
        }
        let blocks = name_count.div_ceil(64);
        self.name_bitmaps = vec![0; blocks * NAME_BITMAP_WIDTH];
        for name_index in 0..name_count {
            let block = name_index / 64;
            let bit = 1u64 << (name_index % 64);
            let start = self.names.offsets[name_index] as usize;
            let end = start + self.names.lengths[name_index] as usize;
            for byte in &self.names.bytes[start..end] {
                if let Some(slot) = ascii_slot(*byte) {
                    self.name_bitmaps[block * NAME_BITMAP_WIDTH + slot] |= bit;
                }
            }
        }
        Ok(())
    }
}

fn prefix_offsets(counts: &[u32]) -> io::Result<Vec<u32>> {
    let mut offsets = Vec::with_capacity(counts.len() + 1);
    offsets.push(0);
    let mut total = 0u32;
    for count in counts {
        total = total.checked_add(*count).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_count_overflow",
            )
        })?;
        offsets.push(total);
    }
    Ok(offsets)
}
