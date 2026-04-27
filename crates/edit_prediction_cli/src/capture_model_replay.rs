use crate::capture_safety::{OutputSafetyResult, assess_zeta_model_output};
use crate::capture_summary::{capture_request_directories, load_capture_summary};
use crate::stub::{ModelCommandConfig, ModelCommandInput, run_model_command};
use anyhow::{Context as _, Result};
use clap::Args;
use gpui::BackgroundExecutor;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;
use zeta_prompt::ZetaFormat;

#[derive(Debug, Args, Clone)]
pub struct ReplayModelCommandArgs {
    /// Fixture or capture directory containing request-* subdirectories.
    #[arg(long)]
    pub directory: PathBuf,
    /// Zeta prompt format used to parse command stdout.
    #[arg(long)]
    pub format: Option<String>,
    /// Run this command once per request and parse stdout as raw Zeta model output.
    #[arg(long)]
    pub model_command: PathBuf,
    /// Argument to pass to --model-command. May be repeated.
    #[arg(long = "model-command-arg")]
    pub model_command_args: Vec<String>,
    /// Input sent to --model-command stdin.
    #[arg(long, default_value = "prompt")]
    pub model_command_input: ModelCommandInput,
    /// Kill --model-command if it does not finish within this many milliseconds.
    #[arg(long, default_value_t = 30_000)]
    pub model_command_timeout_ms: u64,
}

#[derive(Debug)]
struct ModelReplayResult {
    request_name: String,
    command_ok: bool,
    latency_ms: Option<u128>,
    raw_output_bytes: Option<usize>,
    safety: Option<OutputSafetyResult>,
    error: Option<String>,
}

pub fn run_replay_model_command(
    args: &ReplayModelCommandArgs,
    output_path: Option<&PathBuf>,
    background_executor: BackgroundExecutor,
) -> Result<()> {
    if args.model_command_timeout_ms == 0 {
        anyhow::bail!("--model-command-timeout-ms must be greater than zero");
    }

    let format = args
        .format
        .as_deref()
        .map(ZetaFormat::parse)
        .transpose()?
        .unwrap_or_default();
    let config = ModelCommandConfig {
        command: args.model_command.clone(),
        args: args.model_command_args.clone(),
        input: args.model_command_input,
        timeout_ms: args.model_command_timeout_ms,
    };
    let capture_directories = capture_request_directories(&args.directory)?;
    let mut results = Vec::new();

    for capture_directory in capture_directories {
        let capture = load_capture_summary(&capture_directory)
            .with_context(|| format!("failed to load {}", capture_directory.display()))?;
        let Some(request) = capture.parsed_request.as_ref() else {
            results.push(ModelReplayResult {
                request_name: capture.name,
                command_ok: false,
                latency_ms: None,
                raw_output_bytes: None,
                safety: None,
                error: Some("missing or unparsable request".to_string()),
            });
            continue;
        };

        let started_at = Instant::now();
        match run_model_command(&config, request, &background_executor) {
            Ok(raw_output) => {
                let latency_ms = started_at.elapsed().as_millis();
                let safety = assess_zeta_model_output(
                    &capture.name,
                    "model-command",
                    &raw_output,
                    format,
                    &request.input,
                );
                results.push(ModelReplayResult {
                    request_name: capture.name,
                    command_ok: true,
                    latency_ms: Some(latency_ms),
                    raw_output_bytes: Some(raw_output.len()),
                    safety: Some(safety),
                    error: None,
                });
            }
            Err(error) => {
                results.push(ModelReplayResult {
                    request_name: capture.name,
                    command_ok: false,
                    latency_ms: Some(started_at.elapsed().as_millis()),
                    raw_output_bytes: None,
                    safety: None,
                    error: Some(format!("{error:#}")),
                });
            }
        }
    }

    let output = render_model_replay_report(&args.directory, format, &results);
    if let Some(output_path) = output_path {
        std::fs::write(output_path, output).with_context(|| {
            format!(
                "failed to write model-command replay report to {}",
                output_path.display()
            )
        })?;
    } else {
        print!("{output}");
    }

    Ok(())
}

fn render_model_replay_report(
    directory: &Path,
    format: ZetaFormat,
    results: &[ModelReplayResult],
) -> String {
    let summary = model_replay_summary(results);
    let mut output = String::new();
    _ = writeln!(output, "# Model Command Replay Report");
    _ = writeln!(output);
    _ = writeln!(output, "Directory: `{}`", directory.display());
    _ = writeln!(output, "Format: `{format}`");
    _ = writeln!(output);
    _ = writeln!(output, "## Summary");
    _ = writeln!(output);
    _ = writeln!(output, "- Requests: `{}`", summary.total);
    _ = writeln!(
        output,
        "- Command successes: `{}`",
        summary.command_successes
    );
    _ = writeln!(output, "- Safe outputs: `{}`", summary.safe_outputs);
    _ = writeln!(output, "- Unsafe outputs: `{}`", summary.unsafe_outputs);
    _ = writeln!(output, "- Command failures: `{}`", summary.command_failures);
    _ = writeln!(
        output,
        "- p50 latency ms: `{}`",
        format_optional(summary.p50_ms)
    );
    _ = writeln!(
        output,
        "- p95 latency ms: `{}`",
        format_optional(summary.p95_ms)
    );
    _ = writeln!(
        output,
        "- max latency ms: `{}`",
        format_optional(summary.max_ms)
    );
    _ = writeln!(output);
    _ = writeln!(output, "## Requests");
    _ = writeln!(output);
    _ = writeln!(
        output,
        "| Request | Command | Safe | Latency ms | Raw Bytes | Reasons | Error |"
    );
    _ = writeln!(output, "| --- | --- | --- | ---: | ---: | --- | --- |");
    for result in results {
        let safe = result
            .safety
            .as_ref()
            .map(|safety| yes_no(safety.safe_to_apply))
            .unwrap_or("n/a");
        let reasons = result
            .safety
            .as_ref()
            .map(|safety| format_reasons(&safety.reasons))
            .unwrap_or_else(|| "n/a".to_string());
        _ = writeln!(
            output,
            "| `{}` | `{}` | `{}` | `{}` | `{}` | {} | {} |",
            escape_table_cell(&result.request_name),
            yes_no(result.command_ok),
            safe,
            result
                .latency_ms
                .map(|latency| latency.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            result
                .raw_output_bytes
                .map(|bytes| bytes.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            escape_table_cell(&reasons),
            escape_table_cell(result.error.as_deref().unwrap_or("none")),
        );
    }

    output
}

#[derive(Debug)]
struct ModelReplaySummary {
    total: usize,
    command_successes: usize,
    command_failures: usize,
    safe_outputs: usize,
    unsafe_outputs: usize,
    p50_ms: Option<u128>,
    p95_ms: Option<u128>,
    max_ms: Option<u128>,
}

fn model_replay_summary(results: &[ModelReplayResult]) -> ModelReplaySummary {
    let total = results.len();
    let command_successes = results.iter().filter(|result| result.command_ok).count();
    let command_failures = total - command_successes;
    let safe_outputs = results
        .iter()
        .filter(|result| {
            result
                .safety
                .as_ref()
                .is_some_and(|safety| safety.safe_to_apply)
        })
        .count();
    let unsafe_outputs = results
        .iter()
        .filter(|result| {
            result
                .safety
                .as_ref()
                .is_some_and(|safety| !safety.safe_to_apply)
        })
        .count();
    let mut latencies = results
        .iter()
        .filter_map(|result| result.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();

    ModelReplaySummary {
        total,
        command_successes,
        command_failures,
        safe_outputs,
        unsafe_outputs,
        p50_ms: percentile(&latencies, 50),
        p95_ms: percentile(&latencies, 95),
        max_ms: latencies.last().copied(),
    }
}

fn percentile(sorted_values: &[u128], percentile: usize) -> Option<u128> {
    if sorted_values.is_empty() {
        return None;
    }

    let index = ((sorted_values.len() - 1) * percentile).div_ceil(100);
    sorted_values.get(index).copied()
}

fn format_optional(value: Option<u128>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "n/a".to_string())
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
    use cloud_llm_client::predict_edits_v3::PredictEditsV3Request;
    use std::sync::Arc;
    use tempfile::tempdir;
    use zeta_prompt::ExcerptRanges;

    #[gpui::test]
    async fn replays_model_command_against_capture_fixtures(cx: &mut gpui::TestAppContext) {
        let directory = tempdir().unwrap();
        let request_dir = directory.path().join("request-0001");
        std::fs::create_dir_all(&request_dir).unwrap();
        let request = PredictEditsV3Request {
            input: test_prompt_input(),
            trigger: Default::default(),
        };
        let replacement = format!(
            "{}\n// replay command output\n",
            &request.input.cursor_excerpt[request.input.excerpt_ranges.editable_350.clone()]
        );
        std::fs::write(
            request_dir.join("request.json"),
            serde_json::to_string_pretty(&request).unwrap(),
        )
        .unwrap();

        let script_path = directory.path().join("model.sh");
        let output_path = directory.path().join("output.txt");
        std::fs::write(&script_path, "cat >/dev/null\ncat \"$1\"\n").unwrap();
        std::fs::write(&output_path, &replacement).unwrap();

        let args = ReplayModelCommandArgs {
            directory: directory.path().to_path_buf(),
            format: None,
            model_command: PathBuf::from("/bin/sh"),
            model_command_args: vec![
                script_path.display().to_string(),
                output_path.display().to_string(),
            ],
            model_command_input: ModelCommandInput::Prompt,
            model_command_timeout_ms: 30_000,
        };
        let output_path = directory.path().join("model-replay.md");

        run_replay_model_command(&args, Some(&output_path), cx.executor()).unwrap();

        let report = std::fs::read_to_string(output_path).unwrap();
        assert!(report.contains("- Requests: `1`"));
        assert!(report.contains("- Command successes: `1`"));
        assert!(report.contains("- Safe outputs: `1`"));
        assert!(report.contains("| `request-0001` | `yes` | `yes` |"));
    }

    fn test_prompt_input() -> zeta_prompt::ZetaPromptInput {
        let editable = concat!(
            "fn main() {\n",
            "    let message = \"hello\";\n",
            "    println!(\"{}\", message);\n",
            "}\n",
            "// This editable range is stable enough for replay checks.\n",
        );
        let suffix = "\nfn helper() {\n    println!(\"helper\");\n}\n";
        let text = format!("{editable}{suffix}");
        let editable_end = editable.len();

        zeta_prompt::ZetaPromptInput {
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
