//! #494: test-only A/B file-name relevance experiment.
//!
//! Nothing in this module is reachable from the shipping search path. It does
//! not change `search.rs`, the SQLite schema, the FTS triggers, the candidate
//! window, the source authority or the default index scope.
//!
//! Three result channels are kept deliberately separate because they answer
//! three different product questions:
//!
//! * `production` – the current five-tier behaviour, reproduced by an oracle
//!   that is written independently of `search.rs` and cross-checked against it.
//! * `name_path`  – the *proposed* filename UX order (exact → prefix/word
//!   boundary → inner substring → path-only). This is NOT an approved product
//!   contract; it exists so the Owner can compare it with today's behaviour.
//! * `extension`  – extension exact/prefix. The Owner has not approved a
//!   priority for this class, so it is reported as its own candidate set and is
//!   never merged into the ranked `name_path` channel.
//!
//! Candidate A enumerates FTS5 postings without BM25 ordering and ranks in
//! Rust. Candidate B builds a disposable, snapshot-scoped, name-only UTF-8 view
//! with contiguous bytes and bounded Top-K. B's full-coverage cost includes an
//! explicit SQLite completion step for path/extension; that cost is measured
//! separately and is never reported as "B latency".

pub(super) mod bench;
pub(super) mod candidate_a;
pub(super) mod candidate_b;
pub(super) mod cases;
pub(super) mod fixtures;
pub(super) mod oracle;
pub(super) mod scale;

use std::cmp::Ordering;

/// The production candidate-window contract in `search.rs`. A test asserts this
/// copy still matches the production constant so the evidence cannot silently
/// describe a different window.
pub(super) const MAX_WINDOW: usize = 4_096;
/// Default page size used by every comparison in this experiment.
pub(super) const PAGE_LIMIT: u32 = 80;
pub(super) const WARMUP: usize = 5;
pub(super) const SAMPLES: usize = 30;

/// Relevance classes for the *proposed* filename UX.
///
/// The declaration order is the proposed priority order. `ExtensionExact` and
/// `ExtensionPrefix` sort after every name/path class only because a total
/// order is required for the report; they are emitted through a separate
/// channel and their real priority is an open Owner decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(super) enum Class {
    NameExact,
    NamePrefixOrWordBoundary,
    NameInner,
    PathOnly,
    ExtensionExact,
    ExtensionPrefix,
}

impl Class {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Class::NameExact => "name_exact",
            Class::NamePrefixOrWordBoundary => "name_prefix_or_word_boundary",
            Class::NameInner => "name_inner_substring",
            Class::PathOnly => "path_only",
            Class::ExtensionExact => "extension_exact",
            Class::ExtensionPrefix => "extension_prefix",
        }
    }

    pub(super) fn is_extension(self) -> bool {
        matches!(self, Class::ExtensionExact | Class::ExtensionPrefix)
    }
}

/// One ranked hit. `Ord` is "better sorts first" so `Vec::sort` produces the
/// user-visible order directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Hit {
    pub id: String,
    pub class: Class,
    pub modified: Option<i64>,
}

impl Ord for Hit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.class
            .cmp(&other.class)
            .then_with(|| modified_desc(self.modified, other.modified))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialOrd for Hit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `ORDER BY modified_at_fs DESC` in SQLite sorts NULL last, because NULL is
/// the smallest value. Reproduce that instead of relying on `Option` ordering.
pub(super) fn modified_desc(left: Option<i64>, right: Option<i64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// SQLite's built-in `lower()` folds ASCII only. Production tier predicates
/// compare `name_normalized` against `lower(?1)`, so a non-ASCII uppercase
/// query does not reach the exact/prefix tiers at all. The oracle has to
/// reproduce that quirk rather than "fix" it silently.
pub(super) fn sqlite_lower(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_uppercase() {
                character.to_ascii_lowercase()
            } else {
                character
            }
        })
        .collect()
}

/// Unicode-aware folding used by the proposed ranking and by candidate B.
/// `name_normalized` is written by Rust `to_lowercase()` at insert time, so the
/// proposed classes must use the same folding to agree with stored values.
pub(super) fn fold(value: &str) -> String {
    value.to_lowercase()
}

/// A filename separator for word-boundary purposes. Filenames use `-`, `_`,
/// `.` and spaces as separators; all of them are non-alphanumeric.
pub(super) fn is_word_character(character: char) -> bool {
    character.is_alphanumeric()
}

/// True when `needle` occurs in `haystack` at a word boundary, i.e. at the
/// start of the name or directly after a separator. The start-of-name case is
/// also a prefix match, so callers treat both as one class.
pub(super) fn contains_at_word_boundary(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    for (offset, _) in haystack.match_indices(needle) {
        if offset == 0 {
            return true;
        }
        let previous = haystack[..offset].chars().next_back();
        if previous.is_some_and(|character| !is_word_character(character)) {
            return true;
        }
    }
    false
}

/// Proposed filename class for one row, evaluated over the stored
/// `name_normalized` / `path_normalized` / `extension` facts.
///
/// `query_folded` must already be folded with [`fold`].
pub(super) fn proposed_class(
    name_normalized: &str,
    path_normalized: &str,
    extension: &str,
    query_folded: &str,
    query_sql_lower: &str,
) -> Option<Class> {
    if query_folded.is_empty() {
        return None;
    }
    if name_normalized == query_folded {
        Some(Class::NameExact)
    } else if name_normalized.starts_with(query_folded)
        || contains_at_word_boundary(name_normalized, query_folded)
    {
        Some(Class::NamePrefixOrWordBoundary)
    } else if name_normalized.contains(query_folded) {
        Some(Class::NameInner)
    } else if path_normalized.contains(query_folded) {
        Some(Class::PathOnly)
    } else if extension == query_sql_lower {
        Some(Class::ExtensionExact)
    } else if !query_sql_lower.is_empty() && extension.starts_with(query_sql_lower) {
        Some(Class::ExtensionPrefix)
    } else {
        None
    }
}

/// Extension channel membership. Kept separate from [`proposed_class`] because
/// an extension match is almost always shadowed by a name match (the filename
/// literally ends with `.ext`), so folding it into the ranked channel would
/// make the class unreachable and would silently reorder real extension hits.
pub(super) fn extension_class(extension: &str, query_sql_lower: &str) -> Option<Class> {
    if query_sql_lower.is_empty() {
        return None;
    }
    if extension == query_sql_lower {
        Some(Class::ExtensionExact)
    } else if extension.starts_with(query_sql_lower) {
        Some(Class::ExtensionPrefix)
    } else {
        None
    }
}

/// Deduplicate by stable id, keeping the best class, then sort the full set.
///
/// Deliberately unbounded: an oracle that truncates to the candidate window
/// cannot prove that the window is a *prefix* of the true order, and a truncated
/// oracle hides exactly the truncation bugs this experiment is looking for.
pub(super) fn rank_all(hits: impl IntoIterator<Item = Hit>) -> Vec<Hit> {
    let mut unique = std::collections::HashMap::<String, Hit>::new();
    for hit in hits {
        match unique.get_mut(&hit.id) {
            Some(previous) => {
                if hit < *previous {
                    *previous = hit;
                }
            }
            None => {
                unique.insert(hit.id.clone(), hit);
            }
        }
    }
    let mut ranked = unique.into_values().collect::<Vec<_>>();
    ranked.sort();
    ranked
}

/// First `window` entries of a ranked list.
pub(super) fn window_of(ranked: &[Hit], window: usize) -> &[Hit] {
    &ranked[..ranked.len().min(window)]
}

#[cfg(test)]
mod window_contract_tests {
    use super::MAX_WINDOW;
    use crate::global_index::search::diagnostic_candidate_window;

    #[test]
    fn issue494_recorded_window_matches_production_contract() {
        assert_eq!(
            MAX_WINDOW,
            diagnostic_candidate_window(),
            "#494 evidence must describe the real production candidate window"
        );
    }
}
