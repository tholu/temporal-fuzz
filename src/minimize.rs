use crate::case::{FindingClass, Op, OutcomeKind, RunOutcome, ScheduleClass, ScheduleMetadata};
use crate::classifier::classify;
use crate::runner::run_adapter;
use crate::schedule::ScheduleMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePredicate {
    class: FindingClass,
    baseline_kind: OutcomeKind,
    variant_kind: OutcomeKind,
    baseline_status: Option<String>,
    variant_status: Option<String>,
    baseline_hash: Option<String>,
    variant_hash: Option<String>,
    variant_exit_code: Option<i32>,
    status_mismatch: bool,
    hash_mismatch: bool,
}

impl FailurePredicate {
    pub fn new(baseline: &RunOutcome, variant: &RunOutcome) -> Option<Self> {
        let class = classify(baseline, variant)?.class;
        Some(Self {
            class,
            baseline_kind: baseline.kind.clone(),
            variant_kind: variant.kind.clone(),
            baseline_status: baseline.status().map(ToString::to_string),
            variant_status: variant.status().map(ToString::to_string),
            baseline_hash: baseline.output_hash().map(ToString::to_string),
            variant_hash: variant.output_hash().map(ToString::to_string),
            variant_exit_code: variant.exit_code,
            status_mismatch: baseline.status() != variant.status(),
            hash_mismatch: baseline.output_hash() != variant.output_hash(),
        })
    }

    pub fn class(&self) -> FindingClass {
        self.class
    }

    fn matches(&self, baseline: &RunOutcome, variant: &RunOutcome) -> bool {
        if baseline.kind != self.baseline_kind || variant.kind != self.variant_kind {
            return false;
        }
        if variant.kind == OutcomeKind::Crash
            && self.variant_exit_code.is_some()
            && variant.exit_code != self.variant_exit_code
        {
            return false;
        }
        if baseline.status().map(ToString::to_string) != self.baseline_status {
            return false;
        }
        if baseline.output_hash().map(ToString::to_string) != self.baseline_hash {
            return false;
        }

        match self.class {
            FindingClass::Divergence => {
                if self.status_mismatch
                    && variant.status().map(ToString::to_string) != self.variant_status
                {
                    return false;
                }
                if self.hash_mismatch
                    && variant.output_hash().map(ToString::to_string) != self.variant_hash
                {
                    return false;
                }
                true
            }
            FindingClass::Crash | FindingClass::Hang => true,
            FindingClass::Interesting => classify(baseline, variant)
                .map(|classification| classification.class == self.class)
                .unwrap_or(false),
        }
    }
}

pub fn minimize_schedule(
    target: &[String],
    payload: &[u8],
    initial_ops: &[Op],
    baseline: &RunOutcome,
    predicate: &FailurePredicate,
    timeout_ms: u64,
    mode: ScheduleMode,
) -> (Vec<Op>, RunOutcome) {
    let mut ops = initial_ops.to_vec();
    let mut outcome = run_adapter(target, payload, &ops, timeout_ms);
    if !candidate_allowed(payload, &ops, mode) || !predicate.matches(baseline, &outcome) {
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
            if !candidate_allowed(payload, &candidate, mode) {
                continue;
            }
            let candidate_outcome = run_adapter(target, payload, &candidate, timeout_ms);
            if predicate.matches(baseline, &candidate_outcome) {
                ops = candidate;
                outcome = candidate_outcome;
                changed = true;
                break;
            }
        }
    }

    let merged = merge_adjacent_feeds(&ops);
    let merged_outcome = run_adapter(target, payload, &merged, timeout_ms);
    if candidate_allowed(payload, &merged, mode) && predicate.matches(baseline, &merged_outcome) {
        ops = merged;
        outcome = merged_outcome;
    }

    let simplified = simplify_values(&ops);
    let simplified_outcome = run_adapter(target, payload, &simplified, timeout_ms);
    if candidate_allowed(payload, &simplified, mode)
        && predicate.matches(baseline, &simplified_outcome)
    {
        ops = simplified;
        outcome = simplified_outcome;
    }

    let context = ShrinkContext {
        target,
        payload,
        baseline,
        predicate,
        timeout_ms,
        mode,
    };
    shrink_feed_lengths(&context, &mut ops, &mut outcome);
    (ops, outcome)
}

struct ShrinkContext<'a> {
    target: &'a [String],
    payload: &'a [u8],
    baseline: &'a RunOutcome,
    predicate: &'a FailurePredicate,
    timeout_ms: u64,
    mode: ScheduleMode,
}

fn candidate_allowed(payload: &[u8], ops: &[Op], mode: ScheduleMode) -> bool {
    if mode != ScheduleMode::Boundary {
        return true;
    }
    let metadata = ScheduleMetadata::from_ops(payload, ops);
    metadata.baseline_equivalent && metadata.schedule_class == ScheduleClass::EquivalentBoundary
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

fn shrink_feed_lengths(context: &ShrinkContext<'_>, ops: &mut Vec<Op>, outcome: &mut RunOutcome) {
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
            if !candidate_allowed(context.payload, &candidate, context.mode) {
                if candidate_len == 1 {
                    break;
                }
                continue;
            }
            let candidate_outcome = run_adapter(
                context.target,
                context.payload,
                &candidate,
                context.timeout_ms,
            );
            if context
                .predicate
                .matches(context.baseline, &candidate_outcome)
            {
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
