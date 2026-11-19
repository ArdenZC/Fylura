use super::super::*;
use crate::file_naming::{normalize_proposed_file_name, ExtensionChangePolicy};
use crate::settings::{AppSettings, OrganizeRootMode};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct OrganizeRootConfig {
    pub(crate) mode: OrganizeRootMode,
    pub(crate) custom_root: Option<String>,
}

impl From<&AppSettings> for OrganizeRootConfig {
    fn from(settings: &AppSettings) -> Self {
        Self {
            mode: settings.organize_root_mode,
            custom_root: settings.organize_root_path.clone(),
        }
    }
}

pub(crate) fn translate_template(template: &str, language: &str) -> String {
    if language != "zh" {
        return template.to_string();
    }

    template
        .replace("00_Inbox", "00_收件箱")
        .replace("20_Areas", "20_领域")
        .replace("40_Archive", "40_归档")
        .replace("90_Temporary", "90_临时")
        .replace("Personal/Identity", "个人/证件")
        .replace("Sensitive/Identity", "敏感/证件")
        .replace("Sensitive/Finance", "敏感/财务")
        .replace("Work/Documents", "工作/文档")
        .replace("Media/Screenshots", "媒体/截图")
        .replace("Career", "职业")
        .replace("Teaching", "教学")
        .replace("Finance", "财务")
        .replace("Study", "学业")
        .replace("Work", "工作")
        .replace("Sensitive", "敏感")
        .replace("Projects", "项目")
        .replace("Installers", "安装包")
        .replace("Media/Images", "媒体/图片")
        .replace("Media/Videos", "媒体/视频")
        .replace("Media/Audio", "媒体/音频")
        .replace("Documents/Spreadsheets", "文档/表格")
        .replace("Documents/Presentations", "文档/演示")
        .replace("Documents/Identity", "文档/证件")
        .replace("Documents", "文档")
        .replace("Images", "图片")
        .replace("Videos", "视频")
        .replace("Audio", "音频")
        .replace("Screenshots", "截图")
        .replace("Archives", "压缩包")
        .replace("Temporary", "临时")
        .replace("Archive", "归档")
        .replace("Packages", "软件包")
}

pub(crate) fn build_target_path(
    row: &IndexedFileRow,
    file_type: &str,
    template: Option<&str>,
    folder_naming_language: &str,
    organize_root: &OrganizeRootConfig,
) -> String {
    let Some(template) = template.filter(|value| !value.is_empty()) else {
        return String::new();
    };
    let year = unix_seconds_to_iso(row.mtime)
        .get(0..4)
        .unwrap_or("1970")
        .to_string();
    let translated_template = translate_template(template, folder_naming_language);
    let resolved = translated_template
        .replace("{year}", &year)
        .replace("{type}", file_type);
    let mut target = target_base_directory(row, organize_root);
    for segment in resolved
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
    {
        target.push(segment);
    }
    target.to_string_lossy().replace('\\', "/")
}

fn target_base_directory(row: &IndexedFileRow, organize_root: &OrganizeRootConfig) -> PathBuf {
    match organize_root.mode {
        OrganizeRootMode::CurrentFolder => PathBuf::from(parent_directory(&row.path)),
        OrganizeRootMode::FyluraFolder => {
            let mut target = PathBuf::from(parent_directory(&row.path));
            target.push("Fylura");
            target
        }
        OrganizeRootMode::CustomRoot => organize_root
            .custom_root
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(parent_directory(&row.path))),
    }
}

pub(crate) fn build_suggested_name(row: &IndexedFileRow, template: Option<&str>) -> String {
    let Some(template) = template.filter(|value| !value.is_empty()) else {
        return row.name.clone();
    };
    let basename = clean_name(file_stem(&row.name, &row.extension));
    let date = unix_seconds_to_iso(row.mtime)
        .get(0..10)
        .unwrap_or("1970-01-01")
        .replace('-', "");
    let extension = row.extension.trim_start_matches('.');
    let rendered = template
        .replace("{basename}", &basename)
        .replace("{date}", &date)
        .replace("{extension}", extension);
    let candidate = append_extension_once(&rendered, extension);
    normalize_proposed_file_name(
        &row.name,
        &row.extension,
        &candidate,
        row.is_dir,
        ExtensionChangePolicy::Preserve,
    )
    .unwrap_or_else(|_| row.name.clone())
}

fn append_extension_once(rendered: &str, extension: &str) -> String {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty() {
        return rendered.to_string();
    }
    let suffix = format!(".{extension}");
    if rendered
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        rendered.to_string()
    } else {
        format!("{rendered}{suffix}")
    }
}

fn file_stem<'a>(name: &'a str, extension: &str) -> &'a str {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty() {
        return name;
    }
    let suffix = format!(".{extension}");
    if name.to_lowercase().ends_with(&suffix.to_lowercase()) && name.len() > suffix.len() {
        &name[..name.len() - suffix.len()]
    } else {
        name
    }
}

fn clean_name(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;
    for character in value.trim().chars() {
        if character.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&character) {
            output.extend(character.to_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            output.push('_');
            last_was_separator = true;
        }
    }
    output.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_placeholder_does_not_duplicate_the_extension() {
        assert_eq!(append_extension_once("report.pdf", "pdf"), "report.pdf");
        assert_eq!(append_extension_once("report", "pdf"), "report.pdf");
    }
}
