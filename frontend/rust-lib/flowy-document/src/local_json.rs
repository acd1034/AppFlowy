use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use collab_document::blocks::DocumentData;
use collab_integrate::local_json::{
  JsonStorageConfig, LocalJsonDocument, LocalJsonError, LocalJsonManifest, LocalJsonStore,
  SafeJsonRead, export_document_data_to_json, manifest_document_from_view,
  upsert_manifest_document,
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
  if !is_local_json_enabled() {
    return Ok(());
  }

  let uid = user_service.user_id()?;
  let workspace_id = user_service.workspace_id()?;
  let user_data_dir = user_service.user_data_dir()?;

  tokio::task::spawn_blocking(move || {
    export_document_to_local_json(user_data_dir, uid, workspace_id, view_id, data)
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

  let mut document = export_document_data_to_json(&view_id, &workspace_id, &title, &data)
    .map_err(local_json_error)?;
  document.updated_at = Some(timestamp.clone());
  document.last_writer = "appflowy".to_string();
  if let Some(existing_document) = existing_document {
    document.sync = existing_document.sync;
    document.extra = existing_document.extra;
  }
  store.write_document(&document).map_err(local_json_error)?;

  let mut manifest = existing_manifest;
  manifest.exported_at = timestamp.clone();
  upsert_manifest_document(
    &mut manifest.documents,
    manifest_document_from_view(view_id, title, "document", None, None, Some(timestamp)),
  );
  store.write_manifest(&manifest).map_err(local_json_error)?;

  Ok(())
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
