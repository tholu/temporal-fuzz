mod case;
mod classifier;
mod minimize;
mod runner;
mod schedule;

use crate::case::{now_unix_secs, Finding, FindingClass, TemporalCase};
use crate::classifier::classify;
use crate::minimize::minimize_schedule;
use crate::runner::run_adapter;
use crate::schedule::ScheduleGenerator;
use serde_json::json;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

#[derive(Debug)]
enum Command {
    Run {
        target: String,
        input: PathBuf,
        corpus: Option<PathBuf>,
        iterations: usize,
        seed: u64,
        timeout_ms: u64,
        progress_every: usize,
    },
    Replay {
        target: String,
        case_path: PathBuf,
        timeout_ms: u64,
    },
    Generate {
        input: PathBuf,
        out: PathBuf,
        count: usize,
        seed: u64,
    },
    Minimize {
        target: String,
        case_path: PathBuf,
        out: PathBuf,
        timeout_ms: u64,
    },
}

#[derive(Debug, Default)]
struct Summary {
    total: usize,
    crashes: usize,
    hangs: usize,
    divergences: usize,
    interesting: usize,
}

fn main() {
    if let Err(err) = real_main() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn real_main() -> Result<(), String> {
    let args = env::args().collect::<Vec<_>>();
    let command = parse_args(&args)?;
    match command {
        Command::Run {
            target,
            input,
            corpus,
            iterations,
            seed,
            timeout_ms,
            progress_every,
        } => run_command(
            &args,
            &target,
            &input,
            corpus.as_deref(),
            iterations,
            seed,
            timeout_ms,
            progress_every,
        ),
        Command::Replay {
            target,
            case_path,
            timeout_ms,
        } => replay_command(&target, &case_path, timeout_ms),
        Command::Generate {
            input,
            out,
            count,
            seed,
        } => generate_command(&input, &out, count, seed),
        Command::Minimize {
            target,
            case_path,
            out,
            timeout_ms,
        } => minimize_command(&args, &target, &case_path, &out, timeout_ms),
    }
}

fn run_command(
    command_line: &[String],
    target: &str,
    input: &Path,
    corpus: Option<&Path>,
    iterations: usize,
    seed: u64,
    timeout_ms: u64,
    progress_every: usize,
) -> Result<(), String> {
    ensure_output_dirs()?;
    let inputs = collect_inputs(input, corpus)?;
    let mut summary = Summary::default();

    for (input_path, payload) in inputs {
        let input_name = Some(input_path.display().to_string());
        let baseline_ops = ScheduleGenerator::baseline(payload.len());
        let baseline = run_adapter(target, &payload, &baseline_ops, timeout_ms);
        let mut generator = ScheduleGenerator::new(seed);

        for iteration in 0..iterations {
            let case = generator.next_case(input_name.clone(), &payload, iteration);
            let variant = run_adapter(target, &payload, &case.ops, timeout_ms);
            summary.total += 1;

            if let Some(classification) = classify(&baseline, &variant) {
                increment_summary(&mut summary, classification.class);
                let finding = Finding {
                    finding_class: classification.class,
                    input_filename: input_name.clone(),
                    payload_hash: case.payload_hash.clone(),
                    payload_b64: case.payload_b64.clone(),
                    schedule: case.ops.clone(),
                    baseline_result: baseline.clone(),
                    variant_result: variant.clone(),
                    stderr_snippets: vec![baseline.stderr_snippet(), variant.stderr_snippet()],
                    command_line: command_line.to_vec(),
                    timestamp: now_unix_secs(),
                };
                save_finding(&finding, iteration)?;
            }

            if progress_every > 0 && summary.total % progress_every == 0 {
                eprintln!(
                    "progress total={} crashes={} hangs={} divergences={} interesting={}",
                    summary.total,
                    summary.crashes,
                    summary.hangs,
                    summary.divergences,
                    summary.interesting
                );
            }
        }
    }

    println!(
        "summary total={} crashes={} hangs={} divergences={} interesting={}",
        summary.total, summary.crashes, summary.hangs, summary.divergences, summary.interesting
    );
    Ok(())
}

fn replay_command(target: &str, case_path: &Path, timeout_ms: u64) -> Result<(), String> {
    let loaded = load_case_or_finding(case_path)?;
    let payload = loaded.payload()?;
    let baseline_ops = ScheduleGenerator::baseline(payload.len());
    let baseline = run_adapter(target, &payload, &baseline_ops, timeout_ms);
    let variant = run_adapter(target, &payload, loaded.ops(), timeout_ms);
    let classification = classify(&baseline, &variant);

    let replay = json!({
        "case": case_path.display().to_string(),
        "baseline_result": baseline,
        "variant_result": variant,
        "classification": classification.map(|item| json!({
            "class": item.class,
            "reason": item.reason,
        })),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&replay).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn generate_command(input: &Path, out: &Path, count: usize, seed: u64) -> Result<(), String> {
    let payload =
        fs::read(input).map_err(|err| format!("failed to read {}: {err}", input.display()))?;
    fs::create_dir_all(out).map_err(|err| format!("failed to create {}: {err}", out.display()))?;
    let mut generator = ScheduleGenerator::new(seed);

    for idx in 0..count {
        let case = generator.next_case(Some(input.display().to_string()), &payload, idx);
        let path = out.join(format!("case-{idx:06}.json"));
        write_json(&path, &case)?;
    }
    println!("generated {count} cases in {}", out.display());
    Ok(())
}

fn minimize_command(
    command_line: &[String],
    target: &str,
    case_path: &Path,
    out: &Path,
    timeout_ms: u64,
) -> Result<(), String> {
    let loaded = load_case_or_finding(case_path)?;
    let payload = loaded.payload()?;
    let baseline_ops = ScheduleGenerator::baseline(payload.len());
    let baseline = run_adapter(target, &payload, &baseline_ops, timeout_ms);
    let original = run_adapter(target, &payload, loaded.ops(), timeout_ms);
    let Some(classification) = classify(&baseline, &original) else {
        return Err("case does not currently reproduce a finding".to_string());
    };

    let (minimized_ops, minimized_outcome) = minimize_schedule(
        target,
        &payload,
        loaded.ops(),
        &baseline,
        classification.class,
        timeout_ms,
    );
    let finding = Finding {
        finding_class: classification.class,
        input_filename: loaded.input_filename().map(ToString::to_string),
        payload_hash: loaded.payload_hash().to_string(),
        payload_b64: loaded.payload_b64().to_string(),
        schedule: minimized_ops,
        baseline_result: baseline,
        variant_result: minimized_outcome,
        stderr_snippets: Vec::new(),
        command_line: command_line.to_vec(),
        timestamp: now_unix_secs(),
    };

    write_json(out, &finding)?;
    println!(
        "minimized {} -> {} ops, class={:?}, wrote {}",
        loaded.ops().len(),
        finding.schedule.len(),
        finding.finding_class,
        out.display()
    );
    Ok(())
}

enum LoadedCase {
    Case(TemporalCase),
    Finding(Finding),
}

impl LoadedCase {
    fn payload(&self) -> Result<Vec<u8>, String> {
        match self {
            LoadedCase::Case(case) => case.payload(),
            LoadedCase::Finding(finding) => finding.payload(),
        }
    }

    fn ops(&self) -> &[crate::case::Op] {
        match self {
            LoadedCase::Case(case) => &case.ops,
            LoadedCase::Finding(finding) => &finding.schedule,
        }
    }

    fn payload_hash(&self) -> &str {
        match self {
            LoadedCase::Case(case) => &case.payload_hash,
            LoadedCase::Finding(finding) => &finding.payload_hash,
        }
    }

    fn payload_b64(&self) -> &str {
        match self {
            LoadedCase::Case(case) => &case.payload_b64,
            LoadedCase::Finding(finding) => &finding.payload_b64,
        }
    }

    fn input_filename(&self) -> Option<&str> {
        match self {
            LoadedCase::Case(case) => case.input_filename.as_deref(),
            LoadedCase::Finding(finding) => finding.input_filename.as_deref(),
        }
    }
}

fn load_case_or_finding(path: &Path) -> Result<LoadedCase, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("invalid JSON: {err}"))?;
    if value.get("schedule").is_some() {
        serde_json::from_value(value)
            .map(LoadedCase::Finding)
            .map_err(|err| format!("invalid finding JSON: {err}"))
    } else {
        serde_json::from_value(value)
            .map(LoadedCase::Case)
            .map_err(|err| format!("invalid case JSON: {err}"))
    }
}

fn collect_inputs(input: &Path, corpus: Option<&Path>) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut inputs = vec![(input.to_path_buf(), read_payload(input)?)];
    if let Some(corpus) = corpus {
        for entry in fs::read_dir(corpus)
            .map_err(|err| format!("failed to read corpus {}: {err}", corpus.display()))?
        {
            let entry = entry.map_err(|err| format!("failed to read corpus entry: {err}"))?;
            let path = entry.path();
            if path.is_file() {
                inputs.push((path.clone(), read_payload(&path)?));
            }
        }
    }
    Ok(inputs)
}

fn read_payload(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("failed to read {}: {err}", path.display()))
}

fn ensure_output_dirs() -> Result<(), String> {
    for dir in ["crashes", "hangs", "divergences", "interesting"] {
        fs::create_dir_all(dir).map_err(|err| format!("failed to create {dir}: {err}"))?;
    }
    Ok(())
}

fn save_finding(finding: &Finding, iteration: usize) -> Result<(), String> {
    let dir = finding.finding_class.dir_name();
    let path = Path::new(dir).join(format!("id-{}-{iteration:06}.json", finding.timestamp));
    write_json(&path, finding)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn increment_summary(summary: &mut Summary, class: FindingClass) {
    match class {
        FindingClass::Crash => summary.crashes += 1,
        FindingClass::Hang => summary.hangs += 1,
        FindingClass::Divergence => summary.divergences += 1,
        FindingClass::Interesting => summary.interesting += 1,
    }
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some(subcommand) = args.get(1).map(String::as_str) else {
        return Err(usage());
    };

    match subcommand {
        "run" => Ok(Command::Run {
            target: required(args, "--target")?,
            input: PathBuf::from(required(args, "--input")?),
            corpus: optional(args, "--corpus").map(PathBuf::from),
            iterations: parse_optional(args, "--iterations", 1000)?,
            seed: parse_optional(args, "--seed", now_unix_secs())?,
            timeout_ms: parse_optional(args, "--timeout-ms", 1000)?,
            progress_every: parse_optional(args, "--progress-every", 100)?,
        }),
        "replay" => Ok(Command::Replay {
            target: required(args, "--target")?,
            case_path: PathBuf::from(required(args, "--case")?),
            timeout_ms: parse_optional(args, "--timeout-ms", 1000)?,
        }),
        "generate" => Ok(Command::Generate {
            input: PathBuf::from(required(args, "--input")?),
            out: PathBuf::from(required(args, "--out")?),
            count: parse_optional(args, "--count", 100)?,
            seed: parse_optional(args, "--seed", now_unix_secs())?,
        }),
        "minimize" => Ok(Command::Minimize {
            target: required(args, "--target")?,
            case_path: PathBuf::from(required(args, "--case")?),
            out: PathBuf::from(required(args, "--out")?),
            timeout_ms: parse_optional(args, "--timeout-ms", 1000)?,
        }),
        _ => Err(usage()),
    }
}

fn required(args: &[String], flag: &str) -> Result<String, String> {
    optional(args, flag).ok_or_else(|| format!("missing required flag {flag}\n\n{}", usage()))
}

fn optional(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn parse_optional<T>(args: &[String], flag: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match optional(args, flag) {
        Some(value) => value
            .parse::<T>()
            .map_err(|err| format!("invalid value for {flag}: {err}")),
        None => Ok(default),
    }
}

fn usage() -> String {
    "usage:
  temporal-fuzz run --target ./adapter --input sample.bin --iterations 10000 [--seed N] [--timeout-ms N] [--corpus DIR]
  temporal-fuzz replay --target ./adapter --case crashes/id.json [--timeout-ms N]
  temporal-fuzz generate --input sample.bin --out cases/ --count 1000 [--seed N]
  temporal-fuzz minimize --target ./adapter --case crashes/id.json --out minimized.json [--timeout-ms N]"
        .to_string()
}
