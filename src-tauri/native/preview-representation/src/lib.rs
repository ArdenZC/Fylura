//! Pure bounded Text/Source Code/Markdown representation kernel.
//!
//! This crate intentionally has no preview session, provider registry, source
//! resolver, read gate, scheduler, Tauri, SQLite, COM, HWND or filesystem
//! authority. App providers and the Windows handler call these functions only
//! after their own authority has supplied bounded bytes.

use ammonia::Builder as HtmlSanitizer;
use pulldown_cmark::{html, Options, Parser};
use std::{
    collections::{HashMap, HashSet},
    str,
};
use thiserror::Error;

pub const MAX_MARKDOWN_HTML_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentationCompleteness {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub text: String,
    pub completeness: RepresentationCompleteness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeRepresentation {
    Text {
        text: String,
        language: Option<String>,
    },
    SafeHtml {
        html: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RepresentationError {
    #[error("captured source is corrupt or not safe text")]
    CorruptSource,
    #[error("bounded representation output is too large")]
    OutputTooLarge,
}

/// Inert metadata hints. They select presentation only and never identify a
/// source, path, file handle or provider.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepresentationHint {
    pub extension: Option<String>,
    pub media_type: Option<String>,
}

pub fn decode_text(bytes: &[u8], complete: bool) -> Result<DecodedText, RepresentationError> {
    let completeness = if complete {
        RepresentationCompleteness::Complete
    } else {
        RepresentationCompleteness::Partial
    };
    let text = match str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) if !complete && error.error_len().is_none() => {
            str::from_utf8(&bytes[..error.valid_up_to()])
                .map_err(|_| RepresentationError::CorruptSource)?
        }
        Err(_) => return Err(RepresentationError::CorruptSource),
    };
    if text.chars().any(is_obvious_binary_character) {
        return Err(RepresentationError::CorruptSource);
    }
    Ok(DecodedText {
        text: text.strip_prefix('\u{feff}').unwrap_or(text).to_owned(),
        completeness,
    })
}

pub fn render_text(
    bytes: &[u8],
    complete: bool,
    language: Option<&str>,
) -> Result<(SafeRepresentation, RepresentationCompleteness), RepresentationError> {
    let decoded = decode_text(bytes, complete)?;
    Ok((
        SafeRepresentation::Text {
            text: decoded.text,
            language: language.map(str::to_owned),
        },
        decoded.completeness,
    ))
}

pub fn render_markdown(
    bytes: &[u8],
    complete: bool,
) -> Result<(SafeRepresentation, RepresentationCompleteness), RepresentationError> {
    let decoded = decode_text(bytes, complete)?;
    let html = render_safe_markdown(&decoded.text)?;
    Ok((SafeRepresentation::SafeHtml { html }, decoded.completeness))
}

pub fn is_markdown_hint(hint: &RepresentationHint) -> bool {
    matches!(
        normalized_extension(hint).as_deref(),
        Some("md" | "markdown" | "mdown" | "mkdn")
    ) || matches!(
        normalized_media_type(hint).as_deref(),
        Some("text/markdown" | "text/x-markdown" | "application/markdown")
    )
}

pub fn is_plain_text_hint(hint: &RepresentationHint) -> bool {
    let extension = normalized_extension(hint);
    let media_type = normalized_media_type(hint);
    if extension.as_deref().is_some_and(is_binary_extension) {
        return false;
    }
    extension.as_deref().is_some_and(is_known_text_extension)
        || media_type.as_deref().is_some_and(is_text_media_type)
        || media_type
            .as_deref()
            .is_some_and(|value| matches!(value, "application/json" | "application/xml"))
}

pub fn source_code_language(hint: &RepresentationHint) -> Option<&'static str> {
    if let Some(extension) = normalized_extension(hint) {
        let language = match extension.as_str() {
            "bat" => "batch",
            "c" => "c",
            "cc" | "cpp" | "cxx" | "hpp" => "cpp",
            "css" => "css",
            "h" => "c",
            "htm" | "html" => "html",
            "java" => "java",
            "js" | "jsx" => "javascript",
            "json" => "json",
            "kt" | "kts" => "kotlin",
            "php" => "php",
            "ps1" => "powershell",
            "py" => "python",
            "rb" => "ruby",
            "rs" => "rust",
            "sh" => "shell",
            "sql" => "sql",
            "swift" => "swift",
            "svelte" => "svelte",
            "toml" => "toml",
            "ts" | "tsx" => "typescript",
            "vue" => "vue",
            "xml" => "xml",
            "yaml" | "yml" => "yaml",
            _ => return None,
        };
        return Some(language);
    }
    match normalized_media_type(hint).as_deref() {
        Some("application/json") => Some("json"),
        Some("application/xml") | Some("text/xml") => Some("xml"),
        Some("text/css") => Some("css"),
        Some("text/html") => Some("html"),
        Some("text/x-python") => Some("python"),
        Some("text/x-rust") => Some("rust"),
        Some("text/x-shellscript") => Some("shell"),
        Some("text/typescript") => Some("typescript"),
        _ => None,
    }
}

fn is_obvious_binary_character(character: char) -> bool {
    character == '\0'
        || (character.is_control() && !matches!(character, '\t' | '\n' | '\r' | '\u{000c}'))
}

fn render_safe_markdown(text: &str) -> Result<String, RepresentationError> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let mut raw_html = String::with_capacity(text.len().min(MAX_MARKDOWN_HTML_BYTES));
    html::push_html(&mut raw_html, Parser::new_ext(text, options));
    if raw_html.len() > MAX_MARKDOWN_HTML_BYTES {
        return Err(RepresentationError::OutputTooLarge);
    }

    let allowed_tags: HashSet<&str> = [
        "blockquote",
        "br",
        "code",
        "del",
        "em",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "li",
        "ol",
        "p",
        "pre",
        "strong",
        "table",
        "tbody",
        "td",
        "th",
        "thead",
        "tr",
        "ul",
    ]
    .into_iter()
    .collect();
    let sanitized = HtmlSanitizer::default()
        .tags(allowed_tags)
        .tag_attributes(HashMap::new())
        .generic_attributes(HashSet::new())
        .url_schemes(HashSet::new())
        .clean(&raw_html)
        .to_string();
    if sanitized.len() > MAX_MARKDOWN_HTML_BYTES {
        return Err(RepresentationError::OutputTooLarge);
    }
    Ok(sanitized)
}

fn normalized_extension(hint: &RepresentationHint) -> Option<String> {
    hint.extension
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn normalized_media_type(hint: &RepresentationHint) -> Option<String> {
    hint.media_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn is_text_media_type(value: &str) -> bool {
    value == "text/plain"
        || value.starts_with("text/")
        || value == "application/json"
        || value == "application/xml"
}

fn is_known_text_extension(value: &str) -> bool {
    matches!(
        value,
        "bat"
            | "c"
            | "cc"
            | "cfg"
            | "conf"
            | "cpp"
            | "csv"
            | "css"
            | "cxx"
            | "env"
            | "gitignore"
            | "h"
            | "hpp"
            | "htm"
            | "html"
            | "ini"
            | "java"
            | "js"
            | "json"
            | "jsx"
            | "kt"
            | "kts"
            | "log"
            | "markdown"
            | "md"
            | "mdown"
            | "mkdn"
            | "php"
            | "ps1"
            | "py"
            | "rb"
            | "rs"
            | "sh"
            | "sql"
            | "swift"
            | "svelte"
            | "text"
            | "toml"
            | "ts"
            | "tsx"
            | "tsv"
            | "txt"
            | "vue"
            | "xml"
            | "yaml"
            | "yml"
    )
}

fn is_binary_extension(value: &str) -> bool {
    matches!(
        value,
        "7z" | "avi"
            | "bmp"
            | "class"
            | "dll"
            | "doc"
            | "docx"
            | "epub"
            | "gif"
            | "gz"
            | "ico"
            | "jpeg"
            | "jpg"
            | "mkv"
            | "mov"
            | "mp3"
            | "mp4"
            | "otf"
            | "pdf"
            | "png"
            | "rar"
            | "tar"
            | "wav"
            | "webp"
            | "woff"
            | "woff2"
            | "xls"
            | "xlsx"
            | "zip"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_contract_preserves_bom_unicode_crlf_empty_and_partial_utf8() {
        assert_eq!(
            decode_text("\u{feff}你好\r\n".as_bytes(), true).unwrap(),
            DecodedText {
                text: "你好\r\n".to_string(),
                completeness: RepresentationCompleteness::Complete,
            }
        );
        assert_eq!(decode_text(b"", true).unwrap().text, "");
        assert_eq!(decode_text(&[0xe4, 0xb8], false).unwrap().text, "");
        assert_eq!(
            decode_text(&[0xe4, 0xb8], true),
            Err(RepresentationError::CorruptSource)
        );
    }

    #[test]
    fn markdown_is_bounded_and_has_no_active_authority() {
        let (representation, completeness) = render_markdown(
            b"# title\n\n<script>alert(1)</script>\n\n[link](https://example.test) ![img](https://example.test/x.png)",
            true,
        )
        .unwrap();
        assert_eq!(completeness, RepresentationCompleteness::Complete);
        let SafeRepresentation::SafeHtml { html } = representation else {
            panic!("expected safe html");
        };
        assert!(!html.contains("script"));
        assert!(!html.contains("href="));
        assert!(!html.contains("src="));
        assert!(!html.contains("onerror"));
    }

    #[test]
    fn rustsec_0373_css_url_case_variants_cannot_load_resources() {
        for function in ["url", "URL", "uRl", "Url"] {
            let input = format!(
                r#"<p style='background-image:{function}("https://evil.example/pixel");'>safe</p>"#
            );
            let (representation, _) = render_markdown(input.as_bytes(), true).unwrap();
            let SafeRepresentation::SafeHtml { html } = representation else {
                panic!("expected safe html");
            };
            assert!(html.contains("safe"));
            assert!(!html.contains("style="));
            assert!(!html.contains("evil.example"));

            // Exercise the patched dependency's CSS filter as well as our
            // stricter production policy, which never admits style attributes.
            let cleaned = HtmlSanitizer::default()
                .filter_style_properties(["background-image"].into())
                .add_generic_attributes(&["style"])
                .url_schemes(["cid"].into())
                .clean(&input)
                .to_string();
            assert!(!cleaned.contains("evil.example"), "{function}: {cleaned}");
        }
    }

    #[test]
    fn markdown_renders_supported_document_structures_without_raw_syntax() {
        let (representation, _) = render_markdown(
            b"# Heading\n\n**Bold**\n\n- one\n- two\n\n> quote\n\n```rust\nlet answer = 42;\n```\n\n| Name | Value |\n| --- | --- |\n| bounded | safe |",
            true,
        )
        .unwrap();
        let SafeRepresentation::SafeHtml { html } = representation else {
            panic!("expected safe html");
        };
        for tag in [
            "<h1>",
            "<strong>",
            "<ul>",
            "<blockquote>",
            "<pre><code>",
            "<table>",
        ] {
            assert!(html.contains(tag), "missing {tag}: {html}");
        }
        assert!(!html.contains("# Heading"));
        assert!(!html.contains("**Bold**"));
        assert!(!html.contains("```"));
    }

    #[test]
    fn language_and_hint_mapping_is_inert() {
        let hint = RepresentationHint {
            extension: Some(".RS".to_string()),
            media_type: None,
        };
        assert_eq!(source_code_language(&hint), Some("rust"));
        assert!(!is_markdown_hint(&hint));
        assert!(is_plain_text_hint(&hint));
    }
}
