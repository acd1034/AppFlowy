use super::schema::LocalJsonManifestDocument;

pub fn document_relative_path(view_id: &str) -> String {
  format!("documents/{view_id}.json")
}

#[allow(clippy::too_many_arguments)]
pub fn manifest_document_from_view(
  view_id: impl Into<String>,
  title: impl Into<String>,
  layout: impl Into<String>,
  parent_view_id: Option<String>,
  sort_index: Option<i64>,
  updated_at: Option<String>,
) -> LocalJsonManifestDocument {
  let view_id = view_id.into();
  LocalJsonManifestDocument {
    path: document_relative_path(&view_id),
    view_id,
    title: title.into(),
    layout: layout.into(),
    parent_view_id,
    sort_index,
    updated_at: updated_at.clone(),
    appflowy_updated_at: updated_at,
    external_updated_at: None,
    last_appflowy_export_mtime_ms: None,
    last_imported_json_mtime_ms: None,
    last_appflowy_content_hash: None,
    last_json_content_hash: None,
    extra: Default::default(),
  }
}

pub fn upsert_manifest_document(
  documents: &mut Vec<LocalJsonManifestDocument>,
  document: LocalJsonManifestDocument,
) {
  match documents
    .iter_mut()
    .find(|existing| existing.view_id == document.view_id)
  {
    Some(existing) => *existing = document,
    None => documents.push(document),
  }
}
