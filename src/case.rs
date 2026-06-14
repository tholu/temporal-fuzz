use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Op {
    Feed { offset: usize, len: usize },
    FeedZero,
    Flush,
    Drain,
    Reset,
    Reconfigure { mode: u32, limit: u32 },
    Seek { offset: usize },
    Eos,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalCase {
    pub input_filename: Option<String>,
    pub payload_b64: String,
    pub payload_hash: String,
    pub ops: Vec<Op>,
    pub seed: Option<u64>,
    pub description: Option<String>,
}

impl TemporalCase {
    pub fn new(
        input_filename: Option<String>,
        payload: &[u8],
        ops: Vec<Op>,
        seed: Option<u64>,
    ) -> Self {
        Self {
            input_filename,
            payload_b64: base64_encode(payload),
            payload_hash: stable_hash_hex(payload),
            ops,
            seed,
            description: None,
        }
    }

    pub fn payload(&self) -> Result<Vec<u8>, String> {
        base64_decode(&self.payload_b64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Ok,
    Crash,
    Timeout,
    InvalidStdout,
    SpawnError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterOutput {
    pub status: String,
    pub output_hash: String,
    #[serde(default)]
    pub observations: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub kind: OutcomeKind,
    pub adapter: Option<AdapterOutput>,
    pub exit_code: Option<i32>,
    pub stderr: String,
    pub runtime_ms: u128,
    pub max_rss_kb: Option<u64>,
    pub stdout_snippet: String,
    pub error: Option<String>,
}

impl RunOutcome {
    pub fn status(&self) -> Option<&str> {
        self.adapter.as_ref().map(|adapter| adapter.status.as_str())
    }

    pub fn output_hash(&self) -> Option<&str> {
        self.adapter
            .as_ref()
            .map(|adapter| adapter.output_hash.as_str())
    }

    pub fn stderr_snippet(&self) -> String {
        snippet(&self.stderr, 4096)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingClass {
    Crash,
    Hang,
    Divergence,
    Interesting,
}

impl FindingClass {
    pub fn dir_name(self) -> &'static str {
        match self {
            FindingClass::Crash => "crashes",
            FindingClass::Hang => "hangs",
            FindingClass::Divergence => "divergences",
            FindingClass::Interesting => "interesting",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub finding_class: FindingClass,
    pub input_filename: Option<String>,
    pub payload_hash: String,
    pub payload_b64: String,
    pub schedule: Vec<Op>,
    pub baseline_result: RunOutcome,
    pub variant_result: RunOutcome,
    pub stderr_snippets: Vec<String>,
    pub command_line: Vec<String>,
    pub timestamp: u64,
}

impl Finding {
    pub fn payload(&self) -> Result<Vec<u8>, String> {
        base64_decode(&self.payload_b64)
    }
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

pub fn snippet(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        out.push(B64[(b0 >> 2) as usize] as char);
        out.push(B64[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut clean = Vec::new();
    for byte in input.bytes() {
        if !byte.is_ascii_whitespace() {
            clean.push(byte);
        }
    }
    if clean.len() % 4 != 0 {
        return Err("base64 length is not a multiple of 4".to_string());
    }

    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for block in clean.chunks(4) {
        let pad = block.iter().rev().take_while(|byte| **byte == b'=').count();
        let mut value = [0u8; 4];
        for (idx, byte) in block.iter().enumerate() {
            value[idx] = match *byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => 0,
                _ => return Err(format!("invalid base64 byte: {byte}")),
            };
        }

        out.push((value[0] << 2) | (value[1] >> 4));
        if pad < 2 {
            out.push((value[1] << 4) | (value[2] >> 2));
        }
        if pad < 1 {
            out.push((value[2] << 6) | value[3]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{base64_decode, base64_encode, stable_hash_hex};

    #[test]
    fn base64_round_trips_payloads() {
        for payload in [
            b"".as_slice(),
            b"a",
            b"ab",
            b"abc",
            b"abcdef",
            b"\0\xffpayload",
        ] {
            let encoded = base64_encode(payload);
            assert_eq!(base64_decode(&encoded).unwrap(), payload);
        }
    }

    #[test]
    fn stable_hash_is_stable() {
        assert_eq!(stable_hash_hex(b"abc"), "fnv1a64:e71fa2190541574b");
    }
}
