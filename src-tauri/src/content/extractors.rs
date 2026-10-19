use super::*;

pub(super) fn text_extraction(
    family: &str,
    bytes: Vec<u8>,
    policy: &ContentScopePolicyDto,
) -> Result<Extraction, DbError> {
    let text = decode_bounded_text(&bytes, "content_text_not_utf8")?;
    let (text, truncated) = bound_text(text, policy.max_chars as usize);
    Ok(Extraction {
        family: family.into(),
        text,
        source_hash: String::new(),
        truncated,
        status: "completed",
        reason: None,
    })
}

pub(super) fn csv_extraction(
    bytes: Vec<u8>,
    policy: &ContentScopePolicyDto,
) -> Result<Extraction, DbError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes.as_slice());
    let mut rows = Vec::new();
    let mut rows_truncated = false;
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|_| DbError::Validation("content_csv_invalid".into()))?;
        if index >= policy.max_rows as usize {
            rows_truncated = true;
            break;
        }
        rows.push(record.iter().map(str::trim).collect::<Vec<_>>().join("\t"));
    }
    let rows = rows.join("\n");
    let (text, truncated) = bound_text(rows, policy.max_chars as usize);
    Ok(Extraction {
        family: "csv".into(),
        text,
        source_hash: String::new(),
        truncated: truncated || rows_truncated,
        status: "completed",
        reason: None,
    })
}

pub(super) fn decode_bounded_text(bytes: &[u8], error_code: &str) -> Result<String, DbError> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err(DbError::Validation(error_code.to_string()));
        }
        let little_endian = bytes[0] == 0xff;
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| {
                if little_endian {
                    u16::from_le_bytes([chunk[0], chunk[1]])
                } else {
                    u16::from_be_bytes([chunk[0], chunk[1]])
                }
            })
            .collect::<Vec<_>>();
        return String::from_utf16(&units).map_err(|_| DbError::Validation(error_code.to_string()));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| DbError::Validation(error_code.to_string()))
}

pub(super) fn pdf_text_extraction(
    bytes: Vec<u8>,
    policy: &ContentScopePolicyDto,
) -> Result<Extraction, DbError> {
    pdf_text_extraction_with_limits(
        &bytes,
        policy,
        Instant::now() + Duration::from_secs(2),
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PdfStop {
    Timeout,
    Cancelled,
    Invalid,
    Limit(&'static str),
}

pub(super) struct PdfTextAccumulator {
    text: String,
    max_chars: usize,
    emitted_chars: usize,
    truncated: bool,
}

impl PdfTextAccumulator {
    fn new(max_chars: usize) -> Self {
        Self {
            text: String::new(),
            max_chars,
            emitted_chars: 0,
            truncated: false,
        }
    }

    fn push_char(&mut self, value: char) {
        if self.emitted_chars < self.max_chars {
            self.text.push(value);
            self.emitted_chars = self.emitted_chars.saturating_add(1);
        } else {
            self.truncated = true;
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        if bytes.starts_with(&[0xfe, 0xff]) && bytes.len() >= 4 {
            for chunk in bytes[2..].chunks_exact(2) {
                if let Some(character) =
                    char::from_u32(u16::from_be_bytes([chunk[0], chunk[1]]) as u32)
                {
                    self.push_char(character);
                }
            }
        } else {
            for character in String::from_utf8_lossy(bytes).chars() {
                self.push_char(character);
            }
        }
    }

    fn push_mapped_bytes(&mut self, bytes: &[u8], cmap: &HashMap<Vec<u8>, String>) {
        if bytes.len() == 1 && bytes[0] == 0 && !cmap.is_empty() {
            return;
        }
        if bytes.len() >= 4
            && bytes.len().is_multiple_of(2)
            && bytes.chunks_exact(2).all(|chunk| chunk[0] == 0)
        {
            for chunk in bytes.chunks_exact(2) {
                self.push_char(char::from(chunk[1]));
            }
            return;
        }
        if cmap.is_empty() {
            self.push_bytes(bytes);
            return;
        }
        let mut index = 0_usize;
        while index < bytes.len() {
            let mut matched = None;
            for width in (1..=4).rev() {
                if let Some(value) = bytes.get(index..index.saturating_add(width)) {
                    if let Some(mapped) = cmap.get(value) {
                        matched = Some((width, mapped));
                        break;
                    }
                }
            }
            if let Some((width, mapped)) = matched {
                for character in mapped.chars() {
                    self.push_char(character);
                }
                index += width;
            } else {
                // Most application PDFs use fixed-width two-byte glyph IDs.
                // A zero high byte is a transport marker, not user text.
                if bytes[index] == 0 && bytes.get(index + 1).is_some() {
                    index += 1;
                    continue;
                }
                self.push_bytes(&bytes[index..index + 1]);
                index += 1;
            }
        }
    }
}

pub(super) fn pdf_text_extraction_with_limits(
    bytes: &[u8],
    policy: &ContentScopePolicyDto,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<Extraction, DbError> {
    pdf_text_extraction_with_limits_and_hook(bytes, policy, deadline, cancel, None)
}

pub(super) fn pdf_text_extraction_with_limits_and_hook(
    bytes: &[u8],
    policy: &ContentScopePolicyDto,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
    work_hook: Option<&dyn Fn()>,
) -> Result<Extraction, DbError> {
    let blocked = |reason: &str| {
        Ok(Extraction {
            family: "pdf_text".into(),
            text: String::new(),
            source_hash: String::new(),
            truncated: false,
            status: "blocked",
            reason: Some(reason.into()),
        })
    };
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF-") {
        return blocked("content_pdf_invalid");
    }
    let mut accumulator = PdfTextAccumulator::new(policy.max_chars.max(0) as usize);
    let mut objects = 0_usize;
    let mut pages = 0_i64;
    let mut decompressed = 0_usize;
    let mut streams = Vec::<Vec<u8>>::new();
    let mut offset = 5_usize;
    let mut work_started = false;
    while offset < bytes.len() {
        let obj_start = match find_pdf_object_start_bounded(bytes, offset, deadline, cancel) {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        if !work_started {
            if let Some(hook) = work_hook {
                hook();
            }
            work_started = true;
        }
        if let Err(stop) = pdf_budget_check(obj_start, bytes.len(), deadline, cancel) {
            return Ok(pdf_stop_extraction(stop));
        }
        let endobj = match find_pdf_token_bounded(bytes, b"endobj", obj_start + 3, deadline, cancel)
        {
            Ok(Some(value)) => value,
            Ok(None) => break,
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        objects = objects.saturating_add(1);
        if objects > PDF_MAX_OBJECTS {
            return blocked("content_pdf_object_limit_exceeded");
        }
        let object = &bytes[obj_start..endobj];
        match oversized_uncompressed_pdf_cmap_stream(object, deadline, cancel) {
            Ok(true) => return blocked("content_pdf_cmap_decoded_byte_limit_exceeded"),
            Ok(false) => {}
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        }
        let encrypted = match contains_pdf_token_bounded(object, b"/Encrypt", deadline, cancel) {
            Ok(value) => value,
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        if encrypted {
            return blocked("content_encrypted_document");
        }
        let is_page = match contains_pdf_type_page_bounded(object, deadline, cancel) {
            Ok(value) => value,
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        if is_page {
            pages = pages.saturating_add(1);
            if pages > policy.max_pages {
                return blocked("content_pdf_page_limit_exceeded");
            }
        }
        let mut local = 0_usize;
        while let Some(stream_rel) =
            match find_pdf_token_bounded(object, b"stream", local, deadline, cancel) {
                Ok(value) => value,
                Err(stop) => return Ok(pdf_stop_extraction(stop)),
            }
        {
            if let Err(stop) =
                pdf_budget_check(obj_start + stream_rel, bytes.len(), deadline, cancel)
            {
                return Ok(pdf_stop_extraction(stop));
            }
            let data_start = pdf_stream_data_start(object, stream_rel + b"stream".len());
            let endstream =
                match find_pdf_token_bounded(object, b"endstream", data_start, deadline, cancel) {
                    Ok(Some(value)) => value,
                    Ok(None) => return blocked("content_pdf_invalid"),
                    Err(stop) => return Ok(pdf_stop_extraction(stop)),
                };
            let stream = &object[data_start..endstream];
            if stream.len() > policy.max_bytes.max(0) as usize {
                return blocked("content_decompressed_byte_limit_exceeded");
            }
            let dictionary = &object[..stream_rel];
            let flate =
                match contains_pdf_token_bounded(dictionary, b"/FlateDecode", deadline, cancel) {
                    Ok(value) => value,
                    Err(stop) => return Ok(pdf_stop_extraction(stop)),
                };
            if flate {
                let mut decoder = ZlibDecoder::new(stream);
                let mut decoded = Vec::new();
                let mut chunk = [0_u8; 8192];
                loop {
                    if let Err(stop) =
                        pdf_budget_check(obj_start + data_start, bytes.len(), deadline, cancel)
                    {
                        return Ok(pdf_stop_extraction(stop));
                    }
                    let read = decoder
                        .read(&mut chunk)
                        .map_err(|_| DbError::Validation("content_pdf_invalid".into()))?;
                    if read == 0 {
                        break;
                    }
                    decompressed = decompressed.saturating_add(read);
                    if decompressed > pdf_decompressed_limit(policy) {
                        return blocked("content_pdf_decompressed_byte_limit_exceeded");
                    }
                    decoded.extend_from_slice(&chunk[..read]);
                    if decoded.len() > pdf_decompressed_limit(policy) {
                        return blocked("content_pdf_decompressed_byte_limit_exceeded");
                    }
                }
                streams.push(decoded);
            } else {
                decompressed = decompressed.saturating_add(stream.len());
                if decompressed > pdf_decompressed_limit(policy) {
                    return blocked("content_pdf_decompressed_byte_limit_exceeded");
                }
                streams.push(stream.to_vec());
            }
            local = endstream.saturating_add(b"endstream".len());
        }
        offset = endobj.saturating_add(b"endobj".len());
    }
    if objects == 0 || pages == 0 {
        return blocked("content_pdf_invalid");
    }
    let mut cmap = HashMap::new();
    let mut cmap_decoded_bytes = 0_usize;
    for stream in &streams {
        let has_bfchar = match find_pdf_token_bounded(stream, b"beginbfchar", 0, deadline, cancel) {
            Ok(value) => value.is_some(),
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        let has_bfrange = match find_pdf_token_bounded(stream, b"beginbfrange", 0, deadline, cancel)
        {
            Ok(value) => value.is_some(),
            Err(stop) => return Ok(pdf_stop_extraction(stop)),
        };
        if has_bfchar || has_bfrange {
            if let Err(stop) =
                parse_pdf_cmap(stream, &mut cmap, &mut cmap_decoded_bytes, deadline, cancel)
            {
                return Ok(pdf_stop_extraction(stop));
            }
        }
    }
    for stream in &streams {
        if let Err(stop) = parse_pdf_text_stream(stream, &mut accumulator, &cmap, deadline, cancel)
        {
            return Ok(pdf_stop_extraction(stop));
        }
    }
    if accumulator.text.trim().is_empty() {
        return blocked("ocr_only_or_no_text_layer");
    }
    Ok(Extraction {
        family: "pdf_text".into(),
        text: accumulator.text,
        source_hash: String::new(),
        truncated: accumulator.truncated,
        status: "completed",
        reason: None,
    })
}

pub(super) fn oversized_uncompressed_pdf_cmap_stream(
    object: &[u8],
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<bool, PdfStop> {
    let Some(stream_start) = find_pdf_keyword_bounded(object, b"stream", 0, deadline, cancel)?
    else {
        return Ok(false);
    };
    let Some(endstream_start) = find_pdf_keyword_bounded(
        object,
        b"endstream",
        stream_start.saturating_add(b"stream".len()),
        deadline,
        cancel,
    )?
    else {
        return Ok(false);
    };
    let data_start = pdf_stream_data_start_bounded(
        object,
        stream_start.saturating_add(b"stream".len()),
        deadline,
        cancel,
    )?;
    if data_start > endstream_start {
        return Ok(false);
    }
    let dictionary = &object[..stream_start];
    let dictionary_has_filter =
        contains_pdf_token_bounded(dictionary, b"/Filter", deadline, cancel)?
            || contains_pdf_token_bounded(dictionary, b"/FlateDecode", deadline, cancel)?;
    if dictionary_has_filter {
        return Ok(false);
    }
    let raw_stream = &object[data_start..endstream_start];
    if raw_stream.len() <= PDF_MAX_CMAP_DECODED_BYTES {
        return Ok(false);
    }
    let has_cid_init = contains_pdf_token_bounded(raw_stream, b"/CIDInit", deadline, cancel)?;
    let has_cmap_body = contains_pdf_token_bounded(raw_stream, b"beginbfchar", deadline, cancel)?
        || contains_pdf_token_bounded(raw_stream, b"beginbfrange", deadline, cancel)?;
    Ok(has_cid_init && has_cmap_body)
}

pub(super) fn pdf_stop_extraction(stop: PdfStop) -> Extraction {
    let (status, reason) = match stop {
        PdfStop::Timeout => ("failed", "content_extractor_timeout"),
        PdfStop::Cancelled => ("failed", "content_extractor_cancelled"),
        PdfStop::Invalid => ("blocked", "content_pdf_invalid"),
        PdfStop::Limit(reason) => ("blocked", reason),
    };
    Extraction {
        family: "pdf_text".into(),
        text: String::new(),
        source_hash: String::new(),
        truncated: false,
        status,
        reason: Some(reason.into()),
    }
}

pub(super) fn pdf_decompressed_limit(policy: &ContentScopePolicyDto) -> usize {
    (policy.max_bytes.max(1024) as usize)
        .saturating_mul(4)
        .min(PDF_MAX_DECOMPRESSED_BYTES)
}

pub(super) fn pdf_budget_check(
    offset: usize,
    length: usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<(), PdfStop> {
    if cancel.is_some_and(|value| value.load(Ordering::Relaxed)) {
        return Err(PdfStop::Cancelled);
    }
    if Instant::now() > deadline {
        return Err(PdfStop::Timeout);
    }
    let _ = (offset, length);
    Ok(())
}

pub(super) fn find_pdf_object_start_bounded(
    bytes: &[u8],
    from: usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<Option<usize>, PdfStop> {
    let mut index = from;
    while index + 3 < bytes.len() {
        if index
            .saturating_sub(from)
            .is_multiple_of(PDF_SCAN_CHECK_BYTES)
        {
            pdf_budget_check(index, bytes.len(), deadline, cancel)?;
        }
        if bytes[index].is_ascii_digit()
            && (index == 0 || bytes[index - 1].is_ascii_whitespace())
            && bytes[index + 1..].windows(3).next().is_some()
        {
            let mut cursor = index;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                if cursor
                    .saturating_sub(from)
                    .is_multiple_of(PDF_SCAN_CHECK_BYTES)
                {
                    pdf_budget_check(cursor, bytes.len(), deadline, cancel)?;
                }
                cursor += 1;
            }
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                if cursor
                    .saturating_sub(from)
                    .is_multiple_of(PDF_SCAN_CHECK_BYTES)
                {
                    pdf_budget_check(cursor, bytes.len(), deadline, cancel)?;
                }
                cursor += 1;
            }
            let generation_start = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                if cursor
                    .saturating_sub(from)
                    .is_multiple_of(PDF_SCAN_CHECK_BYTES)
                {
                    pdf_budget_check(cursor, bytes.len(), deadline, cancel)?;
                }
                cursor += 1;
            }
            if cursor > generation_start {
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    if cursor
                        .saturating_sub(from)
                        .is_multiple_of(PDF_SCAN_CHECK_BYTES)
                    {
                        pdf_budget_check(cursor, bytes.len(), deadline, cancel)?;
                    }
                    cursor += 1;
                }
                if bytes.get(cursor..cursor + 3) == Some(b"obj") {
                    return Ok(Some(index));
                }
            }
        }
        index += 1;
    }
    pdf_budget_check(bytes.len(), bytes.len(), deadline, cancel)?;
    Ok(None)
}

pub(super) fn find_pdf_token_bounded(
    bytes: &[u8],
    token: &[u8],
    from: usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<Option<usize>, PdfStop> {
    if token.is_empty() {
        return Ok(Some(from.min(bytes.len())));
    }
    let end = bytes.len().saturating_sub(token.len());
    let mut index = from.min(bytes.len());
    while index <= end {
        if index
            .saturating_sub(from)
            .is_multiple_of(PDF_SCAN_CHECK_BYTES)
        {
            pdf_budget_check(index, bytes.len(), deadline, cancel)?;
        }
        if bytes.get(index..index + token.len()) == Some(token) {
            return Ok(Some(index));
        }
        index = index.saturating_add(1);
    }
    pdf_budget_check(bytes.len(), bytes.len(), deadline, cancel)?;
    Ok(None)
}

pub(super) fn find_pdf_keyword_bounded(
    bytes: &[u8],
    token: &[u8],
    from: usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<Option<usize>, PdfStop> {
    if token.is_empty() {
        return Ok(Some(from.min(bytes.len())));
    }
    let end = bytes.len().saturating_sub(token.len());
    let mut index = from.min(bytes.len());
    while index <= end {
        pdf_budget_check(index, bytes.len(), deadline, cancel)?;
        let chunk_end = index
            .saturating_add(PDF_SCAN_CHECK_BYTES)
            .min(end.saturating_add(1));
        let search_end = chunk_end
            .saturating_add(token.len().saturating_sub(1))
            .min(bytes.len());
        let mut search_offset: usize = 0;
        while search_offset.saturating_add(token.len()) <= search_end.saturating_sub(index) {
            let chunk = &bytes[index.saturating_add(search_offset)..search_end];
            let Some(offset) = chunk
                .windows(token.len())
                .position(|window| window == token)
            else {
                break;
            };
            let candidate = index.saturating_add(search_offset).saturating_add(offset);
            if pdf_keyword_boundary(bytes, candidate, token.len()) {
                return Ok(Some(candidate));
            }
            search_offset = search_offset.saturating_add(offset).saturating_add(1);
        }
        index = chunk_end;
    }
    pdf_budget_check(bytes.len(), bytes.len(), deadline, cancel)?;
    Ok(None)
}

pub(super) fn pdf_keyword_boundary(bytes: &[u8], start: usize, length: usize) -> bool {
    let is_delimiter = |value: Option<&u8>| {
        value.is_none_or(|value| {
            value.is_ascii_whitespace()
                || matches!(
                    *value,
                    b'<' | b'>' | b'[' | b']' | b'(' | b')' | b'/' | b'%'
                )
        })
    };
    is_delimiter(start.checked_sub(1).and_then(|index| bytes.get(index)))
        && is_delimiter(bytes.get(start.saturating_add(length)))
}

pub(super) fn contains_pdf_token_bounded(
    bytes: &[u8],
    token: &[u8],
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<bool, PdfStop> {
    Ok(find_pdf_token_bounded(bytes, token, 0, deadline, cancel)?.is_some())
}

pub(super) fn contains_pdf_type_page_bounded(
    bytes: &[u8],
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<bool, PdfStop> {
    let Some(type_start) = find_pdf_token_bounded(bytes, b"/Type", 0, deadline, cancel)? else {
        return Ok(false);
    };
    let remainder = &bytes[type_start + b"/Type".len()..];
    let Some(page_start) = find_pdf_token_bounded(remainder, b"/Page", 0, deadline, cancel)? else {
        return Ok(false);
    };
    Ok(remainder
        .get(page_start + b"/Page".len())
        .is_none_or(|character| !character.is_ascii_alphabetic()))
}

pub(super) fn pdf_stream_data_start(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
        index += 1;
    }
    if bytes.get(index) == Some(&b'\r') {
        index += 1;
        if bytes.get(index) == Some(&b'\n') {
            index += 1;
        }
    } else if bytes.get(index) == Some(&b'\n') {
        index += 1;
    }
    index
}

pub(super) fn pdf_stream_data_start_bounded(
    bytes: &[u8],
    mut index: usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<usize, PdfStop> {
    while index < bytes.len() && (bytes[index] == b' ' || bytes[index] == b'\t') {
        pdf_budget_check(index, bytes.len(), deadline, cancel)?;
        index += 1;
    }
    if bytes.get(index) == Some(&b'\r') {
        index += 1;
        if bytes.get(index) == Some(&b'\n') {
            index += 1;
        }
    } else if bytes.get(index) == Some(&b'\n') {
        index += 1;
    }
    pdf_budget_check(index, bytes.len(), deadline, cancel)?;
    Ok(index)
}

pub(super) fn parse_pdf_text_stream(
    stream: &[u8],
    accumulator: &mut PdfTextAccumulator,
    cmap: &HashMap<Vec<u8>, String>,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<(), PdfStop> {
    let mut index = 0_usize;
    while index < stream.len() {
        if index.is_multiple_of(PDF_SCAN_CHECK_BYTES) {
            pdf_budget_check(index, stream.len(), deadline, cancel)?;
        }
        match stream[index] {
            b'(' => {
                index += 1;
                let mut depth = 1_i32;
                let mut escaped = false;
                let mut literal = Vec::new();
                while index < stream.len() && depth > 0 {
                    if index.is_multiple_of(PDF_SCAN_CHECK_BYTES) {
                        pdf_budget_check(index, stream.len(), deadline, cancel)?;
                    }
                    let character = stream[index];
                    if escaped {
                        let decoded = match character {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 8,
                            b'f' => 12,
                            value => value,
                        };
                        if literal.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                            return Err(PdfStop::Limit(
                                "content_pdf_literal_buffer_limit_exceeded",
                            ));
                        }
                        literal.push(decoded);
                        escaped = false;
                    } else if character == b'\\' {
                        escaped = true;
                    } else if character == b'(' {
                        depth += 1;
                        if literal.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                            return Err(PdfStop::Limit(
                                "content_pdf_literal_buffer_limit_exceeded",
                            ));
                        }
                        literal.push(b'(');
                    } else if character == b')' {
                        depth -= 1;
                        if depth > 0 {
                            if literal.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                                return Err(PdfStop::Limit(
                                    "content_pdf_literal_buffer_limit_exceeded",
                                ));
                            }
                            literal.push(b')');
                        }
                    } else {
                        if literal.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                            return Err(PdfStop::Limit(
                                "content_pdf_literal_buffer_limit_exceeded",
                            ));
                        }
                        literal.push(character);
                    }
                    index += 1;
                }
                if depth != 0 {
                    return Err(PdfStop::Invalid);
                }
                accumulator.push_mapped_bytes(&literal, cmap);
            }
            b'<' if stream.get(index + 1) != Some(&b'<') => {
                index += 1;
                let mut high = None;
                let mut decoded = Vec::new();
                while index < stream.len() && stream[index] != b'>' {
                    if index.is_multiple_of(PDF_SCAN_CHECK_BYTES) {
                        pdf_budget_check(index, stream.len(), deadline, cancel)?;
                    }
                    let value = stream[index];
                    if value.is_ascii_hexdigit() {
                        let nibble = pdf_hex_nibble(value).ok_or(PdfStop::Invalid)?;
                        if let Some(first) = high.take() {
                            if decoded.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                                return Err(PdfStop::Limit(
                                    "content_pdf_hex_buffer_limit_exceeded",
                                ));
                            }
                            decoded.push((first << 4) | nibble);
                        } else {
                            high = Some(nibble);
                        }
                    }
                    index += 1;
                }
                if stream.get(index) != Some(&b'>') {
                    return Err(PdfStop::Invalid);
                }
                if let Some(first) = high {
                    if decoded.len() >= PDF_MAX_TEMP_BUFFER_BYTES {
                        return Err(PdfStop::Limit("content_pdf_hex_buffer_limit_exceeded"));
                    }
                    decoded.push(first << 4);
                }
                accumulator.push_mapped_bytes(&decoded, cmap);
            }
            _ => index += 1,
        }
    }
    Ok(())
}

pub(super) fn parse_pdf_cmap(
    stream: &[u8],
    cmap: &mut HashMap<Vec<u8>, String>,
    decoded_bytes: &mut usize,
    deadline: Instant,
    cancel: Option<&AtomicBool>,
) -> Result<(), PdfStop> {
    if stream.len() > PDF_MAX_CMAP_DECODED_BYTES {
        return Err(PdfStop::Limit(
            "content_pdf_cmap_decoded_byte_limit_exceeded",
        ));
    }
    let source = String::from_utf8_lossy(stream);
    let mut mode = None;
    let mut scanned = 0_usize;
    for line in source.lines() {
        scanned = scanned.saturating_add(line.len());
        if scanned.is_multiple_of(PDF_SCAN_CHECK_BYTES)
            || scanned.saturating_sub(line.len()) < PDF_SCAN_CHECK_BYTES
        {
            pdf_budget_check(scanned, stream.len(), deadline, cancel)?;
        }
        if contains_pdf_token_bounded(line.as_bytes(), b"beginbfchar", deadline, cancel)? {
            mode = Some("char");
            continue;
        }
        if contains_pdf_token_bounded(line.as_bytes(), b"beginbfrange", deadline, cancel)? {
            mode = Some("range");
            continue;
        }
        if contains_pdf_token_bounded(line.as_bytes(), b"endbfchar", deadline, cancel)?
            || contains_pdf_token_bounded(line.as_bytes(), b"endbfrange", deadline, cancel)?
        {
            mode = None;
            continue;
        }
        let Some(mode) = mode else { continue };
        if line.len() > PDF_MAX_TEMP_BUFFER_BYTES {
            return Err(PdfStop::Limit(
                "content_pdf_cmap_temporary_buffer_limit_exceeded",
            ));
        }
        let tokens = line
            .split_whitespace()
            .filter_map(|token| {
                let trimmed = token.trim_matches(|value| value == '<' || value == '>');
                (!trimmed.is_empty()).then(|| pdf_hex_bytes(trimmed))
            })
            .collect::<Option<Vec<_>>>();
        let Some(tokens) = tokens else { continue };
        match (mode, tokens.as_slice()) {
            ("char", [source, target, ..]) => {
                if let Some(text) = pdf_unicode_string(target) {
                    insert_pdf_cmap_entry(cmap, decoded_bytes, source.clone(), text)?;
                }
            }
            ("range", [start, end, target, ..]) if start.len() == end.len() => {
                let start_value = pdf_big_endian_value(start);
                let end_value = pdf_big_endian_value(end);
                if let Some(base) = pdf_unicode_string(target) {
                    if start_value > end_value || end_value - start_value > 1024 {
                        continue;
                    }
                    let base_value = base.chars().next().map(|value| value as u32).unwrap_or(0);
                    for value in start_value..=end_value {
                        pdf_budget_check(value as usize, end_value as usize, deadline, cancel)?;
                        let mut key = value.to_be_bytes().to_vec();
                        key = key[key.len().saturating_sub(start.len())..].to_vec();
                        if let Some(character) = char::from_u32(base_value + value - start_value) {
                            insert_pdf_cmap_entry(cmap, decoded_bytes, key, character.to_string())?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    pdf_budget_check(stream.len(), stream.len(), deadline, cancel)
}

pub(super) fn insert_pdf_cmap_entry(
    cmap: &mut HashMap<Vec<u8>, String>,
    decoded_bytes: &mut usize,
    key: Vec<u8>,
    value: String,
) -> Result<(), PdfStop> {
    if cmap.len() >= PDF_MAX_CMAP_ENTRIES || key.len() > PDF_MAX_TEMP_BUFFER_BYTES {
        return Err(PdfStop::Limit("content_pdf_cmap_entry_limit_exceeded"));
    }
    let value_bytes = value.len();
    if (*decoded_bytes).saturating_add(value_bytes) > PDF_MAX_CMAP_DECODED_BYTES {
        return Err(PdfStop::Limit(
            "content_pdf_cmap_decoded_byte_limit_exceeded",
        ));
    }
    if cmap.insert(key, value).is_none() {
        *decoded_bytes = (*decoded_bytes).saturating_add(value_bytes);
    }
    Ok(())
}

pub(super) fn pdf_hex_bytes(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || value.is_empty()
        || value.len() / 2 > PDF_MAX_TEMP_BUFFER_BYTES
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let chars = value.as_bytes();
    for pair in chars.chunks_exact(2) {
        bytes.push((pdf_hex_nibble(pair[0])? << 4) | pdf_hex_nibble(pair[1])?);
    }
    Some(bytes)
}

pub(super) fn pdf_big_endian_value(value: &[u8]) -> u32 {
    value
        .iter()
        .fold(0_u32, |current, byte| (current << 8) | u32::from(*byte))
}

pub(super) fn pdf_unicode_string(value: &[u8]) -> Option<String> {
    if value.len() > PDF_MAX_TEMP_BUFFER_BYTES {
        return None;
    }
    if value.starts_with(&[0xfe, 0xff]) {
        let units = value[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).ok()
    } else if value.len() >= 2 && value.len().is_multiple_of(2) {
        let units = value
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).ok()
    } else {
        Some(String::from_utf8_lossy(value).into_owned())
    }
}

pub(super) fn pdf_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(super) fn office_xml_extraction(
    family: &str,
    bytes: Vec<u8>,
    policy: &ContentScopePolicyDto,
    names: &[&str],
) -> Result<Extraction, DbError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
    if archive.len() > 1_000 {
        return Ok(Extraction {
            family: family.into(),
            text: String::new(),
            source_hash: String::new(),
            truncated: false,
            status: "blocked",
            reason: Some("content_archive_entry_count_limit_exceeded".into()),
        });
    }
    let mut target_names = names
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut member_names = HashSet::new();
    // zip 9 indexes by raw filename bytes. Keep decoded-name lookup semantics
    // for Office discovery, after the existing duplicate-name validation.
    let mut member_indices = HashMap::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
        let name = entry
            .name()
            .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
        if let Err(error) = register_office_member_name(&mut member_names, &name) {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some(content_error_code(&error)),
            });
        }
        member_indices.insert(name.into_owned(), index);
        if entry.encrypted() {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some("content_encrypted_document".into()),
            });
        }
    }
    let mut document_unit_count = 0_usize;
    if family == "xlsx" || family == "pptx" {
        let mut discovered = Vec::new();
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
            let name = entry
                .name()
                .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?
                .into_owned();
            let prefix = if family == "xlsx" {
                "xl/worksheets/sheet"
            } else {
                "ppt/slides/slide"
            };
            if name.starts_with(prefix) && name.ends_with(".xml") {
                discovered.push(name);
            }
        }
        discovered.sort();
        document_unit_count = discovered.len();
        target_names.extend(discovered);
    }
    target_names.sort();
    target_names.dedup();
    if (family == "xlsx" || family == "pptx") && document_unit_count as i64 > policy.max_pages {
        return Ok(Extraction {
            family: family.into(),
            text: String::new(),
            source_hash: String::new(),
            truncated: false,
            status: "blocked",
            reason: Some("content_page_limit_exceeded".into()),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let shared_strings = if family == "xlsx" {
        if let Some(&index) = member_indices.get("xl/sharedStrings.xml") {
            let mut entry = archive
                .by_index(index)
                .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
            let xml = match read_zip_entry_bounded(&mut entry, policy.max_bytes as u64, deadline) {
                Ok(xml) => xml,
                Err(error) => {
                    let code = content_error_code(&error);
                    return Ok(Extraction {
                        family: family.into(),
                        text: String::new(),
                        source_hash: String::new(),
                        truncated: false,
                        status: if code == "content_extractor_timeout" {
                            "failed"
                        } else {
                            "blocked"
                        },
                        reason: Some(code),
                    });
                }
            };
            match parse_xlsx_shared_strings(&xml) {
                Ok(values) => values,
                Err(_) => {
                    return Ok(Extraction {
                        family: family.into(),
                        text: String::new(),
                        source_hash: String::new(),
                        truncated: false,
                        status: "blocked",
                        reason: Some("content_office_xml_invalid".into()),
                    });
                }
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let mut text = String::new();
    let mut decompressed_bytes = 0_u64;
    let mut total_rows = 0_i64;
    for name in &target_names {
        if Instant::now() > deadline {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "failed",
                reason: Some("content_extractor_timeout".into()),
            });
        }
        let Some(&index) = member_indices.get(name) else {
            continue;
        };
        let mut entry = archive
            .by_index(index)
            .map_err(|_| DbError::Validation("content_office_container_invalid".into()))?;
        if entry.encrypted() {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some("content_encrypted_document".into()),
            });
        }
        if entry.size() > policy.max_bytes as u64
            || (entry.compressed_size() > 0
                && entry.size() > entry.compressed_size().saturating_mul(100))
        {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some("content_archive_entry_limit_exceeded".into()),
            });
        }
        let xml = match read_zip_entry_bounded(&mut entry, policy.max_bytes as u64, deadline) {
            Ok(xml) => xml,
            Err(error) => {
                let code = content_error_code(&error);
                return Ok(Extraction {
                    family: family.into(),
                    text: String::new(),
                    source_hash: String::new(),
                    truncated: false,
                    status: if code == "content_extractor_timeout" {
                        "failed"
                    } else {
                        "blocked"
                    },
                    reason: Some(code),
                });
            }
        };
        decompressed_bytes = decompressed_bytes.saturating_add(xml.len() as u64);
        if decompressed_bytes > policy.max_bytes as u64 {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some("content_decompressed_byte_limit_exceeded".into()),
            });
        }
        let parsed = if family == "xlsx" && name.starts_with("xl/worksheets/") {
            parse_xlsx_sheet_text(&xml, &shared_strings, policy.max_rows)
        } else {
            parse_xml_text(&xml).map(|text| (text, 0))
        };
        let (part_text, rows) = match parsed {
            Ok(value) => value,
            Err(_) => {
                return Ok(Extraction {
                    family: family.into(),
                    text: String::new(),
                    source_hash: String::new(),
                    truncated: false,
                    status: "blocked",
                    reason: Some("content_office_xml_invalid".into()),
                });
            }
        };
        total_rows = total_rows.saturating_add(rows);
        if total_rows > policy.max_rows {
            return Ok(Extraction {
                family: family.into(),
                text: String::new(),
                source_hash: String::new(),
                truncated: false,
                status: "blocked",
                reason: Some("content_row_limit_exceeded".into()),
            });
        }
        text.push_str(&part_text);
        text.push('\n');
        if text.chars().count() >= policy.max_chars as usize {
            break;
        }
    }
    if text.trim().is_empty() {
        return Ok(Extraction {
            family: family.into(),
            text: String::new(),
            source_hash: String::new(),
            truncated: false,
            status: "blocked",
            reason: Some("content_office_text_empty".into()),
        });
    }
    let (text, truncated) = bound_text(text, policy.max_chars as usize);
    Ok(Extraction {
        family: family.into(),
        text,
        source_hash: String::new(),
        truncated,
        status: "completed",
        reason: None,
    })
}

pub(super) fn read_zip_entry_bounded<R: Read>(
    reader: &mut R,
    max_bytes: u64,
    deadline: Instant,
) -> Result<Vec<u8>, DbError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 32 * 1024];
    loop {
        if Instant::now() > deadline {
            return Err(DbError::Validation("content_extractor_timeout".into()));
        }
        let read = reader
            .read(&mut chunk)
            .map_err(|_| DbError::Validation("content_archive_read_failed".into()))?;
        if read == 0 {
            break;
        }
        if read as u64 > max_bytes.saturating_sub(bytes.len() as u64) {
            return Err(DbError::Validation(
                "content_decompressed_byte_limit_exceeded".into(),
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

pub(super) fn enter_office_xml_depth(depth: &mut i32) -> Result<(), DbError> {
    if *depth >= OFFICE_MAX_XML_DEPTH {
        return Err(DbError::Validation(
            "content_office_xml_depth_limit_exceeded".into(),
        ));
    }
    *depth += 1;
    Ok(())
}

pub(super) fn append_office_text(target: &mut String, value: &str) -> Result<(), DbError> {
    if target.len().saturating_add(value.len()) > OFFICE_MAX_TEXT_BYTES {
        return Err(DbError::Validation(
            "content_office_text_limit_exceeded".into(),
        ));
    }
    target.push_str(value);
    Ok(())
}

pub(super) fn register_office_member_name(
    member_names: &mut HashSet<String>,
    name: &str,
) -> Result<(), DbError> {
    if member_names.insert(name.to_string()) {
        Ok(())
    } else {
        Err(DbError::Validation(
            "content_archive_duplicate_member".into(),
        ))
    }
}

pub(super) fn parse_xml_text(xml: &[u8]) -> Result<String, DbError> {
    let mut reader = XmlReader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut depth = 0_i32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(_)) => enter_office_xml_depth(&mut depth)?,
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
            }
            Ok(Event::Text(value)) => {
                let value = value
                    .xml10_content()
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                let value = unescape_xml(value.as_ref())
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut text, &value)?;
                append_office_text(&mut text, " ")?;
            }
            Ok(Event::CData(value)) => {
                append_office_text(&mut text, &String::from_utf8_lossy(&value))?;
                append_office_text(&mut text, " ")?;
            }
            Ok(Event::Comment(value)) => {
                let value = String::from_utf8_lossy(&value);
                if !value.trim().is_empty() {
                    append_office_text(&mut text, &value)?;
                    append_office_text(&mut text, " ")?;
                }
            }
            Ok(Event::GeneralRef(value)) => {
                let reference = format!(
                    "&{};",
                    value
                        .decode()
                        .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?
                );
                let value = unescape_xml(&reference)
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut text, &value)?;
                append_office_text(&mut text, " ")?;
            }
            Ok(Event::Eof) => {
                if depth != 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                break;
            }
            Ok(_) => {}
            Err(_) => return Err(DbError::Validation("content_office_xml_invalid".into())),
        }
        buffer.clear();
    }
    Ok(text)
}

pub(super) fn parse_xlsx_shared_strings(xml: &[u8]) -> Result<Vec<String>, DbError> {
    let mut reader = XmlReader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut values = Vec::new();
    let mut in_item = false;
    let mut current = String::new();
    let mut depth = 0_i32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"si" => {
                enter_office_xml_depth(&mut depth)?;
                in_item = true;
                current.clear();
            }
            Ok(Event::End(event)) if event.local_name().as_ref() == b"si" => {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
                if in_item {
                    if values.len() >= MAX_ITEMS {
                        return Err(DbError::Validation(
                            "content_office_item_limit_exceeded".into(),
                        ));
                    }
                    values.push(std::mem::take(&mut current));
                }
                in_item = false;
            }
            Ok(Event::Start(_)) => enter_office_xml_depth(&mut depth)?,
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
            }
            Ok(Event::Text(value)) if in_item => {
                let value = value
                    .xml10_content()
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                let value = unescape_xml(value.as_ref())
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut current, &value)?;
            }
            Ok(Event::CData(value)) if in_item => {
                append_office_text(&mut current, &String::from_utf8_lossy(&value))?
            }
            Ok(Event::GeneralRef(value)) if in_item => {
                let reference = format!(
                    "&{};",
                    value
                        .decode()
                        .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?
                );
                let value = unescape_xml(&reference)
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut current, &value)?;
            }
            Ok(Event::Eof) => {
                if depth != 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                break;
            }
            Ok(_) => {}
            Err(_) => return Err(DbError::Validation("content_office_xml_invalid".into())),
        }
        buffer.clear();
    }
    Ok(values)
}

pub(super) fn parse_xlsx_sheet_text(
    xml: &[u8],
    shared_strings: &[String],
    max_rows: i64,
) -> Result<(String, i64), DbError> {
    let mut reader = XmlReader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut rows = 0_i64;
    let mut in_cell = false;
    let mut cell_type = String::new();
    let mut cell_value = String::new();
    let mut in_value = false;
    let mut depth = 0_i32;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"row" => {
                enter_office_xml_depth(&mut depth)?;
                rows += 1;
                if rows > max_rows {
                    return Ok((String::new(), rows));
                }
            }
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"c" => {
                enter_office_xml_depth(&mut depth)?;
                in_cell = true;
                cell_type.clear();
                cell_value.clear();
                for attribute in event.attributes().flatten() {
                    if attribute.key.as_ref() == b"t" {
                        cell_type = String::from_utf8_lossy(&attribute.value).into_owned();
                    }
                }
            }
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"v" => {
                enter_office_xml_depth(&mut depth)?;
                in_value = true;
            }
            Ok(Event::Start(event)) if event.local_name().as_ref() == b"t" && in_cell => {
                enter_office_xml_depth(&mut depth)?;
                in_value = true;
            }
            Ok(Event::End(event))
                if event.local_name().as_ref() == b"v" || event.local_name().as_ref() == b"t" =>
            {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
                in_value = false;
            }
            Ok(Event::End(event)) if event.local_name().as_ref() == b"c" => {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
                if in_cell {
                    let value = if cell_type == "s" {
                        cell_value
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared_strings.get(index).cloned())
                            .unwrap_or_default()
                    } else {
                        cell_value.clone()
                    };
                    if !value.is_empty() {
                        text.push_str(value.trim());
                        text.push('\t');
                    }
                }
                in_cell = false;
                in_value = false;
            }
            Ok(Event::Start(_)) => enter_office_xml_depth(&mut depth)?,
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                depth -= 1;
            }
            Ok(Event::Text(value)) if in_cell => {
                let value = value
                    .xml10_content()
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                let value = unescape_xml(value.as_ref())
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut cell_value, &value)?;
            }
            Ok(Event::CData(value)) if in_cell && in_value => {
                append_office_text(&mut cell_value, &String::from_utf8_lossy(&value))?;
            }
            Ok(Event::GeneralRef(value)) if in_cell => {
                let reference = format!(
                    "&{};",
                    value
                        .decode()
                        .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?
                );
                let value = unescape_xml(&reference)
                    .map_err(|_| DbError::Validation("content_office_xml_invalid".into()))?;
                append_office_text(&mut cell_value, &value)?;
            }
            Ok(Event::Eof) => {
                if depth != 0 {
                    return Err(DbError::Validation("content_office_xml_invalid".into()));
                }
                break;
            }
            Ok(_) => {}
            Err(_) => return Err(DbError::Validation("content_office_xml_invalid".into())),
        }
        buffer.clear();
    }
    Ok((text, rows))
}

pub(super) fn bound_text(value: String, max_chars: usize) -> (String, bool) {
    let truncated = value.chars().count() > max_chars;
    (value.chars().take(max_chars).collect(), truncated)
}
