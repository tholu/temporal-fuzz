use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_temporal-fuzz")
}

fn repo_path(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(path)
        .display()
        .to_string()
}

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("temporal-fuzz-{name}-{nonce}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn replay_reproduces_generated_case() {
    let dir = temp_dir("replay");
    let input = dir.join("sample.bin");
    let cases = dir.join("cases");
    fs::write(&input, b"0010abcdefghij").unwrap();

    let generate = Command::new(bin())
        .args([
            "generate",
            "--input",
            input.to_str().unwrap(),
            "--out",
            cases.to_str().unwrap(),
            "--count",
            "2",
            "--seed",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        generate.status.success(),
        "{}",
        String::from_utf8_lossy(&generate.stderr)
    );
    let generated_case: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(cases.join("case-000001.json")).unwrap()).unwrap();
    assert_eq!(
        generated_case["schedule_class"].as_str(),
        Some("equivalent_boundary")
    );
    assert_eq!(generated_case["baseline_equivalent"].as_bool(), Some(true));

    let replay = Command::new(bin())
        .args([
            "replay",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/echo_adapter.py"),
            "--case",
            cases.join("case-000001.json").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let stdout = String::from_utf8_lossy(&replay.stdout);
    assert!(stdout.contains("\"classification\": null"), "{stdout}");
}

#[test]
fn run_finds_divergence_and_saved_case_is_replayable() {
    let dir = temp_dir("divergence");
    let input = dir.join("sample.bin");
    fs::write(&input, b"0010abcdefghij").unwrap();

    let run = Command::new(bin())
        .current_dir(&dir)
        .args([
            "run",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--input",
            input.to_str().unwrap(),
            "--iterations",
            "10",
            "--seed",
            "1",
            "--timeout-ms",
            "200",
            "--mode",
            "chaos",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let divergence = fs::read_dir(dir.join("divergences"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .next()
        .expect("expected divergence finding");
    let finding: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&divergence).unwrap()).unwrap();
    assert!(finding["schedule_class"].as_str().is_some());
    assert!(finding["baseline_equivalent"].as_bool().is_some());

    let replay = Command::new(bin())
        .args([
            "replay",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--case",
            divergence.to_str().unwrap(),
            "--timeout-ms",
            "200",
        ])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("\"class\": \"divergence\""));
}

#[test]
fn minimize_reduces_known_case() {
    let dir = temp_dir("minimize");
    let input = dir.join("sample.bin");
    fs::write(&input, b"0010abcdefghij").unwrap();

    let run = Command::new(bin())
        .current_dir(&dir)
        .args([
            "run",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--input",
            input.to_str().unwrap(),
            "--iterations",
            "10",
            "--seed",
            "1",
            "--timeout-ms",
            "200",
            "--mode",
            "chaos",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let divergence = fs::read_dir(dir.join("divergences"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .next()
        .expect("expected divergence finding");
    let before: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&divergence).unwrap()).unwrap();
    let before_len = before["schedule"].as_array().unwrap().len();
    let out = dir.join("minimized.json");

    let minimize = Command::new(bin())
        .args([
            "minimize",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--case",
            divergence.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--timeout-ms",
            "200",
            "--mode",
            "chaos",
        ])
        .output()
        .unwrap();
    assert!(
        minimize.status.success(),
        "{}",
        String::from_utf8_lossy(&minimize.stderr)
    );

    let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(out).unwrap()).unwrap();
    let after_len = after["schedule"].as_array().unwrap().len();
    assert!(
        after_len <= before_len,
        "expected minimized schedule: {before_len} -> {after_len}"
    );
}

#[test]
fn compact_saved_finding_remains_replayable() {
    let dir = temp_dir("compact");
    let input = dir.join("sample.bin");
    fs::write(&input, b"0010abcdefghij").unwrap();

    let run = Command::new(bin())
        .current_dir(&dir)
        .args([
            "run",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--input",
            input.to_str().unwrap(),
            "--iterations",
            "10",
            "--seed",
            "1",
            "--timeout-ms",
            "200",
            "--embed-payload",
            "false",
            "--mode",
            "chaos",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let divergence = fs::read_dir(dir.join("divergences"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .next()
        .expect("expected divergence finding");
    let finding: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&divergence).unwrap()).unwrap();
    assert!(finding["payload_b64"].is_null());
    assert!(finding["payload_path"]
        .as_str()
        .unwrap()
        .starts_with("payloads/"));

    let replay = Command::new(bin())
        .args([
            "replay",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/buggy_adapter.py"),
            "--case",
            divergence.to_str().unwrap(),
            "--timeout-ms",
            "200",
        ])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("\"class\": \"divergence\""));
}

#[test]
fn boundary_mode_echo_adapter_produces_zero_findings() {
    let dir = temp_dir("boundary-echo");
    let input = dir.join("sample.bin");
    let out_dir = dir.join("findings");
    fs::write(&input, b"0010abcdefghij").unwrap();

    let run = Command::new(bin())
        .args([
            "run",
            "--mode",
            "boundary",
            "--target",
            "python3",
            "--target-arg",
            &repo_path("examples/echo_adapter.py"),
            "--input",
            input.to_str().unwrap(),
            "--iterations",
            "100",
            "--seed",
            "1",
            "--out-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("divergences=0"), "{stdout}");
    for subdir in ["crashes", "hangs", "divergences", "interesting"] {
        let count = fs::read_dir(out_dir.join(subdir)).unwrap().count();
        assert_eq!(count, 0, "expected no findings in {subdir}");
    }

    let run_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["mode"].as_str(), Some("boundary"));
    assert_eq!(run_json["target_argv"][0].as_str(), Some("python3"));
    assert_eq!(run_json["summary"]["divergences"].as_u64(), Some(0));
}

#[test]
fn help_works_for_root_and_subcommands() {
    for args in [
        vec!["--help"],
        vec!["run", "--help"],
        vec!["replay", "--help"],
        vec!["generate", "--help"],
        vec!["minimize", "--help"],
    ] {
        let output = Command::new(bin()).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("usage:"));
    }
}
