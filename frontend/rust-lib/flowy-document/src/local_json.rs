//! Local JSON sidecar integration for AppFlowy documents.
//!
//! MVP boundary: external JSON edits are imported only while opening or
//! reopening a document. Filesystem watching, polling, currently-open document
//! live updates, and UI notifications are follow-up features.

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use collab_document::blocks::DocumentData;
use collab_integrate::local_json::{
  JsonStorageConfig, LocalJsonDocument, LocalJsonError, LocalJsonManifest, LocalJsonStore,
  SafeJsonRead, export_document_data_to_json, import_json_to_document_data,
  manifest_document_from_view, upsert_manifest_document,
};
use flowy_error::{FlowyError, FlowyResult, internal_error};
use tracing::warn;
use uuid::Uuid;

use crate::manager::DocumentUserService;

const LOCAL_JSON_ENV: &str = "APPFLOWY_LOCAL_JSON";
pub(crate) const LOCAL_JSON_EXPORT_DEBOUNCE: Duration = Duration::from_millis(300);

pub(crate) fn is_local_json_enabled() -> bool {
  env::var(LOCAL_JSON_ENV)
    .map(|value| {
      matches!(
        value.as_str(),
        "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
      )
    })
    .unwrap_or(false)
}

pub(crate) async fn export_document_with_user_service(
  user_service: Arc<dyn DocumentUserService>,
  view_id: Uuid,
  data: DocumentData,
) -> FlowyResult<()> {
  export_document_with_mode(user_service, view_id, data, true).await
}

pub(crate) async fn export_document_with_user_service_if_missing(
  user_service: Arc<dyn DocumentUserService>,
  view_id: Uuid,
  data: DocumentData,
) -> FlowyResult<()> {
  export_document_with_mode(user_service, view_id, data, false).await
}

pub(crate) async fn import_document_with_user_service(
  user_service: Arc<dyn DocumentUserService>,
  view_id: Uuid,
) -> FlowyResult<Option<DocumentData>> {
  if !is_local_json_enabled() {
    return Ok(None);
  }

  let uid = user_service.user_id()?;
  let workspace_id = user_service.workspace_id()?;
  let user_data_dir = user_service.user_data_dir()?;

  tokio::task::spawn_blocking(move || {
    import_document_from_local_json(user_data_dir, uid, workspace_id, view_id)
  })
  .await
  .map_err(internal_error)?
}

async fn export_document_with_mode(
  user_service: Arc<dyn DocumentUserService>,
  view_id: Uuid,
  data: DocumentData,
  overwrite_document: bool,
) -> FlowyResult<()> {
  if !is_local_json_enabled() {
    return Ok(());
  }

  let uid = user_service.user_id()?;
  let workspace_id = user_service.workspace_id()?;
  let user_data_dir = user_service.user_data_dir()?;

  tokio::task::spawn_blocking(move || {
    export_document_to_local_json(
      user_data_dir,
      uid,
      workspace_id,
      view_id,
      data,
      overwrite_document,
    )
  })
  .await
  .map_err(internal_error)?
}

fn export_document_to_local_json(
  user_data_dir: PathBuf,
  uid: i64,
  workspace_id: Uuid,
  view_id: Uuid,
  data: DocumentData,
  overwrite_document: bool,
) -> FlowyResult<()> {
  let workspace_id = workspace_id.to_string();
  let view_id = view_id.to_string();
  let timestamp = now_rfc3339();
  let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    user_data_dir,
    uid,
    &workspace_id,
  ));
  store.ensure_dirs().map_err(local_json_error)?;

  let existing_document = read_existing_document(&store, &view_id, &timestamp)?;
  let existing_manifest = read_existing_manifest(&store, uid, &workspace_id, &timestamp)?;
  let title = local_json_title(&existing_manifest, existing_document.as_ref(), &view_id);

  if overwrite_document || existing_document.is_none() {
    let mut document = export_document_data_to_json(&view_id, &workspace_id, &title, &data)
      .map_err(local_json_error)?;
    document.updated_at = Some(timestamp.clone());
    document.last_writer = "appflowy".to_string();
    if let Some(existing_document) = existing_document {
      document.sync = existing_document.sync;
      document.extra = existing_document.extra;
    }
    store.write_document(&document).map_err(local_json_error)?;
  }

  let mut manifest = existing_manifest;
  manifest.exported_at = timestamp.clone();
  let existing_manifest_document = manifest
    .documents
    .iter()
    .find(|document| document.view_id == view_id)
    .cloned();
  let mut manifest_document = manifest_document_from_view(
    view_id,
    title,
    "document",
    existing_manifest_document
      .as_ref()
      .and_then(|document| document.parent_view_id.clone()),
    existing_manifest_document
      .as_ref()
      .and_then(|document| document.sort_index),
    Some(timestamp),
  );
  if let Some(existing_manifest_document) = existing_manifest_document {
    manifest_document.external_updated_at = existing_manifest_document.external_updated_at;
    manifest_document.last_appflowy_export_mtime_ms =
      existing_manifest_document.last_appflowy_export_mtime_ms;
    manifest_document.last_imported_json_mtime_ms =
      existing_manifest_document.last_imported_json_mtime_ms;
    manifest_document.last_appflowy_content_hash =
      existing_manifest_document.last_appflowy_content_hash;
    manifest_document.last_json_content_hash = existing_manifest_document.last_json_content_hash;
    manifest_document.extra = existing_manifest_document.extra;
  }
  upsert_manifest_document(&mut manifest.documents, manifest_document);
  store.write_manifest(&manifest).map_err(local_json_error)?;

  Ok(())
}

fn import_document_from_local_json(
  user_data_dir: PathBuf,
  uid: i64,
  workspace_id: Uuid,
  view_id: Uuid,
) -> FlowyResult<Option<DocumentData>> {
  let workspace_id = workspace_id.to_string();
  let view_id = view_id.to_string();
  let timestamp = now_rfc3339();
  let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    user_data_dir,
    uid,
    &workspace_id,
  ));

  match store
    .read_document_safely(&view_id, &timestamp)
    .map_err(local_json_error)?
  {
    SafeJsonRead::Valid(document) => {
      if document.view_id != view_id || document.workspace_id != workspace_id {
        warn!(
          "skip local JSON import because document identity does not match path: path_view_id={}, json_view_id={}, path_workspace_id={}, json_workspace_id={}",
          view_id, document.view_id, workspace_id, document.workspace_id
        );
        return Ok(None);
      }

      match import_json_to_document_data(&document) {
        Ok(data) => Ok(Some(data)),
        Err(error) => {
          let backup_path = store.backup_document(&view_id, &timestamp).ok().flatten();
          warn!(
            "failed to import local JSON document {}, backup_path={:?}, error={}",
            view_id, backup_path, error
          );
          Ok(None)
        },
      }
    },
    SafeJsonRead::Missing => Ok(None),
    SafeJsonRead::Malformed {
      path,
      backup_path,
      error,
    } => {
      warn!(
        "malformed local JSON document was backed up before AppFlowy import: path={}, backup_path={:?}, error={}",
        path.display(),
        backup_path,
        error
      );
      Ok(None)
    },
  }
}

fn read_existing_document(
  store: &LocalJsonStore,
  view_id: &str,
  timestamp: &str,
) -> FlowyResult<Option<LocalJsonDocument>> {
  match store
    .read_document_safely(view_id, timestamp)
    .map_err(local_json_error)?
  {
    SafeJsonRead::Valid(document) => Ok(Some(document)),
    SafeJsonRead::Missing => Ok(None),
    SafeJsonRead::Malformed {
      path,
      backup_path,
      error,
    } => {
      warn!(
        "malformed local JSON document was backed up before AppFlowy export: path={}, backup_path={:?}, error={}",
        path.display(),
        backup_path,
        error
      );
      Ok(None)
    },
  }
}

fn read_existing_manifest(
  store: &LocalJsonStore,
  uid: i64,
  workspace_id: &str,
  timestamp: &str,
) -> FlowyResult<LocalJsonManifest> {
  match store
    .read_manifest_safely(timestamp)
    .map_err(local_json_error)?
  {
    SafeJsonRead::Valid(manifest) => Ok(manifest),
    SafeJsonRead::Missing => Ok(LocalJsonManifest::new(uid, workspace_id, timestamp)),
    SafeJsonRead::Malformed {
      path,
      backup_path,
      error,
    } => {
      warn!(
        "malformed local JSON manifest was backed up before AppFlowy export: path={}, backup_path={:?}, error={}",
        path.display(),
        backup_path,
        error
      );
      Ok(LocalJsonManifest::new(uid, workspace_id, timestamp))
    },
  }
}

fn local_json_title(
  manifest: &LocalJsonManifest,
  document: Option<&LocalJsonDocument>,
  view_id: &str,
) -> String {
  manifest
    .documents
    .iter()
    .find(|document| document.view_id == view_id)
    .map(|document| document.title.clone())
    .or_else(|| document.map(|document| document.title.clone()))
    .unwrap_or_default()
}

fn now_rfc3339() -> String {
  Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn local_json_error(error: LocalJsonError) -> FlowyError {
  FlowyError::internal().with_context(error)
}
