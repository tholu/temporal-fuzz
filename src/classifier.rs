use crate::case::{FindingClass, OutcomeKind, RunOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub class: FindingClass,
    pub reason: String,
}

pub fn classify(baseline: &RunOutcome, variant: &RunOutcome) -> Option<Classification> {
    if baseline.kind != variant.kind {
        return Some(match variant.kind {
            OutcomeKind::Timeout => Classification {
                class: FindingClass::Hang,
                reason: "timeout mismatch".to_string(),
            },
            OutcomeKind::Crash | OutcomeKind::InvalidStdout | OutcomeKind::SpawnError => {
                Classification {
                    class: FindingClass::Crash,
                    reason: format!(
                        "outcome mismatch: baseline={:?}, variant={:?}",
                        baseline.kind, variant.kind
                    ),
                }
            }
            OutcomeKind::Ok => match baseline.kind {
                OutcomeKind::Timeout => Classification {
                    class: FindingClass::Hang,
                    reason: "timeout mismatch".to_string(),
                },
                _ => Classification {
                    class: FindingClass::Crash,
                    reason: format!(
                        "outcome mismatch: baseline={:?}, variant={:?}",
                        baseline.kind, variant.kind
                    ),
                },
            },
        });
    }

    if variant.kind == OutcomeKind::Timeout {
        return None;
    }

    if baseline.status() != variant.status() {
        return Some(Classification {
            class: FindingClass::Divergence,
            reason: format!(
                "status mismatch: baseline={:?}, variant={:?}",
                baseline.status(),
                variant.status()
            ),
        });
    }

    if baseline.output_hash() != variant.output_hash() {
        return Some(Classification {
            class: FindingClass::Divergence,
            reason: format!(
                "output_hash mismatch: baseline={:?}, variant={:?}",
                baseline.output_hash(),
                variant.output_hash()
            ),
        });
    }

    if baseline.kind == OutcomeKind::Ok
        && major_runtime_difference(baseline.runtime_ms, variant.runtime_ms)
    {
        return Some(Classification {
            class: FindingClass::Interesting,
            reason: format!(
                "major runtime difference: baseline={}ms, variant={}ms",
                baseline.runtime_ms, variant.runtime_ms
            ),
        });
    }

    None
}

fn major_runtime_difference(a: u128, b: u128) -> bool {
    let faster = a.min(b).max(1);
    let slower = a.max(b);
    slower >= 250 && slower >= faster * 10
}

#[cfg(test)]
mod tests {
    use super::classify;
    use crate::case::{AdapterOutput, FindingClass, OutcomeKind, RunOutcome};
    use serde_json::json;

    fn outcome(kind: OutcomeKind, status: &str, hash: &str) -> RunOutcome {
        RunOutcome {
            kind,
            adapter: Some(AdapterOutput {
                status: status.to_string(),
                output_hash: hash.to_string(),
                observations: json!({}),
            }),
            exit_code: Some(0),
            stderr: String::new(),
            runtime_ms: 1,
            max_rss_kb: None,
            stdout_snippet: String::new(),
            error: None,
        }
    }

    #[test]
    fn classifies_status_and_hash_divergence() {
        let baseline = outcome(OutcomeKind::Ok, "ok", "a");
        let status_variant = outcome(OutcomeKind::Ok, "error", "a");
        let hash_variant = outcome(OutcomeKind::Ok, "ok", "b");

        assert_eq!(
            classify(&baseline, &status_variant).unwrap().class,
            FindingClass::Divergence
        );
        assert_eq!(
            classify(&baseline, &hash_variant).unwrap().class,
            FindingClass::Divergence
        );
    }

    #[test]
    fn classifies_timeout_as_hang() {
        let baseline = outcome(OutcomeKind::Ok, "ok", "a");
        let mut variant = outcome(OutcomeKind::Timeout, "ok", "a");
        variant.adapter = None;

        assert_eq!(
            classify(&baseline, &variant).unwrap().class,
            FindingClass::Hang
        );
    }

    #[test]
    fn classifies_crash_outcome_mismatch() {
        let baseline = outcome(OutcomeKind::Ok, "ok", "a");
        let mut variant = outcome(OutcomeKind::Crash, "ok", "a");
        variant.adapter = None;
        variant.exit_code = Some(7);

        assert_eq!(
            classify(&baseline, &variant).unwrap().class,
            FindingClass::Crash
        );
    }
}
