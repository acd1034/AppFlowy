use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::{LocalJsonError, LocalJsonResult};

pub const MANIFEST_SCHEMA: &str = "appflowy.local_json.manifest";
pub const DOCUMENT_SCHEMA: &str = "appflowy.local_json.document";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonManifest {
  pub schema: String,
  pub schema_version: u32,
  pub appflowy_profile_uid: i64,
  pub workspace_id: String,
  pub exported_at: String,
  #[serde(default)]
  pub documents: Vec<LocalJsonManifestDocument>,
  #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
  pub extra: BTreeMap<String, Value>,
}

impl LocalJsonManifest {
  pub fn new(
    appflowy_profile_uid: i64,
    workspace_id: impl Into<String>,
    exported_at: impl Into<String>,
  ) -> Self {
    Self {
      schema: MANIFEST_SCHEMA.to_string(),
      schema_version: SCHEMA_VERSION,
      appflowy_profile_uid,
      workspace_id: workspace_id.into(),
      exported_at: exported_at.into(),
      documents: Vec::new(),
      extra: BTreeMap::new(),
    }
  }

  pub fn validate(&self) -> LocalJsonResult<()> {
    validate_schema(None, MANIFEST_SCHEMA, &self.schema, self.schema_version)
  }

  pub(crate) fn validate_at_path(
    &self,
    path: impl Into<std::path::PathBuf>,
  ) -> LocalJsonResult<()> {
    validate_schema(
      Some(path.into()),
      MANIFEST_SCHEMA,
      &self.schema,
      self.schema_version,
    )
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonManifestDocument {
  pub view_id: String,
  pub title: String,
  pub layout: String,
  pub parent_view_id: Option<String>,
  pub sort_index: Option<i64>,
  pub path: String,
  pub updated_at: Option<String>,
  pub appflowy_updated_at: Option<String>,
  pub external_updated_at: Option<String>,
  pub last_appflowy_export_mtime_ms: Option<u64>,
  pub last_imported_json_mtime_ms: Option<u64>,
  pub last_appflowy_content_hash: Option<String>,
  pub last_json_content_hash: Option<String>,
  #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
  pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonDocument {
  pub schema: String,
  pub schema_version: u32,
  pub view_id: String,
  pub workspace_id: String,
  pub page_id: Option<String>,
  #[serde(default)]
  pub title: String,
  #[serde(default = "default_document_layout")]
  pub layout: String,
  #[serde(default)]
  pub updated_at: Option<String>,
  #[serde(default = "default_last_writer")]
  pub last_writer: String,
  #[serde(default)]
  pub sync: LocalJsonSyncMetadata,
  #[serde(default)]
  pub blocks: Vec<LocalJsonBlock>,
  #[serde(default)]
  pub unsupported_blocks: Vec<Value>,
  #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
  pub extra: BTreeMap<String, Value>,
}

impl LocalJsonDocument {
  pub fn new(
    view_id: impl Into<String>,
    workspace_id: impl Into<String>,
    title: impl Into<String>,
    updated_at: Option<String>,
  ) -> Self {
    Self {
      schema: DOCUMENT_SCHEMA.to_string(),
      schema_version: SCHEMA_VERSION,
      view_id: view_id.into(),
      workspace_id: workspace_id.into(),
      page_id: None,
      title: title.into(),
      layout: default_document_layout(),
      updated_at,
      last_writer: "appflowy".to_string(),
      sync: LocalJsonSyncMetadata::default(),
      blocks: Vec::new(),
      unsupported_blocks: Vec::new(),
      extra: BTreeMap::new(),
    }
  }

  pub fn validate(&self) -> LocalJsonResult<()> {
    validate_schema(None, DOCUMENT_SCHEMA, &self.schema, self.schema_version)
  }

  pub(crate) fn validate_at_path(
    &self,
    path: impl Into<std::path::PathBuf>,
  ) -> LocalJsonResult<()> {
    validate_schema(
      Some(path.into()),
      DOCUMENT_SCHEMA,
      &self.schema,
      self.schema_version,
    )
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonSyncMetadata {
  pub last_appflowy_export_mtime_ms: Option<u64>,
  pub last_imported_json_mtime_ms: Option<u64>,
  pub last_appflowy_content_hash: Option<String>,
  pub last_json_content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonBlock {
  pub id: Option<String>,
  #[serde(rename = "type")]
  pub ty: String,
  #[serde(default)]
  pub raw_type: Option<String>,
  #[serde(default)]
  pub text: Option<String>,
  #[serde(default)]
  pub delta: Option<Value>,
  #[serde(default)]
  pub children: Vec<LocalJsonBlock>,
  #[serde(default)]
  pub appflowy: Option<LocalJsonBlockAppFlowy>,
  #[serde(default)]
  pub raw: Option<Value>,
  #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
  pub extra: BTreeMap<String, Value>,
}

impl LocalJsonBlock {
  pub fn paragraph(text: impl Into<String>) -> Self {
    Self {
      id: None,
      ty: "paragraph".to_string(),
      raw_type: None,
      text: Some(text.into()),
      delta: None,
      children: Vec::new(),
      appflowy: None,
      raw: None,
      extra: BTreeMap::new(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalJsonBlockAppFlowy {
  pub ty: Option<String>,
  pub parent: Option<String>,
  pub children_key: Option<String>,
  pub external_id: Option<String>,
  pub external_type: Option<String>,
  #[serde(default)]
  pub data: BTreeMap<String, Value>,
  #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
  pub extra: BTreeMap<String, Value>,
}

fn validate_schema(
  path: Option<std::path::PathBuf>,
  expected_schema: &'static str,
  actual_schema: &str,
  actual_version: u32,
) -> LocalJsonResult<()> {
  if actual_schema != expected_schema {
    return Err(LocalJsonError::InvalidSchema {
      path,
      expected: expected_schema,
      actual: actual_schema.to_string(),
    });
  }

  if actual_version != SCHEMA_VERSION {
    return Err(LocalJsonError::UnsupportedSchemaVersion {
      path,
      schema: expected_schema,
      version: actual_version,
    });
  }

  Ok(())
}

fn default_document_layout() -> String {
  "document".to_string()
}

fn default_last_writer() -> String {
  "unknown".to_string()
}
