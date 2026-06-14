use crate::case::{FindingClass, Op, RunOutcome};
use crate::classifier::classify;
use crate::runner::run_adapter;

pub fn minimize_schedule(
    target: &str,
    payload: &[u8],
    initial_ops: &[Op],
    baseline: &RunOutcome,
    finding_class: FindingClass,
    timeout_ms: u64,
) -> (Vec<Op>, RunOutcome) {
    let mut ops = initial_ops.to_vec();
    let mut outcome = run_adapter(target, payload, &ops, timeout_ms);
    if !preserves(baseline, &outcome, finding_class) {
        return (ops, outcome);
    }

    let mut changed = true;
    while changed {
        changed = false;
        for idx in 0..ops.len() {
            let mut candidate = ops.clone();
            candidate.remove(idx);
            if candidate.is_empty() {
                continue;
            }
            let candidate_outcome = run_adapter(target, payload, &candidate, timeout_ms);
            if preserves(baseline, &candidate_outcome, finding_class) {
                ops = candidate;
                outcome = candidate_outcome;
                changed = true;
                break;
            }
        }
    }

    let merged = merge_adjacent_feeds(&ops);
    let merged_outcome = run_adapter(target, payload, &merged, timeout_ms);
    if preserves(baseline, &merged_outcome, finding_class) {
        ops = merged;
        outcome = merged_outcome;
    }

    let simplified = simplify_values(&ops);
    let simplified_outcome = run_adapter(target, payload, &simplified, timeout_ms);
    if preserves(baseline, &simplified_outcome, finding_class) {
        ops = simplified;
        outcome = simplified_outcome;
    }

    shrink_feed_lengths(
        target,
        payload,
        baseline,
        finding_class,
        timeout_ms,
        &mut ops,
        &mut outcome,
    );
    (ops, outcome)
}

fn preserves(baseline: &RunOutcome, variant: &RunOutcome, finding_class: FindingClass) -> bool {
    classify(baseline, variant)
        .map(|classification| classification.class == finding_class)
        .unwrap_or(false)
}

fn merge_adjacent_feeds(ops: &[Op]) -> Vec<Op> {
    let mut merged = Vec::new();
    for op in ops {
        match (merged.last_mut(), op) {
            (
                Some(Op::Feed { offset, len }),
                Op::Feed {
                    offset: next_offset,
                    len: next_len,
                },
            ) if *offset + *len == *next_offset => {
                *len += *next_len;
            }
            _ => merged.push(op.clone()),
        }
    }
    merged
}

fn simplify_values(ops: &[Op]) -> Vec<Op> {
    ops.iter()
        .map(|op| match op {
            Op::Seek { .. } => Op::Seek { offset: 0 },
            Op::Reconfigure { .. } => Op::Reconfigure { mode: 0, limit: 0 },
            other => other.clone(),
        })
        .collect()
}

fn shrink_feed_lengths(
    target: &str,
    payload: &[u8],
    baseline: &RunOutcome,
    finding_class: FindingClass,
    timeout_ms: u64,
    ops: &mut Vec<Op>,
    outcome: &mut RunOutcome,
) {
    for idx in 0..ops.len() {
        let Op::Feed { len, .. } = ops[idx] else {
            continue;
        };
        let mut candidate_len = len;
        while candidate_len > 1 {
            candidate_len = candidate_len.div_ceil(2);
            let mut candidate = ops.clone();
            if let Op::Feed { len, .. } = &mut candidate[idx] {
                *len = candidate_len;
            }
            let candidate_outcome = run_adapter(target, payload, &candidate, timeout_ms);
            if preserves(baseline, &candidate_outcome, finding_class) {
                *ops = candidate;
                *outcome = candidate_outcome;
            } else if candidate_len == 1 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_adjacent_feeds, simplify_values};
    use crate::case::Op;

    #[test]
    fn minimizer_helpers_reduce_known_case_shape() {
        let ops = vec![
            Op::Feed { offset: 0, len: 2 },
            Op::Feed { offset: 2, len: 2 },
            Op::Seek { offset: 99 },
            Op::Reconfigure { mode: 3, limit: 44 },
            Op::Eos,
        ];

        let merged = merge_adjacent_feeds(&ops);
        assert_eq!(merged[0], Op::Feed { offset: 0, len: 4 });

        let simplified = simplify_values(&merged);
        assert!(simplified.contains(&Op::Seek { offset: 0 }));
        assert!(simplified.contains(&Op::Reconfigure { mode: 0, limit: 0 }));
    }
}
