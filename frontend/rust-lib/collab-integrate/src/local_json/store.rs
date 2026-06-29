use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::schema::{LocalJsonDocument, LocalJsonManifest};

pub type LocalJsonResult<T> = Result<T, LocalJsonError>;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonConflictPolicy {
  LastWriterWins,
  PreferExternal,
  PreferAppFlowy,
}

impl Default for JsonConflictPolicy {
  fn default() -> Self {
    Self::LastWriterWins
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonStorageConfig {
  pub workspace_root: PathBuf,
  pub appflowy_profile_uid: i64,
  pub workspace_id: String,
  pub conflict_policy: JsonConflictPolicy,
}

impl JsonStorageConfig {
  pub fn new(
    workspace_root: impl Into<PathBuf>,
    appflowy_profile_uid: i64,
    workspace_id: impl Into<String>,
  ) -> Self {
    Self {
      workspace_root: workspace_root.into(),
      appflowy_profile_uid,
      workspace_id: workspace_id.into(),
      conflict_policy: JsonConflictPolicy::default(),
    }
  }

  pub fn from_user_data_dir(
    user_data_dir: impl AsRef<Path>,
    appflowy_profile_uid: i64,
    workspace_id: impl Into<String>,
  ) -> Self {
    let workspace_id = workspace_id.into();
    let workspace_root = user_data_dir
      .as_ref()
      .join("codex_json")
      .join("workspaces")
      .join(&workspace_id);
    Self::new(workspace_root, appflowy_profile_uid, workspace_id)
  }

  pub fn with_conflict_policy(mut self, conflict_policy: JsonConflictPolicy) -> Self {
    self.conflict_policy = conflict_policy;
    self
  }
}

#[derive(Debug, Clone)]
pub struct LocalJsonStore {
  config: JsonStorageConfig,
}

impl LocalJsonStore {
  pub fn new(config: JsonStorageConfig) -> Self {
    Self { config }
  }

  pub fn config(&self) -> &JsonStorageConfig {
    &self.config
  }

  pub fn workspace_root(&self) -> &Path {
    &self.config.workspace_root
  }

  pub fn documents_dir(&self) -> PathBuf {
    self.workspace_root().join("documents")
  }

  pub fn backups_dir(&self) -> PathBuf {
    self.workspace_root().join("backups")
  }

  pub fn lock_path(&self) -> PathBuf {
    self.workspace_root().join(".appflowy-json-lock")
  }

  pub fn manifest_path(&self) -> PathBuf {
    self.workspace_root().join("manifest.json")
  }

  pub fn document_path(&self, view_id: &str) -> PathBuf {
    self
      .documents_dir()
      .join(format!("{}.json", sanitize_path_component(view_id)))
  }

  pub fn backup_path(&self, view_id: &str, timestamp: &str) -> PathBuf {
    self.backups_dir().join(format!(
      "{}_{}.json",
      sanitize_path_component(timestamp),
      sanitize_path_component(view_id)
    ))
  }

  pub fn ensure_dirs(&self) -> LocalJsonResult<()> {
    create_dir_all(self.workspace_root())?;
    create_dir_all(self.documents_dir())?;
    create_dir_all(self.backups_dir())?;
    Ok(())
  }

  pub fn read_manifest(&self) -> LocalJsonResult<Option<LocalJsonManifest>> {
    let path = self.manifest_path();
    read_json(&path, |manifest: &LocalJsonManifest| {
      manifest.validate_at_path(path.clone())
    })
  }

  pub fn read_manifest_safely(
    &self,
    timestamp: &str,
  ) -> LocalJsonResult<SafeJsonRead<LocalJsonManifest>> {
    let path = self.manifest_path();
    read_json_safely(
      &path,
      || self.backup_file(&path, "manifest", timestamp),
      |manifest: &LocalJsonManifest| manifest.validate_at_path(path.clone()),
    )
  }

  pub fn write_manifest(&self, manifest: &LocalJsonManifest) -> LocalJsonResult<()> {
    manifest.validate_at_path(self.manifest_path())?;
    write_json_pretty(&self.manifest_path(), manifest)
  }

  pub fn read_document(&self, view_id: &str) -> LocalJsonResult<Option<LocalJsonDocument>> {
    let path = self.document_path(view_id);
    read_json(&path, |document: &LocalJsonDocument| {
      document.validate_at_path(path.clone())
    })
  }

  pub fn read_document_safely(
    &self,
    view_id: &str,
    timestamp: &str,
  ) -> LocalJsonResult<SafeJsonRead<LocalJsonDocument>> {
    let path = self.document_path(view_id);
    read_json_safely(
      &path,
      || self.backup_file(&path, view_id, timestamp),
      |document: &LocalJsonDocument| document.validate_at_path(path.clone()),
    )
  }

  pub fn write_document(&self, document: &LocalJsonDocument) -> LocalJsonResult<()> {
    document.validate_at_path(self.document_path(&document.view_id))?;
    write_json_pretty(&self.document_path(&document.view_id), document)
  }

  pub fn backup_document(
    &self,
    view_id: &str,
    timestamp: &str,
  ) -> LocalJsonResult<Option<PathBuf>> {
    let path = self.document_path(view_id);
    if !path.exists() {
      return Ok(None);
    }

    self.backup_file(&path, view_id, timestamp).map(Some)
  }

  pub fn backup_file(
    &self,
    source_path: impl AsRef<Path>,
    view_id: &str,
    timestamp: &str,
  ) -> LocalJsonResult<PathBuf> {
    let source_path = source_path.as_ref();
    if !source_path.exists() {
      return Err(LocalJsonError::BackupSourceMissing {
        path: source_path.to_path_buf(),
      });
    }

    let backup_path = self.backup_path(view_id, timestamp);
    if let Some(parent) = backup_path.parent() {
      create_dir_all(parent)?;
    }
    fs::copy(source_path, &backup_path).map_err(|source| LocalJsonError::Io {
      path: backup_path.clone(),
      source,
    })?;
    Ok(backup_path)
  }
}

#[derive(Debug)]
pub enum SafeJsonRead<T> {
  Missing,
  Valid(T),
  Malformed {
    path: PathBuf,
    backup_path: Option<PathBuf>,
    error: LocalJsonError,
  },
}

impl<T> SafeJsonRead<T> {
  pub fn is_valid(&self) -> bool {
    matches!(self, Self::Valid(_))
  }
}

#[derive(Debug)]
pub enum LocalJsonError {
  Io {
    path: PathBuf,
    source: io::Error,
  },
  Serialize {
    path: PathBuf,
    source: serde_json::Error,
  },
  Deserialize {
    path: PathBuf,
    source: serde_json::Error,
  },
  InvalidSchema {
    path: Option<PathBuf>,
    expected: &'static str,
    actual: String,
  },
  UnsupportedSchemaVersion {
    path: Option<PathBuf>,
    schema: &'static str,
    version: u32,
  },
  MissingFileName {
    path: PathBuf,
  },
  BackupSourceMissing {
    path: PathBuf,
  },
  CodecNotImplemented {
    operation: &'static str,
  },
}

impl Display for LocalJsonError {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::Io { path, source } => {
        write!(f, "local JSON IO error at {}: {}", path.display(), source)
      },
      Self::Serialize { path, source } => {
        write!(
          f,
          "local JSON serialize error at {}: {}",
          path.display(),
          source
        )
      },
      Self::Deserialize { path, source } => {
        write!(
          f,
          "local JSON parse error at {}: {}",
          path.display(),
          source
        )
      },
      Self::InvalidSchema {
        path,
        expected,
        actual,
      } => write!(
        f,
        "local JSON schema mismatch at {}: expected {}, got {}",
        display_optional_path(path),
        expected,
        actual
      ),
      Self::UnsupportedSchemaVersion {
        path,
        schema,
        version,
      } => write!(
        f,
        "unsupported local JSON schema version at {}: {} v{}",
        display_optional_path(path),
        schema,
        version
      ),
      Self::MissingFileName { path } => {
        write!(f, "local JSON path has no file name: {}", path.display())
      },
      Self::BackupSourceMissing { path } => {
        write!(
          f,
          "local JSON backup source does not exist: {}",
          path.display()
        )
      },
      Self::CodecNotImplemented { operation } => {
        write!(
          f,
          "local JSON document codec is not implemented yet: {operation}"
        )
      },
    }
  }
}

impl std::error::Error for LocalJsonError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Io { source, .. } => Some(source),
      Self::Serialize { source, .. } | Self::Deserialize { source, .. } => Some(source),
      _ => None,
    }
  }
}

fn read_json<T, F>(path: &Path, validate: F) -> LocalJsonResult<Option<T>>
where
  T: DeserializeOwned,
  F: FnOnce(&T) -> LocalJsonResult<()>,
{
  let bytes = match fs::read(path) {
    Ok(bytes) => bytes,
    Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
    Err(source) => {
      return Err(LocalJsonError::Io {
        path: path.to_path_buf(),
        source,
      });
    },
  };
  let value =
    serde_json::from_slice::<T>(&bytes).map_err(|source| LocalJsonError::Deserialize {
      path: path.to_path_buf(),
      source,
    })?;
  validate(&value)?;
  Ok(Some(value))
}

fn read_json_safely<T, B, F>(
  path: &Path,
  backup: B,
  validate: F,
) -> LocalJsonResult<SafeJsonRead<T>>
where
  T: DeserializeOwned,
  B: FnOnce() -> LocalJsonResult<PathBuf>,
  F: FnOnce(&T) -> LocalJsonResult<()>,
{
  match read_json(path, validate) {
    Ok(Some(value)) => Ok(SafeJsonRead::Valid(value)),
    Ok(None) => Ok(SafeJsonRead::Missing),
    Err(error @ LocalJsonError::Deserialize { .. })
    | Err(error @ LocalJsonError::InvalidSchema { .. })
    | Err(error @ LocalJsonError::UnsupportedSchemaVersion { .. }) => {
      let backup_path = match backup() {
        Ok(path) => Some(path),
        Err(err) => {
          tracing::warn!(
            "failed to backup malformed local JSON {}: {}",
            path.display(),
            err
          );
          None
        },
      };
      Ok(SafeJsonRead::Malformed {
        path: path.to_path_buf(),
        backup_path,
        error,
      })
    },
    Err(error) => Err(error),
  }
}

fn write_json_pretty<T: Serialize>(path: &Path, value: &T) -> LocalJsonResult<()> {
  if let Some(parent) = path.parent() {
    create_dir_all(parent)?;
  }

  let bytes = serde_json::to_vec_pretty(value).map_err(|source| LocalJsonError::Serialize {
    path: path.to_path_buf(),
    source,
  })?;
  atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> LocalJsonResult<()> {
  let tmp_path = tmp_path_for(path)?;
  {
    let mut file = File::create(&tmp_path).map_err(|source| LocalJsonError::Io {
      path: tmp_path.clone(),
      source,
    })?;
    file.write_all(bytes).map_err(|source| LocalJsonError::Io {
      path: tmp_path.clone(),
      source,
    })?;
    file.sync_all().map_err(|source| LocalJsonError::Io {
      path: tmp_path.clone(),
      source,
    })?;
  }

  #[cfg(target_os = "windows")]
  if path.exists() {
    fs::remove_file(path).map_err(|source| LocalJsonError::Io {
      path: path.to_path_buf(),
      source,
    })?;
  }

  fs::rename(&tmp_path, path).map_err(|source| LocalJsonError::Io {
    path: path.to_path_buf(),
    source,
  })?;

  if let Some(parent) = path.parent() {
    if let Ok(dir) = File::open(parent) {
      let _ = dir.sync_all();
    }
  }

  Ok(())
}

fn tmp_path_for(path: &Path) -> LocalJsonResult<PathBuf> {
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .ok_or_else(|| LocalJsonError::MissingFileName {
      path: path.to_path_buf(),
    })?;
  let timestamp_nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_nanos())
    .unwrap_or_default();
  let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
  Ok(path.with_file_name(format!(
    "{file_name}.{}.{}.{}.tmp",
    process::id(),
    timestamp_nanos,
    counter
  )))
}

fn create_dir_all(path: impl AsRef<Path>) -> LocalJsonResult<()> {
  fs::create_dir_all(path.as_ref()).map_err(|source| LocalJsonError::Io {
    path: path.as_ref().to_path_buf(),
    source,
  })
}

fn sanitize_path_component(value: &str) -> String {
  value
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
        ch
      } else {
        '_'
      }
    })
    .collect()
}

fn display_optional_path(path: &Option<PathBuf>) -> String {
  path
    .as_ref()
    .map(|path| path.display().to_string())
    .unwrap_or_else(|| "<unknown>".to_string())
}

#[cfg(test)]
mod tests {
  use std::time::{SystemTime, UNIX_EPOCH};

  use serde_json::json;

  use super::*;
  use crate::local_json::manifest::manifest_document_from_view;
  use crate::local_json::schema::{DOCUMENT_SCHEMA, LocalJsonBlock, SCHEMA_VERSION};

  fn test_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_nanos();
    std::env::temp_dir().join(format!("appflowy_local_json_{name}_{nanos}"))
  }

  fn test_store(name: &str) -> LocalJsonStore {
    LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
      test_root(name),
      1,
      "workspace-1",
    ))
  }

  #[test]
  fn path_generation_uses_workspace_layout() {
    let root = test_root("paths");
    let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
      &root,
      42,
      "workspace-1",
    ));

    assert_eq!(
      store.workspace_root(),
      root
        .join("codex_json")
        .join("workspaces")
        .join("workspace-1")
    );
    assert_eq!(
      store.manifest_path(),
      store.workspace_root().join("manifest.json")
    );
    assert_eq!(
      store.document_path("view-1"),
      store.workspace_root().join("documents").join("view-1.json")
    );
    assert_eq!(
      store.backup_path("view-1", "20260629T000000Z"),
      store
        .workspace_root()
        .join("backups")
        .join("20260629T000000Z_view-1.json")
    );
  }

  #[test]
  fn atomic_write_creates_pretty_manifest_json() {
    let store = test_store("atomic_manifest");
    let mut manifest = LocalJsonManifest::new(1, "workspace-1", "2026-06-29T00:00:00Z");
    manifest.documents.push(manifest_document_from_view(
      "view-1",
      "Example",
      "document",
      Some("workspace-1".to_string()),
      Some(0),
      Some("2026-06-29T00:00:00Z".to_string()),
    ));

    store.write_manifest(&manifest).unwrap();
    let raw = fs::read_to_string(store.manifest_path()).unwrap();
    assert!(raw.contains('\n'));
    assert!(raw.contains("\"schema\": \"appflowy.codex_json.manifest\""));
    assert!(
      !store
        .manifest_path()
        .with_file_name("manifest.json.tmp")
        .exists()
    );
  }

  #[test]
  fn manifest_serializes_and_deserializes() {
    let store = test_store("manifest_round_trip");
    let mut manifest = LocalJsonManifest::new(1, "workspace-1", "2026-06-29T00:00:00Z");
    manifest.documents.push(manifest_document_from_view(
      "view-1",
      "Example",
      "document",
      Some("workspace-1".to_string()),
      Some(0),
      Some("2026-06-29T00:00:00Z".to_string()),
    ));

    store.write_manifest(&manifest).unwrap();
    let read = store.read_manifest().unwrap().unwrap();
    assert_eq!(read, manifest);
  }

  #[test]
  fn document_serializes_and_deserializes() {
    let store = test_store("document_round_trip");
    let mut document = LocalJsonDocument::new(
      "view-1",
      "workspace-1",
      "Example",
      Some("2026-06-29T00:00:00Z".to_string()),
    );
    document.page_id = Some("page-1".to_string());
    document.blocks.push(LocalJsonBlock {
      id: Some("block-1".to_string()),
      ty: "paragraph".to_string(),
      raw_type: None,
      text: Some("hello from codex".to_string()),
      delta: Some(json!([{ "insert": "hello from codex" }])),
      children: Vec::new(),
      appflowy: None,
      raw: None,
      extra: Default::default(),
    });

    store.write_document(&document).unwrap();
    let read = store.read_document("view-1").unwrap().unwrap();
    assert_eq!(read, document);
    assert_eq!(read.schema, DOCUMENT_SCHEMA);
    assert_eq!(read.schema_version, SCHEMA_VERSION);
  }

  #[test]
  fn malformed_json_is_reported_and_backed_up() {
    let store = test_store("malformed");
    store.ensure_dirs().unwrap();
    let path = store.document_path("view-1");
    fs::write(&path, b"{ definitely not json").unwrap();

    let result = store
      .read_document_safely("view-1", "20260629T000000Z")
      .unwrap();
    match result {
      SafeJsonRead::Malformed {
        backup_path: Some(backup_path),
        ..
      } => {
        assert!(backup_path.exists());
        assert_eq!(
          fs::read_to_string(backup_path).unwrap(),
          "{ definitely not json"
        );
      },
      other => panic!("expected malformed read with backup, got {other:?}"),
    }
  }

  #[test]
  fn backup_document_copies_current_json() {
    let store = test_store("backup");
    let mut document = LocalJsonDocument::new(
      "view-1",
      "workspace-1",
      "Example",
      Some("2026-06-29T00:00:00Z".to_string()),
    );
    document.blocks.push(LocalJsonBlock::paragraph("backup me"));
    store.write_document(&document).unwrap();

    let backup_path = store
      .backup_document("view-1", "20260629T000000Z")
      .unwrap()
      .unwrap();
    assert!(backup_path.exists());
    assert_eq!(
      fs::read_to_string(backup_path).unwrap(),
      fs::read_to_string(store.document_path("view-1")).unwrap()
    );
  }
}
