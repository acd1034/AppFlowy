use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::Weak;

use chrono::{SecondsFormat, Utc};
use collab_folder::{Folder, View, ViewLayout};
use collab_integrate::local_json::{
  JsonStorageConfig, LocalJsonDocument, LocalJsonManifest, LocalJsonManifestDocument,
  LocalJsonStore, SafeJsonRead, manifest_document_from_view,
};
use tracing::warn;
use uuid::Uuid;

use crate::manager::FolderUser;

const LOCAL_JSON_ENV: &str = "APPFLOWY_LOCAL_JSON";

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

pub(crate) fn export_folder_manifest_snapshot(
  user: Weak<dyn FolderUser>,
  workspace_id: Uuid,
  folder: &Folder,
) {
  if !is_local_json_enabled() {
    return;
  }

  let Some(user) = user.upgrade() else {
    return;
  };

  let uid = match user.user_id() {
    Ok(uid) => uid,
    Err(err) => {
      warn!(
        "failed to read user id for local JSON manifest export: {}",
        err
      );
      return;
    },
  };
  let user_data_dir = match user.user_data_dir() {
    Ok(path) => path,
    Err(err) => {
      warn!(
        "failed to read user data dir for local JSON manifest export: {}",
        err
      );
      return;
    },
  };
  let workspace_id = workspace_id.to_string();
  let timestamp = now_rfc3339();
  let documents = manifest_documents_from_folder(folder, &workspace_id, &timestamp);

  tokio::task::spawn_blocking(move || {
    if let Err(err) =
      write_manifest_snapshot(user_data_dir, uid, workspace_id, timestamp, documents)
    {
      warn!("failed to export local JSON manifest: {}", err);
    }
  });
}

fn manifest_documents_from_folder(
  folder: &Folder,
  workspace_id: &str,
  timestamp: &str,
) -> Vec<LocalJsonManifestDocument> {
  let trash_ids = trash_view_ids(folder);
  let mut visited = HashSet::new();
  let mut documents = Vec::new();

  collect_manifest_documents(
    folder,
    workspace_id,
    &trash_ids,
    &mut visited,
    &mut documents,
    timestamp,
  );

  for view in folder.get_all_views() {
    if visited.contains(&view.id) || trash_ids.contains(&view.id) {
      continue;
    }

    if view.layout == ViewLayout::Document {
      documents.push(manifest_document_from_view(
        view.id.clone(),
        view.name.clone(),
        "document",
        Some(view.parent_view_id.clone()),
        sibling_sort_index(folder, &view),
        Some(timestamp.to_string()),
      ));
    }
  }

  documents
}

fn collect_manifest_documents(
  folder: &Folder,
  parent_id: &str,
  trash_ids: &HashSet<String>,
  visited: &mut HashSet<String>,
  documents: &mut Vec<LocalJsonManifestDocument>,
  timestamp: &str,
) {
  let children = folder.get_views_belong_to(parent_id);
  for (sort_index, view) in children.into_iter().enumerate() {
    if !visited.insert(view.id.clone()) || trash_ids.contains(&view.id) {
      continue;
    }

    if view.layout == ViewLayout::Document {
      documents.push(manifest_document_from_view(
        view.id.clone(),
        view.name.clone(),
        "document",
        Some(view.parent_view_id.clone()),
        Some(sort_index as i64),
        Some(timestamp.to_string()),
      ));
    }

    collect_manifest_documents(folder, &view.id, trash_ids, visited, documents, timestamp);
  }
}

fn trash_view_ids(folder: &Folder) -> HashSet<String> {
  let mut ids = HashSet::new();
  for trash in folder.get_all_trash_sections() {
    ids.insert(trash.id.clone());
    for child in folder.get_view_recursively(&trash.id) {
      ids.insert(child.id.clone());
    }
  }
  ids
}

fn sibling_sort_index(folder: &Folder, view: &View) -> Option<i64> {
  folder
    .get_views_belong_to(&view.parent_view_id)
    .iter()
    .position(|sibling| sibling.id == view.id)
    .map(|index| index as i64)
}

fn write_manifest_snapshot(
  user_data_dir: PathBuf,
  uid: i64,
  workspace_id: String,
  timestamp: String,
  documents: Vec<LocalJsonManifestDocument>,
) -> Result<(), collab_integrate::local_json::LocalJsonError> {
  let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    user_data_dir,
    uid,
    &workspace_id,
  ));
  store.ensure_dirs()?;

  let existing_manifest = read_existing_manifest(&store, uid, &workspace_id, &timestamp)?;
  let existing_documents = existing_manifest
    .documents
    .into_iter()
    .map(|document| (document.view_id.clone(), document))
    .collect::<HashMap<_, _>>();

  let mut manifest = LocalJsonManifest::new(uid, workspace_id.clone(), timestamp.clone());
  manifest.extra = existing_manifest.extra;
  manifest.documents = documents
    .into_iter()
    .map(|mut document| {
      if let Some(existing) = existing_documents.get(&document.view_id) {
        document.external_updated_at = existing.external_updated_at.clone();
        document.last_appflowy_export_mtime_ms = existing.last_appflowy_export_mtime_ms;
        document.last_imported_json_mtime_ms = existing.last_imported_json_mtime_ms;
        document.last_appflowy_content_hash = existing.last_appflowy_content_hash.clone();
        document.last_json_content_hash = existing.last_json_content_hash.clone();
        document.extra = existing.extra.clone();
      }
      document
    })
    .collect();

  store.write_manifest(&manifest)?;
  mirror_existing_document_titles(&store, &workspace_id, &timestamp, &manifest.documents);
  Ok(())
}

fn read_existing_manifest(
  store: &LocalJsonStore,
  uid: i64,
  workspace_id: &str,
  timestamp: &str,
) -> Result<LocalJsonManifest, collab_integrate::local_json::LocalJsonError> {
  match store.read_manifest_safely(timestamp)? {
    SafeJsonRead::Valid(manifest) => Ok(manifest),
    SafeJsonRead::Missing => Ok(LocalJsonManifest::new(uid, workspace_id, timestamp)),
    SafeJsonRead::Malformed {
      path,
      backup_path,
      error,
    } => {
      warn!(
        "malformed local JSON manifest was backed up before folder metadata export: path={}, backup_path={:?}, error={}",
        path.display(),
        backup_path,
        error
      );
      Ok(LocalJsonManifest::new(uid, workspace_id, timestamp))
    },
  }
}

fn mirror_existing_document_titles(
  store: &LocalJsonStore,
  workspace_id: &str,
  timestamp: &str,
  documents: &[LocalJsonManifestDocument],
) {
  for document in documents {
    if let Err(err) = mirror_existing_document_title(store, workspace_id, timestamp, document) {
      warn!(
        "failed to mirror local JSON document title for {}: {}",
        document.view_id, err
      );
    }
  }
}

fn mirror_existing_document_title(
  store: &LocalJsonStore,
  workspace_id: &str,
  timestamp: &str,
  manifest_document: &LocalJsonManifestDocument,
) -> Result<(), collab_integrate::local_json::LocalJsonError> {
  let mut document = match store.read_document_safely(&manifest_document.view_id, timestamp)? {
    SafeJsonRead::Valid(document) => document,
    SafeJsonRead::Missing => return Ok(()),
    SafeJsonRead::Malformed {
      path,
      backup_path,
      error,
    } => {
      warn!(
        "malformed local JSON document was backed up before title mirror export: path={}, backup_path={:?}, error={}",
        path.display(),
        backup_path,
        error
      );
      return Ok(());
    },
  };

  if document.view_id != manifest_document.view_id || document.workspace_id != workspace_id {
    warn!(
      "skip local JSON title mirror because document identity does not match manifest: manifest_view_id={}, json_view_id={}, manifest_workspace_id={}, json_workspace_id={}",
      manifest_document.view_id, document.view_id, workspace_id, document.workspace_id
    );
    return Ok(());
  }

  if apply_manifest_title(&mut document, manifest_document, timestamp) {
    store.write_document(&document)?;
  }

  Ok(())
}

fn apply_manifest_title(
  document: &mut LocalJsonDocument,
  manifest_document: &LocalJsonManifestDocument,
  timestamp: &str,
) -> bool {
  let mut changed = false;
  if document.title != manifest_document.title {
    document.title.clone_from(&manifest_document.title);
    changed = true;
  }
  if document.layout != manifest_document.layout {
    document.layout.clone_from(&manifest_document.layout);
    changed = true;
  }

  if changed {
    document.updated_at = Some(timestamp.to_string());
    document.last_writer = "appflowy".to_string();
  }

  changed
}

fn now_rfc3339() -> String {
  Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
