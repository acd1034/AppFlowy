use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub type ContentHash = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChangeState {
  Missing,
  Unchanged,
  Changed,
}

pub fn content_hash(bytes: &[u8]) -> ContentHash {
  let mut hash = 0xcbf29ce484222325u64;
  for byte in bytes {
    hash ^= u64::from(*byte);
    hash = hash.wrapping_mul(0x100000001b3);
  }
  format!("{hash:016x}")
}

pub fn file_mtime_ms(path: impl AsRef<Path>) -> Option<u64> {
  let modified = fs::metadata(path).ok()?.modified().ok()?;
  let duration = modified.duration_since(UNIX_EPOCH).ok()?;
  Some(duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

pub fn has_external_change(
  current_mtime_ms: Option<u64>,
  current_hash: Option<&str>,
  last_appflowy_export_mtime_ms: Option<u64>,
  last_json_content_hash: Option<&str>,
) -> ExternalChangeState {
  let Some(current_mtime_ms) = current_mtime_ms else {
    return ExternalChangeState::Missing;
  };

  if last_appflowy_export_mtime_ms
    .map(|last| current_mtime_ms > last)
    .unwrap_or(false)
  {
    return ExternalChangeState::Changed;
  }

  match (current_hash, last_json_content_hash) {
    (Some(current), Some(last)) if current != last => ExternalChangeState::Changed,
    (Some(_), Some(_)) => ExternalChangeState::Unchanged,
    _ => ExternalChangeState::Unchanged,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn content_hash_is_stable() {
    assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
    assert_ne!(content_hash(b"hello"), content_hash(b"hello!"));
  }
}
