use memmap2::{Mmap, MmapOptions};
use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{
    FLAG_DIRECTORY, FORMAT_VERSION, HEADER_SIZE, MAGIC, MAX_INDEX_BYTES, NAME_BITMAP_WIDTH,
    NO_MATCH, NO_PARENT, NULL_MTIME, SECTION_COUNT,
};

#[derive(Default)]
pub(super) struct StringTableBuilder {
    pub(super) offsets: Vec<u32>,
    pub(super) lengths: Vec<u32>,
    pub(super) bytes: Vec<u8>,
}

impl StringTableBuilder {
    pub(super) fn push_sorted(&mut self, value: &str) -> io::Result<()> {
        if self
            .offsets
            .last()
            .is_some_and(|_| self.get(self.offsets.len() - 1).as_bytes() >= value.as_bytes())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_string_table_not_sorted_unique",
            ));
        }
        let offset = u32::try_from(self.bytes.len()).map_err(io::Error::other)?;
        let length = u32::try_from(value.len()).map_err(io::Error::other)?;
        self.offsets.push(offset);
        self.lengths.push(length);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub(super) fn get(&self, index: usize) -> &str {
        let start = self.offsets[index] as usize;
        let end = start + self.lengths[index] as usize;
        std::str::from_utf8(&self.bytes[start..end]).expect("SQLite text is UTF-8")
    }

    pub(super) fn find(&self, value: &str) -> Option<u32> {
        let mut low = 0usize;
        let mut high = self.offsets.len();
        while low < high {
            let middle = low + (high - low) / 2;
            match self.get(middle).as_bytes().cmp(value.as_bytes()) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return u32::try_from(middle).ok(),
            }
        }
        None
    }
}

pub(super) struct CompactIndexData {
    pub(super) generation: i64,
    pub(super) path_relation_complete: bool,
    pub(super) names: StringTableBuilder,
    pub(super) extensions: StringTableBuilder,
    pub(super) sources: StringTableBuilder,
    pub(super) name_posting_offsets: Vec<u32>,
    pub(super) name_postings: Vec<u32>,
    pub(super) name_bitmaps: Vec<u64>,
    pub(super) extension_posting_offsets: Vec<u32>,
    pub(super) extension_postings: Vec<u32>,
    pub(super) id_offsets: Vec<u32>,
    pub(super) id_lengths: Vec<u32>,
    pub(super) id_bytes: Vec<u8>,
    pub(super) entry_name_ids: Vec<u32>,
    pub(super) entry_extension_ids: Vec<u32>,
    pub(super) entry_source_ids: Vec<u32>,
    pub(super) entry_parent_dirs: Vec<u32>,
    pub(super) entry_mtimes: Vec<i64>,
    pub(super) entry_flags: Vec<u8>,
    pub(super) dir_entry_indexes: Vec<u32>,
    pub(super) dir_parent_indexes: Vec<u32>,
}

pub(super) fn ascii_slot(byte: u8) -> Option<usize> {
    match byte {
        b'a'..=b'z' => Some((byte - b'a') as usize),
        b'0'..=b'9' => Some(26 + (byte - b'0') as usize),
        _ => None,
    }
}

pub(super) fn write_index_file(path: &Path, data: &CompactIndexData) -> io::Result<()> {
    if data.generation < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "compact_name_search_generation_negative",
        ));
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".{}.tmp", Uuid::new_v4()));
    let temporary = PathBuf::from(temporary);
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "compact_name_search_temp_open path={} error={error}",
                    temporary.display()
                ),
            )
        })?;
    file.write_all(&vec![0u8; HEADER_SIZE])?;
    let mut offsets = [0u64; SECTION_COUNT];
    let mut checksum = blake3::Hasher::new();

    offsets[0] = write_u32_section(&mut file, &mut checksum, &data.names.offsets)?;
    offsets[1] = write_u32_section(&mut file, &mut checksum, &data.names.lengths)?;
    offsets[2] = write_bytes_section(&mut file, &mut checksum, &data.names.bytes)?;
    offsets[3] = write_u32_section(&mut file, &mut checksum, &data.name_posting_offsets)?;
    offsets[4] = write_u32_section(&mut file, &mut checksum, &data.name_postings)?;
    offsets[5] = write_u64_section(&mut file, &mut checksum, &data.name_bitmaps)?;
    offsets[6] = write_u32_section(&mut file, &mut checksum, &data.extensions.offsets)?;
    offsets[7] = write_u32_section(&mut file, &mut checksum, &data.extensions.lengths)?;
    offsets[8] = write_bytes_section(&mut file, &mut checksum, &data.extensions.bytes)?;
    offsets[9] = write_u32_section(&mut file, &mut checksum, &data.extension_posting_offsets)?;
    offsets[10] = write_u32_section(&mut file, &mut checksum, &data.extension_postings)?;
    offsets[11] = write_u32_section(&mut file, &mut checksum, &data.id_offsets)?;
    offsets[12] = write_u32_section(&mut file, &mut checksum, &data.id_lengths)?;
    offsets[13] = write_bytes_section(&mut file, &mut checksum, &data.id_bytes)?;
    offsets[14] = write_u32_section(&mut file, &mut checksum, &data.entry_name_ids)?;
    offsets[15] = write_u32_section(&mut file, &mut checksum, &data.entry_extension_ids)?;
    offsets[16] = write_u32_section(&mut file, &mut checksum, &data.entry_source_ids)?;
    offsets[17] = write_u32_section(&mut file, &mut checksum, &data.entry_parent_dirs)?;
    offsets[18] = write_i64_section(&mut file, &mut checksum, &data.entry_mtimes)?;
    offsets[19] = write_bytes_section(&mut file, &mut checksum, &data.entry_flags)?;
    offsets[20] = write_u32_section(&mut file, &mut checksum, &data.sources.offsets)?;
    offsets[21] = write_u32_section(&mut file, &mut checksum, &data.sources.lengths)?;
    offsets[22] = write_bytes_section(&mut file, &mut checksum, &data.sources.bytes)?;
    offsets[23] = write_u32_section(&mut file, &mut checksum, &data.dir_entry_indexes)?;
    offsets[24] = write_u32_section(&mut file, &mut checksum, &data.dir_parent_indexes)?;
    let file_length = file.stream_position()?;
    if file_length > MAX_INDEX_BYTES {
        let _ = fs::remove_file(&temporary);
        return Err(io::Error::other("compact_name_search_index_too_large"));
    }
    let digest = *checksum.finalize().as_bytes();
    let header = encode_header(
        data.generation,
        HeaderCounts {
            entries: u32::try_from(data.entry_name_ids.len()).map_err(io::Error::other)?,
            names: u32::try_from(data.names.offsets.len()).map_err(io::Error::other)?,
            extensions: u32::try_from(data.extensions.offsets.len()).map_err(io::Error::other)?,
            sources: u32::try_from(data.sources.offsets.len()).map_err(io::Error::other)?,
            directories: u32::try_from(data.dir_entry_indexes.len()).map_err(io::Error::other)?,
            flags: u32::from(data.path_relation_complete),
        },
        offsets,
        digest,
    );
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "compact_name_search_index_rename temp={} destination={} temp_exists={} parent_exists={} error={error}",
                temporary.display(),
                path.display(),
                temporary.exists(),
                path.parent().is_some_and(Path::exists)
            ),
        )
    })?;
    Ok(())
}

struct HeaderCounts {
    entries: u32,
    names: u32,
    extensions: u32,
    sources: u32,
    directories: u32,
    flags: u32,
}

fn encode_header(
    generation: i64,
    counts: HeaderCounts,
    offsets: [u64; SECTION_COUNT],
    digest: [u8; 32],
) -> [u8; HEADER_SIZE] {
    let mut header = [0u8; HEADER_SIZE];
    let mut at = 0;
    header[at..at + 8].copy_from_slice(MAGIC);
    at += 8;
    put_u32(&mut header, &mut at, FORMAT_VERSION);
    header[at..at + 8].copy_from_slice(&generation.to_le_bytes());
    at += 8;
    for value in [
        counts.entries,
        counts.names,
        counts.extensions,
        counts.sources,
        counts.directories,
        counts.flags,
    ] {
        put_u32(&mut header, &mut at, value);
    }
    for offset in offsets {
        header[at..at + 8].copy_from_slice(&offset.to_le_bytes());
        at += 8;
    }
    header[at..at + 32].copy_from_slice(&digest);
    header
}

fn put_u32(buffer: &mut [u8], at: &mut usize, value: u32) {
    buffer[*at..*at + 4].copy_from_slice(&value.to_le_bytes());
    *at += 4;
}

fn write_bytes_section(
    file: &mut File,
    hash: &mut blake3::Hasher,
    bytes: &[u8],
) -> io::Result<u64> {
    let offset = file.stream_position()?;
    file.write_all(bytes)?;
    hash.update(bytes);
    Ok(offset)
}

fn write_u32_section(
    file: &mut File,
    hash: &mut blake3::Hasher,
    values: &[u32],
) -> io::Result<u64> {
    let offset = file.stream_position()?;
    let mut buffer = Vec::with_capacity(64 * 1024);
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
        if buffer.len() >= 64 * 1024 {
            file.write_all(&buffer)?;
            hash.update(&buffer);
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        file.write_all(&buffer)?;
        hash.update(&buffer);
    }
    Ok(offset)
}

fn write_u64_section(
    file: &mut File,
    hash: &mut blake3::Hasher,
    values: &[u64],
) -> io::Result<u64> {
    let offset = file.stream_position()?;
    let mut buffer = Vec::with_capacity(64 * 1024);
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
        if buffer.len() >= 64 * 1024 {
            file.write_all(&buffer)?;
            hash.update(&buffer);
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        file.write_all(&buffer)?;
        hash.update(&buffer);
    }
    Ok(offset)
}

fn write_i64_section(
    file: &mut File,
    hash: &mut blake3::Hasher,
    values: &[i64],
) -> io::Result<u64> {
    let offset = file.stream_position()?;
    let mut buffer = Vec::with_capacity(64 * 1024);
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
        if buffer.len() >= 64 * 1024 {
            file.write_all(&buffer)?;
            hash.update(&buffer);
            buffer.clear();
        }
    }
    if !buffer.is_empty() {
        file.write_all(&buffer)?;
        hash.update(&buffer);
    }
    Ok(offset)
}

pub(super) struct CompactIndex {
    map: Mmap,
    pub(super) generation: i64,
    entry_count: u32,
    pub(super) name_count: u32,
    pub(super) extension_count: u32,
    source_count: u32,
    directory_count: u32,
    #[allow(dead_code)]
    pub(super) path_relation_complete: bool,
    pub(super) offsets: [u64; SECTION_COUNT],
}

impl CompactIndex {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if metadata.len() < HEADER_SIZE as u64 || metadata.len() > MAX_INDEX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_file_size_invalid",
            ));
        }
        // The versioned file is immutable once published. The lock protects
        // garbage collection from queries that hold an older SQLite snapshot.
        let map = unsafe { MmapOptions::new().map(&file) }?;
        Self::parse(map)
    }

    fn parse(map: Mmap) -> io::Result<Self> {
        if map.len() < HEADER_SIZE || &map[..8] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "compact_name_search_magic_invalid",
            ));
        }
        let mut at = 8usize;
        let version = take_u32(&map, &mut at)?;
        let generation = take_i64(&map, &mut at)?;
        let entry_count = take_u32(&map, &mut at)?;
        let name_count = take_u32(&map, &mut at)?;
        let extension_count = take_u32(&map, &mut at)?;
        let source_count = take_u32(&map, &mut at)?;
        let directory_count = take_u32(&map, &mut at)?;
        let flags = take_u32(&map, &mut at)?;
        let mut offsets = [0u64; SECTION_COUNT];
        for offset in &mut offsets {
            *offset = take_u64(&map, &mut at)?;
        }
        let digest = map
            .get(at..at + 32)
            .ok_or_else(|| invalid_index("compact_name_search_checksum_missing"))?;
        if version != FORMAT_VERSION || generation < 0 {
            return Err(invalid_index("compact_name_search_version_invalid"));
        }
        if offsets[0] != HEADER_SIZE as u64
            || offsets.iter().any(|offset| *offset > map.len() as u64)
            || offsets.windows(2).any(|pair| pair[0] > pair[1])
            || offsets[SECTION_COUNT - 1] > map.len() as u64
        {
            return Err(invalid_index("compact_name_search_sections_invalid"));
        }
        let computed = blake3::hash(&map[HEADER_SIZE..]);
        if computed.as_bytes() != digest {
            return Err(invalid_index("compact_name_search_checksum_mismatch"));
        }
        let mut index = Self {
            map,
            generation,
            entry_count,
            name_count,
            extension_count,
            source_count,
            directory_count,
            path_relation_complete: flags & 1 != 0,
            offsets,
        };
        index.validate_sections()?;
        Ok(index)
    }

    fn validate_sections(&mut self) -> io::Result<()> {
        let entries = self.entry_count as usize;
        let names = self.name_count as usize;
        let extensions = self.extension_count as usize;
        let sources = self.source_count as usize;
        let directories = self.directory_count as usize;
        self.expect_section_len(0, mul_len(names, 4)?)?;
        self.expect_section_len(1, mul_len(names, 4)?)?;
        self.expect_section_len(3, mul_len(names + 1, 4)?)?;
        self.expect_section_len(4, mul_len(entries, 4)?)?;
        self.expect_section_len(5, mul_len(names.div_ceil(64) * NAME_BITMAP_WIDTH, 8)?)?;
        self.expect_section_len(6, mul_len(extensions, 4)?)?;
        self.expect_section_len(7, mul_len(extensions, 4)?)?;
        self.expect_section_len(9, mul_len(extensions + 1, 4)?)?;
        self.expect_section_len(10, mul_len(entries, 4)?)?;
        self.expect_section_len(11, mul_len(entries, 4)?)?;
        self.expect_section_len(12, mul_len(entries, 4)?)?;
        self.expect_section_len(14, mul_len(entries, 4)?)?;
        self.expect_section_len(15, mul_len(entries, 4)?)?;
        self.expect_section_len(16, mul_len(entries, 4)?)?;
        self.expect_section_len(17, mul_len(entries, 4)?)?;
        self.expect_section_len(18, mul_len(entries, 8)?)?;
        self.expect_section_len(19, entries)?;
        self.expect_section_len(20, mul_len(sources, 4)?)?;
        self.expect_section_len(21, mul_len(sources, 4)?)?;
        self.expect_section_len(23, mul_len(directories, 4)?)?;
        self.expect_section_len(24, mul_len(directories, 4)?)?;
        self.validate_string_table(0, 1, 2, names)?;
        self.validate_string_table(6, 7, 8, extensions)?;
        self.validate_string_table(20, 21, 22, sources)?;
        self.validate_offsets(3, names, entries)?;
        self.validate_offsets(9, extensions, entries)?;
        for entry in 0..entries {
            let name_id = self.u32_at(self.offsets[14], entry);
            let extension_id = self.u32_at(self.offsets[15], entry);
            let source_id = self.u32_at(self.offsets[16], entry);
            let parent = self.u32_at(self.offsets[17], entry);
            if name_id >= self.name_count
                || extension_id >= self.extension_count
                || source_id >= self.source_count
                || (parent != NO_PARENT && parent >= self.directory_count)
            {
                return Err(invalid_index("compact_name_search_entry_reference_invalid"));
            }
            let id = self.entry_id(entry as u32);
            if id.is_empty() {
                return Err(invalid_index("compact_name_search_entry_id_empty"));
            }
        }
        for directory in 0..directories {
            let entry = self.u32_at(self.offsets[23], directory);
            let parent = self.u32_at(self.offsets[24], directory);
            if entry >= self.entry_count
                || !self.entry_flag(entry, FLAG_DIRECTORY)
                || (parent != NO_PARENT && parent >= self.directory_count)
            {
                return Err(invalid_index(
                    "compact_name_search_directory_reference_invalid",
                ));
            }
        }
        let name_postings_end = self.u32_at(self.offsets[3], names);
        let extension_postings_end = self.u32_at(self.offsets[9], extensions);
        if name_postings_end != self.entry_count || extension_postings_end != self.entry_count {
            return Err(invalid_index("compact_name_search_postings_incomplete"));
        }
        for name_id in 0..names {
            let (start, end) = self.name_posting_range(name_id);
            for posting in start..end {
                let entry = self.u32_at(self.offsets[4], posting);
                if self.entry_name(entry) as usize != name_id {
                    return Err(invalid_index("compact_name_search_name_posting_mismatch"));
                }
            }
        }
        for extension_id in 0..extensions {
            let (start, end) = self.extension_posting_range(extension_id);
            for posting in start..end {
                let entry = self.u32_at(self.offsets[10], posting);
                if self.entry_extension(entry) as usize != extension_id {
                    return Err(invalid_index(
                        "compact_name_search_extension_posting_mismatch",
                    ));
                }
            }
        }
        Ok(())
    }

    fn section_len(&self, section: usize) -> io::Result<usize> {
        let start = self.offsets[section];
        let end = if section + 1 < SECTION_COUNT {
            self.offsets[section + 1]
        } else {
            self.map.len() as u64
        };
        usize::try_from(end - start).map_err(io::Error::other)
    }

    fn expect_section_len(&self, section: usize, expected: usize) -> io::Result<()> {
        if self.section_len(section)? != expected {
            return Err(invalid_index("compact_name_search_section_length_invalid"));
        }
        Ok(())
    }

    fn validate_string_table(
        &self,
        offsets_section: usize,
        lengths_section: usize,
        bytes_section: usize,
        count: usize,
    ) -> io::Result<()> {
        let bytes_len = self.section_len(bytes_section)?;
        let mut previous: Option<String> = None;
        for index in 0..count {
            let offset = self.u32_at(self.offsets[offsets_section], index) as usize;
            let length = self.u32_at(self.offsets[lengths_section], index) as usize;
            let end = offset
                .checked_add(length)
                .filter(|end| *end <= bytes_len)
                .ok_or_else(|| invalid_index("compact_name_search_string_bounds_invalid"))?;
            let text = std::str::from_utf8(
                &self.map[self.offsets[bytes_section] as usize + offset
                    ..self.offsets[bytes_section] as usize + end],
            )
            .map_err(|_| invalid_index("compact_name_search_string_utf8_invalid"))?;
            if previous.as_deref().is_some_and(|value| value >= text) {
                return Err(invalid_index("compact_name_search_string_order_invalid"));
            }
            previous = Some(text.to_owned());
        }
        Ok(())
    }

    fn validate_offsets(
        &self,
        section: usize,
        count: usize,
        expected_end: usize,
    ) -> io::Result<()> {
        let mut previous = 0u32;
        for index in 0..=count {
            let value = self.u32_at(self.offsets[section], index);
            if value < previous || value as usize > expected_end {
                return Err(invalid_index("compact_name_search_posting_offsets_invalid"));
            }
            previous = value;
        }
        if previous as usize != expected_end {
            return Err(invalid_index(
                "compact_name_search_posting_offsets_incomplete",
            ));
        }
        Ok(())
    }

    pub(super) fn u32_at(&self, section_offset: u64, index: usize) -> u32 {
        let at = section_offset as usize + index * 4;
        u32::from_le_bytes(
            self.map[at..at + 4]
                .try_into()
                .expect("validated u32 section"),
        )
    }

    fn u64_at(&self, section_offset: u64, index: usize) -> u64 {
        let at = section_offset as usize + index * 8;
        u64::from_le_bytes(
            self.map[at..at + 8]
                .try_into()
                .expect("validated u64 section"),
        )
    }

    fn i64_at(&self, section_offset: u64, index: usize) -> i64 {
        let at = section_offset as usize + index * 8;
        i64::from_le_bytes(
            self.map[at..at + 8]
                .try_into()
                .expect("validated i64 section"),
        )
    }

    fn string_at(
        &self,
        offsets_section: usize,
        lengths_section: usize,
        bytes_section: usize,
        index: usize,
    ) -> &str {
        let offset = self.u32_at(self.offsets[offsets_section], index) as usize;
        let length = self.u32_at(self.offsets[lengths_section], index) as usize;
        let start = self.offsets[bytes_section] as usize + offset;
        std::str::from_utf8(&self.map[start..start + length]).expect("validated string table")
    }

    fn name(&self, index: usize) -> &str {
        self.string_at(0, 1, 2, index)
    }

    fn extension(&self, index: usize) -> &str {
        self.string_at(6, 7, 8, index)
    }

    pub(super) fn source(&self, index: usize) -> &str {
        self.string_at(20, 21, 22, index)
    }

    pub(super) fn entry_id(&self, index: u32) -> &str {
        self.string_at(11, 12, 13, index as usize)
    }

    pub(super) fn entry_name(&self, index: u32) -> u32 {
        self.u32_at(self.offsets[14], index as usize)
    }

    fn entry_extension(&self, index: u32) -> u32 {
        self.u32_at(self.offsets[15], index as usize)
    }

    pub(super) fn entry_source(&self, index: u32) -> u32 {
        self.u32_at(self.offsets[16], index as usize)
    }

    pub(super) fn entry_mtime(&self, index: u32) -> Option<i64> {
        let value = self.i64_at(self.offsets[18], index as usize);
        (value != NULL_MTIME).then_some(value)
    }

    pub(super) fn entry_flag(&self, index: u32, flag: u8) -> bool {
        self.map[self.offsets[19] as usize + index as usize] & flag != 0
    }

    pub(super) fn name_posting_range(&self, name_id: usize) -> (usize, usize) {
        (
            self.u32_at(self.offsets[3], name_id) as usize,
            self.u32_at(self.offsets[3], name_id + 1) as usize,
        )
    }

    pub(super) fn extension_posting_range(&self, extension_id: usize) -> (usize, usize) {
        (
            self.u32_at(self.offsets[9], extension_id) as usize,
            self.u32_at(self.offsets[9], extension_id + 1) as usize,
        )
    }

    pub(super) fn matching_names(&self, query: &str) -> Vec<u8> {
        let mut output = vec![NO_MATCH; self.name_count as usize];
        let mut slots = [false; NAME_BITMAP_WIDTH];
        for byte in query.bytes() {
            if let Some(slot) = ascii_slot(byte) {
                slots[slot] = true;
            }
        }
        let blocks = (self.name_count as usize).div_ceil(64);
        for block in 0..blocks {
            let remaining = self.name_count as usize - block * 64;
            let mut candidates = if remaining >= 64 {
                u64::MAX
            } else {
                (1u64 << remaining) - 1
            };
            for (slot, required) in slots.iter().enumerate() {
                if *required {
                    candidates &= self.u64_at(
                        self.offsets[5] + (block * NAME_BITMAP_WIDTH + slot) as u64 * 8,
                        0,
                    );
                    if candidates == 0 {
                        break;
                    }
                }
            }
            while candidates != 0 {
                let bit = candidates.trailing_zeros() as usize;
                candidates &= candidates - 1;
                let name_id = block * 64 + bit;
                if let Some(class) = super::query::match_name(self.name(name_id), query) {
                    output[name_id] = class;
                }
            }
        }
        output
    }

    pub(super) fn matching_extensions(&self, query: &str) -> Vec<u8> {
        (0..self.extension_count as usize)
            .map(|index| u8::from(self.extension(index).contains(query)))
            .collect()
    }
}

fn take_u32(bytes: &[u8], at: &mut usize) -> io::Result<u32> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| invalid_index("compact_name_search_header_overflow"))?;
    let value = bytes
        .get(*at..end)
        .ok_or_else(|| invalid_index("compact_name_search_header_truncated"))?;
    *at = end;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}

fn take_u64(bytes: &[u8], at: &mut usize) -> io::Result<u64> {
    let end = at
        .checked_add(8)
        .ok_or_else(|| invalid_index("compact_name_search_header_overflow"))?;
    let value = bytes
        .get(*at..end)
        .ok_or_else(|| invalid_index("compact_name_search_header_truncated"))?;
    *at = end;
    Ok(u64::from_le_bytes(value.try_into().expect("eight bytes")))
}

fn take_i64(bytes: &[u8], at: &mut usize) -> io::Result<i64> {
    take_u64(bytes, at).map(|value| i64::from_le_bytes(value.to_le_bytes()))
}

fn mul_len(left: usize, right: usize) -> io::Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| invalid_index("compact_name_search_section_overflow"))
}

fn invalid_index(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
