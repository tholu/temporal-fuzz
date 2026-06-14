use crate::case::{Op, TemporalCase};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleMode {
    Boundary,
    Stateful,
    Chaos,
}

impl ScheduleMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "boundary" => Ok(Self::Boundary),
            "stateful" => Ok(Self::Stateful),
            "chaos" => Ok(Self::Chaos),
            _ => Err(format!(
                "invalid mode {value}; expected boundary, stateful, or chaos"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduleGenerator {
    seed: u64,
    rng: Lcg,
}

impl ScheduleGenerator {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            rng: Lcg::new(seed),
        }
    }

    pub fn baseline(payload_len: usize) -> Vec<Op> {
        vec![
            Op::Feed {
                offset: 0,
                len: payload_len,
            },
            Op::Eos,
        ]
    }

    pub fn chunked_interesting(payload: &[u8]) -> Vec<Op> {
        let mut points = split_points(payload);
        points.retain(|point| *point <= payload.len());
        points.sort_unstable();
        points.dedup();
        feed_ops_from_points(payload.len(), &points, true)
    }

    pub fn next_case(
        &mut self,
        input_filename: Option<String>,
        payload: &[u8],
        iteration: usize,
        mode: ScheduleMode,
    ) -> TemporalCase {
        let ops = self.next_ops_for_mode(payload, iteration, mode);
        let mut case = TemporalCase::new(input_filename, payload, ops, Some(self.seed));
        case.description = Some(format!("generated {mode:?} iteration {iteration}"));
        case
    }

    pub fn next_ops_for_mode(
        &mut self,
        payload: &[u8],
        iteration: usize,
        mode: ScheduleMode,
    ) -> Vec<Op> {
        match mode {
            ScheduleMode::Boundary => self.boundary_ops(payload, iteration),
            ScheduleMode::Stateful => self.stateful_ops(payload, iteration),
            ScheduleMode::Chaos => self.chaos_ops(payload, iteration),
        }
    }

    fn chaos_ops(&mut self, payload: &[u8], iteration: usize) -> Vec<Op> {
        if iteration == 0 {
            return Self::baseline(payload.len());
        }
        if iteration == 1 {
            return Self::chunked_interesting(payload);
        }

        match iteration % 5 {
            0 => self.random_chunked(payload.len(), false),
            1 => self.random_chunked(payload.len(), true),
            2 => self.control_heavy(payload.len()),
            3 => self.mutate_ops(Self::chunked_interesting(payload), payload.len()),
            _ => self.header_aware(payload),
        }
    }

    fn boundary_ops(&mut self, payload: &[u8], iteration: usize) -> Vec<Op> {
        if iteration == 0 {
            return Self::baseline(payload.len());
        }
        if iteration == 1 {
            return Self::chunked_interesting(payload);
        }

        match iteration % 4 {
            0 => self.random_chunked(payload.len(), false),
            1 => self.random_chunked(payload.len(), false),
            2 => self.boundary_header_aware(payload),
            _ => self.even_chunks(payload.len()),
        }
    }

    fn stateful_ops(&mut self, payload: &[u8], iteration: usize) -> Vec<Op> {
        if iteration == 0 {
            return Self::baseline(payload.len());
        }

        let mut ops = match iteration % 4 {
            0 => self.random_chunked(payload.len(), false),
            1 => Self::chunked_interesting(payload),
            2 => self.boundary_header_aware(payload),
            _ => self.even_chunks(payload.len()),
        };

        let insert_limit = ops.len().saturating_sub(1).max(1);
        match iteration % 6 {
            0 => ops.insert(self.rng.usize(insert_limit), Op::Flush),
            1 => ops.insert(self.rng.usize(insert_limit), Op::Drain),
            2 => ops.insert(self.rng.usize(insert_limit), Op::FeedZero),
            3 => ops.insert(self.rng.usize(insert_limit), Op::Reset),
            4 => ops.insert(
                self.rng.usize(insert_limit),
                Op::Reconfigure {
                    mode: self.rng.u32(4),
                    limit: self.rng.u32(1024),
                },
            ),
            _ => ops.insert(
                self.rng.usize(insert_limit),
                Op::Seek {
                    offset: self.rng.usize(payload.len().saturating_add(1)),
                },
            ),
        }
        ops
    }

    fn random_chunked(&mut self, payload_len: usize, controls: bool) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut offset = 0;
        while offset < payload_len {
            let remaining = payload_len - offset;
            let max_chunk = remaining.min(1 + self.rng.usize(64));
            let len = 1 + self.rng.usize(max_chunk);
            ops.push(Op::Feed { offset, len });
            offset += len;

            if controls {
                match self.rng.usize(12) {
                    0 => ops.push(Op::Flush),
                    1 => ops.push(Op::Drain),
                    2 => ops.push(Op::FeedZero),
                    3 => ops.push(Op::Seek {
                        offset: self.rng.usize(payload_len.saturating_add(1)),
                    }),
                    _ => {}
                }
            }
        }
        ops.push(Op::Eos);
        ops
    }

    fn control_heavy(&mut self, payload_len: usize) -> Vec<Op> {
        let mut ops = self.random_chunked(payload_len, true);
        let insert_at = self.rng.usize(ops.len().max(1));
        ops.insert(insert_at, Op::Reset);
        ops.insert(
            self.rng.usize(ops.len().max(1)),
            Op::Reconfigure {
                mode: self.rng.u32(4),
                limit: self.rng.u32(1024),
            },
        );
        if self.rng.bool() {
            ops.push(Op::Close);
        }
        ops
    }

    fn header_aware(&mut self, payload: &[u8]) -> Vec<Op> {
        let mut points = split_points(payload);
        let max_extra = payload.len().min(8);
        for _ in 0..max_extra {
            points.push(self.rng.usize(payload.len().saturating_add(1)));
        }
        points.sort_unstable();
        points.dedup();
        let mut ops = feed_ops_from_points(payload.len(), &points, true);
        if self.rng.bool() {
            ops.insert(self.rng.usize(ops.len().max(1)), Op::Flush);
        }
        if self.rng.usize(4) == 0 {
            ops.insert(self.rng.usize(ops.len().max(1)), Op::Drain);
        }
        ops
    }

    fn boundary_header_aware(&mut self, payload: &[u8]) -> Vec<Op> {
        let mut points = split_points(payload);
        let max_extra = payload.len().min(8);
        for _ in 0..max_extra {
            points.push(self.rng.usize(payload.len().saturating_add(1)));
        }
        points.sort_unstable();
        points.dedup();
        feed_ops_from_points(payload.len(), &points, true)
    }

    fn even_chunks(&mut self, payload_len: usize) -> Vec<Op> {
        if payload_len == 0 {
            return Self::baseline(payload_len);
        }

        let chunk_len = 1 + self.rng.usize(payload_len.min(64));
        let mut ops = Vec::new();
        let mut offset = 0;
        while offset < payload_len {
            let len = chunk_len.min(payload_len - offset);
            ops.push(Op::Feed { offset, len });
            offset += len;
        }
        ops.push(Op::Eos);
        ops
    }

    fn mutate_ops(&mut self, mut ops: Vec<Op>, payload_len: usize) -> Vec<Op> {
        if ops.len() > 2 && self.rng.bool() {
            let idx = self.rng.usize(ops.len() - 1);
            ops.remove(idx);
        }

        if ops.len() > 3 && self.rng.bool() {
            let a = self.rng.usize(ops.len() - 1);
            let b = self.rng.usize(ops.len() - 1);
            ops.swap(a, b);
        }

        for op in &mut ops {
            match op {
                Op::Feed { offset, len } => {
                    if self.rng.bool() && *len > 1 {
                        *len = 1 + self.rng.usize(*len);
                    } else if self.rng.bool() {
                        let grow = self.rng.usize(8);
                        *len = (*len + grow).min(payload_len.saturating_sub(*offset));
                    }
                }
                Op::Seek { offset } => *offset = self.rng.usize(payload_len.saturating_add(1)),
                Op::Reconfigure { mode, limit } => {
                    *mode = self.rng.u32(8);
                    *limit = self.rng.u32(4096);
                }
                _ => {}
            }
        }

        let insertions = 1 + self.rng.usize(3);
        for _ in 0..insertions {
            let op = match self.rng.usize(7) {
                0 => Op::FeedZero,
                1 => Op::Flush,
                2 => Op::Drain,
                3 => Op::Reset,
                4 => Op::Seek {
                    offset: self.rng.usize(payload_len.saturating_add(1)),
                },
                5 => Op::Reconfigure {
                    mode: self.rng.u32(4),
                    limit: self.rng.u32(2048),
                },
                _ => Op::Eos,
            };
            ops.insert(self.rng.usize(ops.len().saturating_add(1)), op);
        }

        if !ops.iter().any(|op| matches!(op, Op::Eos | Op::Close)) {
            ops.push(Op::Eos);
        }
        ops
    }
}

fn feed_ops_from_points(payload_len: usize, points: &[usize], eos: bool) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut prev = 0;
    for point in points.iter().copied().filter(|point| *point <= payload_len) {
        if point > prev {
            ops.push(Op::Feed {
                offset: prev,
                len: point - prev,
            });
        }
        prev = point;
    }
    if prev < payload_len {
        ops.push(Op::Feed {
            offset: prev,
            len: payload_len - prev,
        });
    }
    if eos {
        ops.push(Op::Eos);
    }
    ops
}

fn split_points(payload: &[u8]) -> Vec<usize> {
    let len = payload.len();
    let mut points = vec![0, len];

    let mut power = 1;
    while power < len {
        push_around(&mut points, power, len);
        power *= 2;
    }

    if len <= 4096 {
        let step = if len <= 128 { 8 } else { 32 };
        let mut pos = step;
        while pos < len {
            push_around(&mut points, pos, len);
            pos += step;
        }
    }

    for pos in [1usize, 2, 3, 4, 8, 12, 16, 24, 32] {
        push_around(&mut points, pos, len);
    }

    for (idx, window) in payload.windows(2).enumerate() {
        if matches!(
            window,
            b"\r\n" | b"\n\n" | b"PK" | b"\x1f\x8b" | b"{\"" | b"[{"
        ) {
            push_around(&mut points, idx, len);
            push_around(&mut points, idx + 2, len);
        }
    }

    for (idx, byte) in payload.iter().enumerate() {
        if byte.is_ascii_digit() || matches!(*byte, b':' | b',' | b'\n' | 0) {
            push_around(&mut points, idx, len);
        }
    }

    points.sort_unstable();
    points.dedup();
    points
}

fn push_around(points: &mut Vec<usize>, pos: usize, len: usize) {
    for candidate in [pos.saturating_sub(1), pos, pos.saturating_add(1)] {
        if candidate <= len {
            points.push(candidate);
        }
    }
}

#[derive(Debug, Clone)]
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e3779b97f4a7c15,
        }
    }

    fn next(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn usize(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive == 0 {
            return 0;
        }
        (self.next() as usize) % upper_exclusive
    }

    fn u32(&mut self, upper_exclusive: u32) -> u32 {
        if upper_exclusive == 0 {
            return 0;
        }
        (self.next() as u32) % upper_exclusive
    }

    fn bool(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::{ScheduleGenerator, ScheduleMode};
    use crate::case::Op;

    #[test]
    fn schedule_generation_includes_full_and_chunked_modes() {
        let payload = b"LEN:0010\nabcdefghij";
        let mut generator = ScheduleGenerator::new(123);

        let baseline = generator.next_ops_for_mode(payload, 0, ScheduleMode::Boundary);
        assert_eq!(
            baseline,
            vec![
                Op::Feed {
                    offset: 0,
                    len: payload.len()
                },
                Op::Eos
            ]
        );

        let chunked = generator.next_ops_for_mode(payload, 1, ScheduleMode::Boundary);
        let feeds = chunked
            .iter()
            .filter(|op| matches!(op, Op::Feed { .. }))
            .count();
        assert!(feeds > 1, "expected multiple FEED ops, got {chunked:?}");
        assert!(chunked.iter().any(|op| matches!(op, Op::Eos)));
    }

    #[test]
    fn boundary_mode_generates_only_feeds_and_eos() {
        let payload = b"LEN:0010\nabcdefghij";
        let mut generator = ScheduleGenerator::new(123);

        for iteration in 0..100 {
            let ops = generator.next_ops_for_mode(payload, iteration, ScheduleMode::Boundary);
            assert!(
                ops.iter().all(|op| matches!(op, Op::Feed { .. } | Op::Eos)),
                "unexpected boundary op in {ops:?}"
            );
            assert!(matches!(ops.last(), Some(Op::Eos)));
        }
    }
}
