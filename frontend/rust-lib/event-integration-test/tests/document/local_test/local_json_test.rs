use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use collab_document::blocks::DocumentData;
use collab_integrate::local_json::{
  JsonStorageConfig, LocalJsonBlock, LocalJsonDocument, LocalJsonStore,
};
use event_integration_test::event_builder::EventBuilder;
use event_integration_test::EventIntegrationTest;
use flowy_folder::entities::{CreateViewPayloadPB, UpdateViewPayloadPB, ViewLayoutPB, ViewPB};
use flowy_folder::event_map::FolderEvent;
use serde_json::Value;
use serial_test::serial;
use tokio::time::sleep;

#[tokio::test]
#[serial]
async fn local_json_manifest_exports_document_title_and_path() {
  enable_local_json_for_test();

  let (test, _uid, workspace_id, workspace_root) = setup_local_json_test().await;

  let view = test.create_document("Manifest title").await;
  let manifest_path = workspace_root.join("manifest.json");
  let document_path = workspace_root
    .join("documents")
    .join(format!("{}.json", view.id));

  let expected_document_path = format!("documents/{}.json", view.id);
  let manifest = wait_for_manifest_entry(&manifest_path, &view.id, |entry| {
    entry["title"].as_str() == Some("Manifest title")
      && entry["layout"].as_str() == Some("document")
      && entry["parent_view_id"].as_str() == Some(workspace_id.as_str())
      && entry["path"].as_str() == Some(expected_document_path.as_str())
      && entry["sort_index"].is_i64()
  })
  .await;
  let entry = manifest_document(&manifest, &view.id).unwrap();
  assert_eq!(entry["layout"].as_str(), Some("document"));
  assert_eq!(
    entry["parent_view_id"].as_str(),
    Some(workspace_id.as_str())
  );
  assert_eq!(
    entry["path"].as_str(),
    Some(expected_document_path.as_str())
  );
  assert!(entry["sort_index"].is_i64());

  let document = wait_for_document_title(&document_path, "Manifest title").await;
  assert_eq!(document["title"].as_str(), Some("Manifest title"));

  let err = test
    .update_view(UpdateViewPayloadPB {
      view_id: view.id.clone(),
      name: Some("Renamed title".to_string()),
      ..Default::default()
    })
    .await;
  assert!(err.is_none(), "rename failed: {:?}", err);

  let manifest = wait_for_manifest_entry(&manifest_path, &view.id, |entry| {
    entry["title"].as_str() == Some("Renamed title")
      && entry["layout"].as_str() == Some("document")
      && entry["parent_view_id"].as_str() == Some(workspace_id.as_str())
      && entry["path"].as_str() == Some(expected_document_path.as_str())
      && entry["sort_index"].is_i64()
  })
  .await;
  let entry = manifest_document(&manifest, &view.id).unwrap();
  assert_eq!(entry["title"].as_str(), Some("Renamed title"));

  let document = wait_for_document_title(&document_path, "Renamed title").await;
  assert_eq!(document["title"].as_str(), Some("Renamed title"));
}

#[tokio::test]
#[serial]
async fn local_json_exports_document_text_after_appflowy_edit() {
  enable_local_json_for_test();

  let (test, _uid, _workspace_id, workspace_root) = setup_local_json_test().await;
  let view = test.create_document("Export body").await;
  let document_path = workspace_root
    .join("documents")
    .join(format!("{}.json", view.id));

  test
    .insert_document_text(&view.id, "hello from appflowy", 1)
    .await;

  let document = wait_for_document_block_text(&document_path, "hello from appflowy").await;
  assert_eq!(
    document["schema"].as_str(),
    Some("appflowy.codex_json.document")
  );
  assert_eq!(document["view_id"].as_str(), Some(view.id.as_str()));
  assert!(document_blocks_contain_text(
    &document,
    "hello from appflowy"
  ));
}

#[tokio::test]
#[serial]
async fn local_json_imports_external_text_before_first_open() {
  enable_local_json_for_test();

  let (test, uid, workspace_id, _workspace_root) = setup_local_json_test().await;
  let view = create_document_view_without_open(&test, "Import body").await;
  let store = local_json_store(&test, uid, &workspace_id);
  let mut document =
    LocalJsonDocument::new(view.id.clone(), workspace_id.clone(), "Import body", None);
  document.last_writer = "codex".to_string();
  document.blocks = vec![LocalJsonBlock::paragraph("hello from codex")];
  store.write_document(&document).unwrap();

  test.open_document(view.id.clone()).await;
  let data = test.get_document_data(&view.id).await;
  assert_eq!(document_plain_text(&data), "hello from codex");
}

#[tokio::test]
#[serial]
async fn malformed_local_json_is_backed_up_and_existing_document_opens() {
  enable_local_json_for_test();

  let (test, uid, workspace_id, _workspace_root) = setup_local_json_test().await;
  let view = create_document_view_without_open(&test, "Malformed body").await;
  let store = local_json_store(&test, uid, &workspace_id);
  fs::create_dir_all(store.documents_dir()).unwrap();
  fs::write(store.document_path(&view.id), b"{ malformed json").unwrap();

  let opened = test.open_document(view.id.clone()).await;

  assert!(!opened.data.page_id.is_empty());
  assert!(fs::read_dir(store.backups_dir()).unwrap().next().is_some());
}

async fn setup_local_json_test() -> (EventIntegrationTest, i64, String, PathBuf) {
  let test = EventIntegrationTest::new().await;
  let user = test.init_anon_user().await;
  let uid = user.id;
  let workspace_id = test.get_workspace_id().await.to_string();
  let workspace_root = PathBuf::from(test.user_data_path())
    .join(uid.to_string())
    .join("codex_json")
    .join("workspaces")
    .join(&workspace_id);

  (test, uid, workspace_id, workspace_root)
}

fn enable_local_json_for_test() {
  unsafe {
    std::env::set_var("APPFLOWY_LOCAL_JSON", "1");
  }
}

fn local_json_store(test: &EventIntegrationTest, uid: i64, workspace_id: &str) -> LocalJsonStore {
  LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    PathBuf::from(test.user_data_path()).join(uid.to_string()),
    uid,
    workspace_id,
  ))
}

async fn create_document_view_without_open(test: &EventIntegrationTest, name: &str) -> ViewPB {
  EventBuilder::new(test.clone())
    .event(FolderEvent::CreateView)
    .payload(CreateViewPayloadPB {
      parent_view_id: test.get_workspace_id().await.to_string(),
      name: name.to_string(),
      thumbnail: None,
      layout: ViewLayoutPB::Document,
      initial_data: vec![],
      meta: Default::default(),
      // Keep the document out of the active cache so OpenDocument exercises
      // the MVP import-on-open boundary.
      set_as_current: false,
      index: None,
      section: None,
      view_id: None,
      extra: None,
    })
    .async_send()
    .await
    .parse_or_panic::<ViewPB>()
}

async fn wait_for_manifest_entry(
  path: &Path,
  view_id: &str,
  predicate: impl Fn(&Value) -> bool,
) -> Value {
  wait_for_json(path, |value| match manifest_document(value, view_id) {
    Some(document) => predicate(document),
    None => false,
  })
  .await
}

async fn wait_for_document_title(path: &Path, title: &str) -> Value {
  wait_for_json(path, |value| value["title"].as_str() == Some(title)).await
}

async fn wait_for_document_block_text(path: &Path, text: &str) -> Value {
  wait_for_json(path, |value| document_blocks_contain_text(value, text)).await
}

async fn wait_for_json(path: &Path, predicate: impl Fn(&Value) -> bool) -> Value {
  for _ in 0..80 {
    if let Ok(raw) = fs::read_to_string(path) {
      if let Ok(value) = serde_json::from_str::<Value>(&raw) {
        if predicate(&value) {
          return value;
        }
      }
    }
    sleep(Duration::from_millis(50)).await;
  }

  panic!("timed out waiting for JSON file {}", path.display());
}

fn manifest_document<'a>(manifest: &'a Value, view_id: &str) -> Option<&'a Value> {
  manifest["documents"]
    .as_array()?
    .iter()
    .find(|document| document["view_id"].as_str() == Some(view_id))
}

fn document_blocks_contain_text(document: &Value, text: &str) -> bool {
  document["blocks"]
    .as_array()
    .map(|blocks| blocks.iter().any(|block| block_contains_text(block, text)))
    .unwrap_or(false)
}

fn block_contains_text(block: &Value, text: &str) -> bool {
  block["text"].as_str() == Some(text)
    || block["children"]
      .as_array()
      .map(|children| {
        children
          .iter()
          .any(|child| block_contains_text(child, text))
      })
      .unwrap_or(false)
}

fn document_plain_text(data: &DocumentData) -> String {
  let Some(root) = data.blocks.get(&data.page_id) else {
    return String::new();
  };
  let Some(children) = data.meta.children_map.get(&root.children) else {
    return String::new();
  };
  let Some(text_map) = data.meta.text_map.as_ref() else {
    return String::new();
  };

  children
    .iter()
    .filter_map(|block_id| data.blocks.get(block_id))
    .filter_map(|block| block.external_id.as_ref())
    .filter_map(|external_id| text_map.get(external_id))
    .filter_map(|delta| serde_json::from_str::<Value>(delta).ok())
    .map(|delta| delta_plain_text(&delta))
    .collect::<Vec<_>>()
    .join("\n")
}

fn delta_plain_text(delta: &Value) -> String {
  delta
    .as_array()
    .map(|ops| {
      ops
        .iter()
        .filter_map(|op| op.get("insert").and_then(Value::as_str))
        .collect::<String>()
    })
    .unwrap_or_default()
}
