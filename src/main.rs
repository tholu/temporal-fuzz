mod case;
mod classifier;
mod minimize;
mod runner;
mod schedule;

use crate::case::{now_unix_secs, Finding, FindingClass, OutcomeKind, TemporalCase};
use crate::classifier::classify;
use crate::minimize::{minimize_schedule, FailurePredicate};
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
        target: Vec<String>,
        input: PathBuf,
        corpus: Option<PathBuf>,
        iterations: usize,
        seed: u64,
        timeout_ms: u64,
        progress_every: usize,
        embed_payload: bool,
    },
    Replay {
        target: Vec<String>,
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
        target: Vec<String>,
        case_path: PathBuf,
        out: PathBuf,
        timeout_ms: u64,
        embed_payload: bool,
    },
}

#[derive(Debug, Default)]
struct Summary {
    total: usize,
    crashes: usize,
    hangs: usize,
    divergences: usize,
    interesting: usize,
    baseline_failures: usize,
}

#[derive(Debug)]
struct RunOptions<'a> {
    command_line: &'a [String],
    target: &'a [String],
    input: &'a Path,
    corpus: Option<&'a Path>,
    iterations: usize,
    seed: u64,
    timeout_ms: u64,
    progress_every: usize,
    embed_payload: bool,
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
            embed_payload,
        } => run_command(RunOptions {
            command_line: &args,
            target: &target,
            input: &input,
            corpus: corpus.as_deref(),
            iterations,
            seed,
            timeout_ms,
            progress_every,
            embed_payload,
        }),
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
            embed_payload,
        } => minimize_command(&args, &target, &case_path, &out, timeout_ms, embed_payload),
    }
}

fn run_command(options: RunOptions<'_>) -> Result<(), String> {
    ensure_output_dirs()?;
    let inputs = collect_input_paths(options.input, options.corpus)?;
    let mut summary = Summary::default();

    for input_path in inputs {
        let payload = read_payload(&input_path)?;
        let input_name = Some(input_path.display().to_string());
        let baseline_ops = ScheduleGenerator::baseline(payload.len());
        let baseline = run_adapter(options.target, &payload, &baseline_ops, options.timeout_ms);
        if baseline.kind != OutcomeKind::Ok {
            summary.baseline_failures += 1;
            eprintln!(
                "baseline failure input={} kind={:?} error={:?}",
                input_path.display(),
                baseline.kind,
                baseline.error
            );
            continue;
        }
        let mut generator = ScheduleGenerator::new(options.seed);

        for iteration in 0..options.iterations {
            let case = generator.next_case(input_name.clone(), &payload, iteration);
            let variant = run_adapter(options.target, &payload, &case.ops, options.timeout_ms);
            summary.total += 1;

            if let Some(classification) = classify(&baseline, &variant) {
                increment_summary(&mut summary, classification.class);
                let finding = Finding {
                    finding_class: classification.class,
                    input_filename: input_name.clone(),
                    payload_hash: case.payload_hash.clone(),
                    payload_b64: Some(case.payload_b64.clone()),
                    payload_path: None,
                    schedule: case.ops.clone(),
                    baseline_result: baseline.clone(),
                    variant_result: variant.clone(),
                    stderr_snippets: vec![baseline.stderr_snippet(), variant.stderr_snippet()],
                    command_line: options.command_line.to_vec(),
                    timestamp: now_unix_secs(),
                };
                save_finding(&finding, iteration, options.embed_payload, &payload)?;
            }

            if options.progress_every > 0 && summary.total % options.progress_every == 0 {
                eprintln!(
                    "progress total={} crashes={} hangs={} divergences={} interesting={} baseline_failures={}",
                    summary.total,
                    summary.crashes,
                    summary.hangs,
                    summary.divergences,
                    summary.interesting,
                    summary.baseline_failures
                );
            }
        }
    }

    println!(
        "summary total={} crashes={} hangs={} divergences={} interesting={} baseline_failures={}",
        summary.total,
        summary.crashes,
        summary.hangs,
        summary.divergences,
        summary.interesting,
        summary.baseline_failures
    );
    Ok(())
}

fn replay_command(target: &[String], case_path: &Path, timeout_ms: u64) -> Result<(), String> {
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
    target: &[String],
    case_path: &Path,
    out: &Path,
    timeout_ms: u64,
    embed_payload: bool,
) -> Result<(), String> {
    let loaded = load_case_or_finding(case_path)?;
    let payload = loaded.payload()?;
    let baseline_ops = ScheduleGenerator::baseline(payload.len());
    let baseline = run_adapter(target, &payload, &baseline_ops, timeout_ms);
    let original = run_adapter(target, &payload, loaded.ops(), timeout_ms);
    let Some(predicate) = FailurePredicate::new(&baseline, &original) else {
        return Err("case does not currently reproduce a finding".to_string());
    };

    let (minimized_ops, minimized_outcome) = minimize_schedule(
        target,
        &payload,
        loaded.ops(),
        &baseline,
        &predicate,
        timeout_ms,
    );
    let finding = Finding {
        finding_class: predicate.class(),
        input_filename: loaded.input_filename().map(ToString::to_string),
        payload_hash: loaded.payload_hash().to_string(),
        payload_b64: Some(loaded.payload_b64()?),
        payload_path: None,
        schedule: minimized_ops,
        baseline_result: baseline,
        variant_result: minimized_outcome,
        stderr_snippets: Vec::new(),
        command_line: command_line.to_vec(),
        timestamp: now_unix_secs(),
    };

    let finding = prepare_finding_payload(
        finding,
        out.parent().unwrap_or_else(|| Path::new(".")),
        embed_payload,
        &payload,
    )?;
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
    Finding(Box<Finding>, PathBuf),
}

impl LoadedCase {
    fn payload(&self) -> Result<Vec<u8>, String> {
        match self {
            LoadedCase::Case(case) => case.payload(),
            LoadedCase::Finding(finding, base_dir) => finding.payload_from_dir(base_dir),
        }
    }

    fn ops(&self) -> &[crate::case::Op] {
        match self {
            LoadedCase::Case(case) => &case.ops,
            LoadedCase::Finding(finding, _) => &finding.schedule,
        }
    }

    fn payload_hash(&self) -> &str {
        match self {
            LoadedCase::Case(case) => &case.payload_hash,
            LoadedCase::Finding(finding, _) => &finding.payload_hash,
        }
    }

    fn payload_b64(&self) -> Result<String, String> {
        match self {
            LoadedCase::Case(case) => Ok(case.payload_b64.clone()),
            LoadedCase::Finding(finding, base_dir) => {
                if let Some(payload_b64) = &finding.payload_b64 {
                    Ok(payload_b64.clone())
                } else {
                    let payload = finding.payload_from_dir(base_dir)?;
                    Ok(crate::case::base64_encode(&payload))
                }
            }
        }
    }

    fn input_filename(&self) -> Option<&str> {
        match self {
            LoadedCase::Case(case) => case.input_filename.as_deref(),
            LoadedCase::Finding(finding, _) => finding.input_filename.as_deref(),
        }
    }
}

fn load_case_or_finding(path: &Path) -> Result<LoadedCase, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("invalid JSON: {err}"))?;
    let base_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if value.get("schedule").is_some() {
        serde_json::from_value(value)
            .map(|finding| LoadedCase::Finding(Box::new(finding), base_dir))
            .map_err(|err| format!("invalid finding JSON: {err}"))
    } else {
        serde_json::from_value(value)
            .map(LoadedCase::Case)
            .map_err(|err| format!("invalid case JSON: {err}"))
    }
}

fn collect_input_paths(input: &Path, corpus: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let mut inputs = vec![input.to_path_buf()];
    if let Some(corpus) = corpus {
        for entry in fs::read_dir(corpus)
            .map_err(|err| format!("failed to read corpus {}: {err}", corpus.display()))?
        {
            let entry = entry.map_err(|err| format!("failed to read corpus entry: {err}"))?;
            let path = entry.path();
            if path.is_file() {
                inputs.push(path);
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

fn save_finding(
    finding: &Finding,
    iteration: usize,
    embed_payload: bool,
    payload: &[u8],
) -> Result<(), String> {
    let dir = finding.finding_class.dir_name();
    let path = Path::new(dir).join(format!("id-{}-{iteration:06}.json", finding.timestamp));
    let finding = prepare_finding_payload(finding.clone(), Path::new(dir), embed_payload, payload)?;
    write_json(&path, &finding)
}

fn prepare_finding_payload(
    mut finding: Finding,
    base_dir: &Path,
    embed_payload: bool,
    payload: &[u8],
) -> Result<Finding, String> {
    if embed_payload {
        finding.payload_b64 = Some(crate::case::base64_encode(payload));
        finding.payload_path = None;
        return Ok(finding);
    }

    let payload_dir = base_dir.join("payloads");
    fs::create_dir_all(&payload_dir)
        .map_err(|err| format!("failed to create {}: {err}", payload_dir.display()))?;
    let filename = format!("{}.bin", safe_hash_filename(&finding.payload_hash));
    let payload_path = payload_dir.join(&filename);
    if !payload_path.exists() {
        fs::write(&payload_path, payload)
            .map_err(|err| format!("failed to write {}: {err}", payload_path.display()))?;
    }
    finding.payload_b64 = None;
    finding.payload_path = Some(format!("payloads/{filename}"));
    Ok(finding)
}

fn safe_hash_filename(hash: &str) -> String {
    hash.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
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
        "run" => {
            let flags = parse_flags(
                &args[2..],
                &[
                    "--target",
                    "--target-arg",
                    "--input",
                    "--corpus",
                    "--iterations",
                    "--seed",
                    "--timeout-ms",
                    "--progress-every",
                    "--embed-payload",
                ],
                &["--target-arg"],
            )?;
            Ok(Command::Run {
                target: target_argv(&flags)?,
                input: PathBuf::from(flags.required("--input")?),
                corpus: flags.optional("--corpus").map(PathBuf::from),
                iterations: flags.parse_optional("--iterations", 1000)?,
                seed: flags.parse_optional("--seed", now_unix_secs())?,
                timeout_ms: flags.parse_optional("--timeout-ms", 1000)?,
                progress_every: flags.parse_optional("--progress-every", 100)?,
                embed_payload: flags.parse_bool_optional("--embed-payload", true)?,
            })
        }
        "replay" => {
            let flags = parse_flags(
                &args[2..],
                &["--target", "--target-arg", "--case", "--timeout-ms"],
                &["--target-arg"],
            )?;
            Ok(Command::Replay {
                target: target_argv(&flags)?,
                case_path: PathBuf::from(flags.required("--case")?),
                timeout_ms: flags.parse_optional("--timeout-ms", 1000)?,
            })
        }
        "generate" => {
            let flags = parse_flags(&args[2..], &["--input", "--out", "--count", "--seed"], &[])?;
            Ok(Command::Generate {
                input: PathBuf::from(flags.required("--input")?),
                out: PathBuf::from(flags.required("--out")?),
                count: flags.parse_optional("--count", 100)?,
                seed: flags.parse_optional("--seed", now_unix_secs())?,
            })
        }
        "minimize" => {
            let flags = parse_flags(
                &args[2..],
                &[
                    "--target",
                    "--target-arg",
                    "--case",
                    "--out",
                    "--timeout-ms",
                    "--embed-payload",
                ],
                &["--target-arg"],
            )?;
            Ok(Command::Minimize {
                target: target_argv(&flags)?,
                case_path: PathBuf::from(flags.required("--case")?),
                out: PathBuf::from(flags.required("--out")?),
                timeout_ms: flags.parse_optional("--timeout-ms", 1000)?,
                embed_payload: flags.parse_bool_optional("--embed-payload", true)?,
            })
        }
        _ => Err(usage()),
    }
}

#[derive(Debug)]
struct Flags {
    values: Vec<(String, String)>,
}

impl Flags {
    fn required(&self, flag: &str) -> Result<String, String> {
        self.optional(flag)
            .ok_or_else(|| format!("missing required flag {flag}\n\n{}", usage()))
    }

    fn optional(&self, flag: &str) -> Option<String> {
        self.values
            .iter()
            .find(|(candidate, _)| candidate == flag)
            .map(|(_, value)| value.clone())
    }

    fn all(&self, flag: &str) -> Vec<String> {
        self.values
            .iter()
            .filter(|(candidate, _)| candidate == flag)
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn parse_optional<T>(&self, flag: &str, default: T) -> Result<T, String>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        match self.optional(flag) {
            Some(value) => value
                .parse::<T>()
                .map_err(|err| format!("invalid value for {flag}: {err}")),
            None => Ok(default),
        }
    }

    fn parse_bool_optional(&self, flag: &str, default: bool) -> Result<bool, String> {
        match self.optional(flag).as_deref() {
            Some("true") | Some("1") | Some("yes") => Ok(true),
            Some("false") | Some("0") | Some("no") => Ok(false),
            Some(value) => Err(format!(
                "invalid value for {flag}: {value}; expected true or false"
            )),
            None => Ok(default),
        }
    }
}

fn parse_flags(args: &[String], allowed: &[&str], repeatable: &[&str]) -> Result<Flags, String> {
    let mut values = Vec::new();
    let mut idx = 0;
    while idx < args.len() {
        let flag = args[idx].as_str();
        if !flag.starts_with("--") {
            return Err(format!(
                "unexpected positional argument: {flag}\n\n{}",
                usage()
            ));
        }
        if !allowed.contains(&flag) {
            return Err(format!("unknown flag {flag}\n\n{}", usage()));
        }
        let Some(value) = args.get(idx + 1) else {
            return Err(format!("missing value for {flag}\n\n{}", usage()));
        };
        if value.starts_with("--") {
            return Err(format!("missing value for {flag}\n\n{}", usage()));
        }
        if !repeatable.contains(&flag) && values.iter().any(|(existing, _)| existing == flag) {
            return Err(format!("duplicate flag {flag}\n\n{}", usage()));
        }
        values.push((flag.to_string(), value.clone()));
        idx += 2;
    }
    Ok(Flags { values })
}

fn target_argv(flags: &Flags) -> Result<Vec<String>, String> {
    let target = flags.required("--target")?;
    if target.trim().is_empty() {
        return Err("target command cannot be empty".to_string());
    }
    if target.split_whitespace().count() > 1 && !Path::new(&target).exists() {
        return Err(
            "target must be one executable path; pass adapter arguments with repeated --target-arg"
                .to_string(),
        );
    }
    let mut argv = vec![target];
    argv.extend(flags.all("--target-arg"));
    Ok(argv)
}

fn usage() -> String {
    "usage:
  temporal-fuzz run --target ./adapter [--target-arg ARG ...] --input sample.bin --iterations 10000 [--seed N] [--timeout-ms N] [--corpus DIR] [--embed-payload true|false]
  temporal-fuzz replay --target ./adapter [--target-arg ARG ...] --case crashes/id.json [--timeout-ms N]
  temporal-fuzz generate --input sample.bin --out cases/ --count 1000 [--seed N]
  temporal-fuzz minimize --target ./adapter [--target-arg ARG ...] --case crashes/id.json --out minimized.json [--timeout-ms N] [--embed-payload true|false]"
        .to_string()
}
