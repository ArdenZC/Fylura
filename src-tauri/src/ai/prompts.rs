use super::{classification::AIClassificationInputFile, cleanup::AICleanupInputCandidate};

pub fn ai_file_classification_system_prompt(enable_thinking: bool) -> String {
    let mut lines = vec![
        "You are the Fylura AI file classification engine.",
        "Classify from metadata only; never infer file content.",
        "Return JSON only; 只返回 JSON；JSON 输出示例必须遵循下方 shape；no Markdown, code fences, thinking, <think>, explanations, or chain-of-thought.",
        "不得输出 Markdown、不得输出代码块、不得输出思考内容；最终 content 必须是完整 JSON。",
        "Never suggest direct deletion. Use Review when uncertain.",
        "targetTemplate is a relative target directory only, never a file path or absolute path.",
        "suggestedName must preserve the original file extension exactly.",
        "For files, suggestedName must either be empty or contain the complete filename with the same extension.",
        "Never remove, replace, or invent a file extension.",
        "For Windows shortcuts, preserve .lnk, .url, and .appref-ms.",
        "Return the same refId exactly. Do not use path as id, invent ids, or return path in id.",
        "Sensitive or low-confidence results must set requiresConfirmation=true.",
        "Enums: fileType=Document|Image|Video|Audio|Code|ArchivePackage|Installer|Spreadsheet|Presentation|Other; purpose=Project|Teaching|Study|Work|Personal|Career|Finance|Identity|Media|Installer|Temporary|Archive|Unknown; lifecycle=Inbox|Active|Reference|Archive|Disposable|Duplicate|Sensitive; riskLevel=Normal|Sensitive|System|Unknown; suggestedAction=Keep|Move|MoveAndRename|Archive|Review|DeleteCandidate.",
        "Shape: {\"classifications\":[{\"refId\":\"f1\",\"fileType\":\"Document\",\"purpose\":\"Teaching\",\"lifecycle\":\"Active\",\"context\":\"Scala\",\"riskLevel\":\"Normal\",\"suggestedAction\":\"Move\",\"targetTemplate\":\"Teaching/Scala\",\"suggestedName\":\"Install_Package.lnk\",\"confidence\":0.92,\"reason\":\"short reason\",\"keywords\":[\"Scala\"],\"requiresConfirmation\":false}]}",
    ];
    if !enable_thinking {
        lines.push("Do not output thinking, reasoning traces, chain-of-thought, or analysis.");
    }
    lines.join("\n")
}

pub fn build_ai_classification_prompt(
    files: &[AIClassificationInputFile],
    learned_rules: &[String],
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "task": "classifyFiles",
        "learnedRulesInstruction": "用户确认/纠正形成的习惯，仅作参考；不得违反安全规则。",
        "learnedRules": learned_rules,
        "files": files,
    }))
    .map_err(|error| format!("failed to build AI classification prompt: {error}"))
}

pub fn ai_cleanup_analysis_system_prompt(enable_thinking: bool) -> String {
    let mut lines = vec![
        "You are the Fylura file cleanup risk analyzer.",
        "You only analyze existing cleanup candidates. You do not execute cleanup.",
        "You must not directly delete, move, or trash files.",
        "You must not suggest bypassing Fylura Safe Trash, preview, confirmation, or restore flows.",
        "Be conservative. If uncertain, return tier = Review or tier = Caution.",
        "Return only JSON.",
        "If you are a thinking model, do not output thinking content.",
        "Do not output <think> tags.",
        "Do not output reasoning process.",
        "Only output the final JSON.",
        "Do not output Markdown.",
        "Do not output code fences.",
        "Allowed tier values: Safe, Review, Caution.",
        "Allowed suggestedAction values: MoveToTrash, Reveal, UninstallAdvice, AppInternalCleanup, None.",
        "System paths must not be cleaned.",
        "Windows, Program Files, and ProgramData paths must not be cleaned.",
        "AppData should not default to MoveToTrash.",
        "Browser profiles must be Caution.",
        "Chat application databases must be Caution.",
        "Database files must be Caution.",
        "Virtual machine images must be Caution.",
        "Unknown large files can only be Review.",
        "Caution items must set selectedByDefault = false.",
        "If the original trashAllowed is false, you must not change trashAllowed to true.",
        "Do not invent candidateId values.",
        "Do not invent or rewrite paths.",
        "You may improve category, reason, and riskNote text.",
        "Return JSON in this exact shape: {\"analyses\":[{\"candidateId\":\"id\",\"tier\":\"Safe\",\"category\":\"可重新生成的依赖目录\",\"suggestedAction\":\"MoveToTrash\",\"confidence\":0.95,\"reason\":\"该目录是 node_modules，通常可通过包管理器重新生成。\",\"riskNote\":\"如果存在 npm link、本地补丁或未提交依赖改动，清理前应确认。\",\"trashAllowed\":true,\"selectedByDefault\":true}]}",
    ];
    if !enable_thinking {
        lines.push("Do not output thinking, reasoning traces, chain-of-thought, or analysis.");
    }
    lines.join("\n")
}

pub fn build_ai_cleanup_analysis_prompt(
    candidates: &[AICleanupInputCandidate],
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "task": "analyzeCleanupCandidates",
        "candidates": candidates,
    }))
    .map_err(|error| format!("failed to build AI cleanup analysis prompt: {error}"))
}

pub(crate) fn clean_ai_json_text(content: &str) -> String {
    let without_thinking = strip_think_blocks(content);
    let mut text = without_thinking.trim().to_string();
    text = strip_markdown_fence(&text);
    if let Some(index) = first_json_start(&text) {
        text = text[index..].to_string();
    }
    text.trim().to_string()
}

pub(crate) fn extract_first_json_value(content: &str) -> Option<String> {
    let content = clean_ai_json_text(content);
    let mut start = None;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut expected_close = Vec::new();

    for (index, ch) in content.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => {
                if start.is_none() {
                    start = Some(index);
                }
                depth += 1;
                expected_close.push(if ch == '{' { '}' } else { ']' });
            }
            '}' | ']' if depth > 0 && expected_close.last().copied() == Some(ch) => {
                expected_close.pop();
                depth -= 1;
                if depth == 0 {
                    let start = start?;
                    return Some(content[start..=index].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn strip_think_blocks(content: &str) -> String {
    let mut output = content.to_string();
    loop {
        let lower = output.to_ascii_lowercase();
        let Some(start) = lower.find("<think>") else {
            break;
        };
        let Some(end) = lower[start + "<think>".len()..].find("</think>") else {
            output.replace_range(start.., "");
            break;
        };
        let end = start + "<think>".len() + end + "</think>".len();
        output.replace_range(start..end, "");
    }
    output
}

fn strip_markdown_fence(content: &str) -> String {
    let trimmed = content.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let Some(first_newline) = trimmed.find('\n') else {
        return trimmed.trim_matches('`').trim().to_string();
    };
    let body = &trimmed[first_newline + 1..];
    let body = body.trim();
    if let Some(stripped) = body.strip_suffix("```") {
        stripped.trim().to_string()
    } else {
        body.to_string()
    }
}

fn first_json_start(content: &str) -> Option<usize> {
    let object = content.find('{');
    let array = content.find('[');
    match (object, array) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(index), None) | (None, Some(index)) => Some(index),
        (None, None) => None,
    }
}
