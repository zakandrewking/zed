use crate::capture_summary::{capture_request_directories, load_capture_summary};
use anyhow::{Context as _, Result};
use clap::Args;
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use zeta_prompt::{
    ParsedOutput, ZetaFormat, ZetaPromptInput, parse_zeta2_model_output, parsed_output_to_patch,
    resolve_cursor_region,
};

#[derive(Debug, Args, Clone)]
pub struct ReplayOutputSafetyArgs {
    /// Fixture or capture directory containing request-* subdirectories.
    #[arg(long)]
    pub directory: PathBuf,
    /// Zeta prompt format used to parse outputs.
    #[arg(long)]
    pub format: Option<String>,
    /// Include generated nasty cases for each request.
    #[arg(long, default_value_t = true)]
    pub include_synthetic: bool,
}

#[derive(Debug)]
struct OutputSafetyResult {
    request_name: String,
    case_name: String,
    safe_to_apply: bool,
    parse_ok: bool,
    patch_ok: bool,
    raw_output_bytes: usize,
    old_editable_bytes: Option<usize>,
    new_editable_bytes: Option<usize>,
    deletion_ratio: Option<f64>,
    reasons: Vec<String>,
}

pub fn run_replay_output_safety(
    args: &ReplayOutputSafetyArgs,
    output_path: Option<&PathBuf>,
) -> Result<()> {
    let format = args
        .format
        .as_deref()
        .map(ZetaFormat::parse)
        .transpose()?
        .unwrap_or_default();
    let capture_directories = capture_request_directories(&args.directory)?;
    let mut results = Vec::new();

    for capture_directory in capture_directories {
        let capture = load_capture_summary(&capture_directory)
            .with_context(|| format!("failed to load {}", capture_directory.display()))?;
        let Some(request) = capture.parsed_request.as_ref() else {
            continue;
        };

        if let Some(response) = capture.parsed_response.as_ref() {
            results.push(assess_output(
                &capture.name,
                "captured-response",
                &response.output,
                format,
                &request.input,
            ));
        }

        if args.include_synthetic {
            for synthetic_case in synthetic_cases(format, &request.input) {
                results.push(assess_output(
                    &capture.name,
                    synthetic_case.name,
                    &synthetic_case.output,
                    format,
                    &request.input,
                ));
            }
        }
    }

    let output = render_safety_report(&args.directory, format, &results);
    if let Some(output_path) = output_path {
        std::fs::write(output_path, output).with_context(|| {
            format!("failed to write safety report to {}", output_path.display())
        })?;
    } else {
        print!("{output}");
    }

    Ok(())
}

#[derive(Debug)]
struct SyntheticCase {
    name: &'static str,
    output: String,
}

fn synthetic_cases(format: ZetaFormat, input: &ZetaPromptInput) -> Vec<SyntheticCase> {
    let old_editable_region = expected_old_editable_region(format, input)
        .map(str::to_string)
        .unwrap_or_default();
    vec![
        SyntheticCase {
            name: "identity-old-editable",
            output: old_editable_region,
        },
        SyntheticCase {
            name: "raw-sentinel-leak",
            output: "<|fim_prefix|>\n<<<<<<< CURRENT\nstale\n>>>>>>> UPDATED\n".to_string(),
        },
        SyntheticCase {
            name: "giant-deletion",
            output: String::new(),
        },
        SyntheticCase {
            name: "malformed-marker-span",
            output: "<|marker_999|>orphan marker output<[end_of_sentence]>".to_string(),
        },
    ]
}

fn assess_output(
    request_name: &str,
    case_name: &str,
    raw_output: &str,
    format: ZetaFormat,
    input: &ZetaPromptInput,
) -> OutputSafetyResult {
    let mut reasons = Vec::new();
    let parsed = match parse_zeta2_model_output(raw_output, format, input) {
        Ok(parsed) => parsed,
        Err(error) => {
            return OutputSafetyResult {
                request_name: request_name.to_string(),
                case_name: case_name.to_string(),
                safe_to_apply: false,
                parse_ok: false,
                patch_ok: false,
                raw_output_bytes: raw_output.len(),
                old_editable_bytes: None,
                new_editable_bytes: None,
                deletion_ratio: None,
                reasons: vec![format!("parse failed: {error:#}")],
            };
        }
    };

    let patch_ok = match parsed_output_to_patch(input, parsed.clone()) {
        Ok(_) => true,
        Err(error) => {
            reasons.push(format!("patch conversion failed: {error:#}"));
            false
        }
    };

    let excerpt = input.cursor_excerpt.as_ref();
    let old_editable_region = excerpt.get(parsed.range_in_excerpt.clone());
    if old_editable_region.is_none() {
        reasons.push(format!(
            "parsed range {:?} is outside cursor excerpt length {}",
            parsed.range_in_excerpt,
            excerpt.len()
        ));
    }

    let expected_range = expected_editable_range(format, input);
    if parsed.range_in_excerpt != expected_range {
        reasons.push(format!(
            "parsed range {:?} does not match expected editable range {:?}",
            parsed.range_in_excerpt, expected_range
        ));
    }

    if leaked_sentinel_in_applied_text(&parsed) {
        reasons.push("raw sentinel leaked into applied text".to_string());
    }

    let old_editable_bytes = old_editable_region.map(str::len);
    let new_editable_bytes = Some(parsed.new_editable_region.len());
    let deletion_ratio =
        old_editable_region.map(|old| deletion_ratio(old.len(), parsed.new_editable_region.len()));

    if let (Some(old), Some(new)) = (old_editable_bytes, new_editable_bytes)
        && is_suspicious_giant_deletion(old, new)
    {
        reasons.push(format!(
            "suspicious giant deletion: old_bytes={old} new_bytes={new}"
        ));
    }

    OutputSafetyResult {
        request_name: request_name.to_string(),
        case_name: case_name.to_string(),
        safe_to_apply: patch_ok && reasons.is_empty(),
        parse_ok: true,
        patch_ok,
        raw_output_bytes: raw_output.len(),
        old_editable_bytes,
        new_editable_bytes,
        deletion_ratio,
        reasons,
    }
}

fn expected_editable_range(format: ZetaFormat, input: &ZetaPromptInput) -> Range<usize> {
    let (_, editable_range_in_context, context_range, _) = resolve_cursor_region(input, format);
    context_range.start + editable_range_in_context.start
        ..context_range.start + editable_range_in_context.end
}

fn expected_old_editable_region(format: ZetaFormat, input: &ZetaPromptInput) -> Option<&str> {
    input
        .cursor_excerpt
        .get(expected_editable_range(format, input))
}

fn leaked_sentinel_in_applied_text(parsed: &ParsedOutput) -> bool {
    RAW_SENTINELS
        .iter()
        .any(|sentinel| parsed.new_editable_region.contains(sentinel))
}

const RAW_SENTINELS: &[&str] = &[
    "<|fim_prefix|>",
    "<|fim_suffix|>",
    "<|fim_middle|>",
    "<|marker_",
    "<|marker+",
    "<|marker-",
    "<[end",
    "<<<<<<< CURRENT",
    ">>>>>>> UPDATED",
    "<|set|>",
    "<|insert|>",
    "<|no_edits|>",
];

fn deletion_ratio(old_bytes: usize, new_bytes: usize) -> f64 {
    if old_bytes == 0 || new_bytes >= old_bytes {
        return 0.0;
    }

    (old_bytes - new_bytes) as f64 / old_bytes as f64
}

fn is_suspicious_giant_deletion(old_bytes: usize, new_bytes: usize) -> bool {
    if new_bytes >= old_bytes {
        return false;
    }

    let removed_bytes = old_bytes - new_bytes;
    (old_bytes >= 200 && new_bytes <= old_bytes / 4) || removed_bytes >= 1024
}

fn render_safety_report(
    directory: &Path,
    format: ZetaFormat,
    results: &[OutputSafetyResult],
) -> String {
    let summary = safety_summary(results);
    let mut output = String::new();
    _ = writeln!(output, "# Output Safety Replay Report");
    _ = writeln!(output);
    _ = writeln!(output, "Directory: `{}`", directory.display());
    _ = writeln!(output, "Format: `{format}`");
    _ = writeln!(output);
    _ = writeln!(output, "## Summary");
    _ = writeln!(output);
    _ = writeln!(output, "- Cases: `{}`", summary.total);
    _ = writeln!(output, "- Safe to apply: `{}`", summary.safe);
    _ = writeln!(output, "- Unsafe: `{}`", summary.unsafe_count);
    _ = writeln!(output, "- Parse failures: `{}`", summary.parse_failures);
    _ = writeln!(output, "- Patch failures: `{}`", summary.patch_failures);
    _ = writeln!(output, "- Sentinel leaks: `{}`", summary.sentinel_leaks);
    _ = writeln!(output, "- Giant deletions: `{}`", summary.giant_deletions);
    _ = writeln!(output);
    _ = writeln!(output, "## Cases");
    _ = writeln!(output);
    _ = writeln!(
        output,
        "| Request | Case | Safe | Parse | Patch | Raw Bytes | Old Bytes | New Bytes | Deletion Ratio | Reasons |"
    );
    _ = writeln!(
        output,
        "| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | --- |"
    );
    for result in results {
        _ = writeln!(
            output,
            "| `{}` | `{}` | `{}` | `{}` | `{}` | `{}` | `{}` | `{}` | `{}` | {} |",
            escape_table_cell(&result.request_name),
            escape_table_cell(&result.case_name),
            yes_no(result.safe_to_apply),
            yes_no(result.parse_ok),
            yes_no(result.patch_ok),
            result.raw_output_bytes,
            result
                .old_editable_bytes
                .map(|bytes| bytes.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            result
                .new_editable_bytes
                .map(|bytes| bytes.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            result
                .deletion_ratio
                .map(|ratio| format!("{ratio:.2}"))
                .unwrap_or_else(|| "n/a".to_string()),
            escape_table_cell(&format_reasons(&result.reasons)),
        );
    }

    output
}

#[derive(Debug)]
struct SafetySummary {
    total: usize,
    safe: usize,
    unsafe_count: usize,
    parse_failures: usize,
    patch_failures: usize,
    sentinel_leaks: usize,
    giant_deletions: usize,
}

fn safety_summary(results: &[OutputSafetyResult]) -> SafetySummary {
    let total = results.len();
    let safe = results.iter().filter(|result| result.safe_to_apply).count();
    let parse_failures = results.iter().filter(|result| !result.parse_ok).count();
    let patch_failures = results.iter().filter(|result| !result.patch_ok).count();
    let sentinel_leaks = results
        .iter()
        .filter(|result| {
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("sentinel leaked"))
        })
        .count();
    let giant_deletions = results
        .iter()
        .filter(|result| {
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("giant deletion"))
        })
        .count();

    SafetySummary {
        total,
        safe,
        unsafe_count: total - safe,
        parse_failures,
        patch_failures,
        sentinel_leaks,
        giant_deletions,
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn format_reasons(reasons: &[String]) -> String {
    if reasons.is_empty() {
        "none".to_string()
    } else {
        reasons.join("; ")
    }
}

fn escape_table_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cloud_llm_client::predict_edits_v3::{PredictEditsV3Request, PredictEditsV3Response};
    use std::sync::Arc;
    use tempfile::tempdir;
    use zeta_prompt::ExcerptRanges;

    #[test]
    fn flags_sentinel_leaks_and_giant_deletions() {
        let input = test_prompt_input();

        let sentinel = assess_output(
            "request-0001",
            "sentinel",
            "<|fim_prefix|>\n<<<<<<< CURRENT\nbad\n>>>>>>> UPDATED\n",
            ZetaFormat::default(),
            &input,
        );
        assert!(!sentinel.safe_to_apply);
        assert!(
            sentinel
                .reasons
                .iter()
                .any(|reason| reason.contains("sentinel leaked"))
        );

        let deletion = assess_output(
            "request-0001",
            "giant-deletion",
            "",
            ZetaFormat::default(),
            &input,
        );
        assert!(!deletion.safe_to_apply);
        assert!(
            deletion
                .reasons
                .iter()
                .any(|reason| reason.contains("giant deletion"))
        );
    }

    #[test]
    fn replays_captured_and_synthetic_safety_cases() {
        let directory = tempdir().unwrap();
        let request_dir = directory.path().join("request-0001");
        std::fs::create_dir_all(&request_dir).unwrap();

        let request = PredictEditsV3Request {
            input: test_prompt_input(),
            trigger: Default::default(),
        };
        let response = PredictEditsV3Response {
            request_id: "stub-request-1".to_string(),
            output: expected_old_editable_region(ZetaFormat::default(), &request.input)
                .unwrap()
                .to_string(),
            editable_range: expected_editable_range(ZetaFormat::default(), &request.input),
            model_version: Some("local-stub".to_string()),
        };

        std::fs::write(
            request_dir.join("request.json"),
            serde_json::to_string_pretty(&request).unwrap(),
        )
        .unwrap();
        std::fs::write(
            request_dir.join("response.json"),
            serde_json::to_string_pretty(&response).unwrap(),
        )
        .unwrap();

        let args = ReplayOutputSafetyArgs {
            directory: directory.path().to_path_buf(),
            format: None,
            include_synthetic: true,
        };
        let output_path = directory.path().join("safety-report.md");
        run_replay_output_safety(&args, Some(&output_path)).unwrap();

        let report = std::fs::read_to_string(output_path).unwrap();
        assert!(report.contains("- Cases: `5`"));
        assert!(report.contains("| `request-0001` | `captured-response` | `yes`"));
        assert!(report.contains("| `request-0001` | `raw-sentinel-leak` | `no`"));
        assert!(report.contains("| `request-0001` | `giant-deletion` | `no`"));
    }

    fn test_prompt_input() -> ZetaPromptInput {
        let editable =
            "fn main() {\n    let message = \"hello\";\n    println!(\"{}\", message);\n}\n";
        let suffix = "\nfn helper() {\n    println!(\"helper\");\n}\n";
        let text = format!(
            "{editable}{}{}{}{}",
            "// A long trailing comment keeps deletion safety meaningful.\n",
            "// This fixture is intentionally larger than the safety threshold.\n",
            "// It models a realistic editable range rather than a toy one.\n",
            suffix
        );
        let editable_end = text.find("\nfn helper").unwrap();

        ZetaPromptInput {
            cursor_path: Arc::from(Path::new("src/main.rs")),
            cursor_excerpt: Arc::from(text.as_str()),
            cursor_offset_in_excerpt: 12,
            excerpt_start_row: Some(0),
            events: Vec::new(),
            related_files: Some(Vec::new()),
            active_buffer_diagnostics: Vec::new(),
            excerpt_ranges: ExcerptRanges {
                editable_150: 0..editable_end,
                editable_180: 0..editable_end,
                editable_350: 0..editable_end,
                editable_512: Some(0..editable_end),
                editable_150_context_350: 0..text.len(),
                editable_180_context_350: 0..text.len(),
                editable_350_context_150: 0..text.len(),
                editable_350_context_512: Some(0..text.len()),
                editable_350_context_1024: Some(0..text.len()),
                context_4096: Some(0..text.len()),
                context_8192: Some(0..text.len()),
            },
            syntax_ranges: None,
            in_open_source_repo: true,
            can_collect_data: false,
            repo_url: None,
        }
    }
}
