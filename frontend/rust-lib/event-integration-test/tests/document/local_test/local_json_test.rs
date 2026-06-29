use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use event_integration_test::EventIntegrationTest;
use flowy_folder::entities::UpdateViewPayloadPB;
use serde_json::Value;
use serial_test::serial;
use tokio::time::sleep;

#[tokio::test]
#[serial]
async fn local_json_manifest_exports_document_title_and_path() {
  unsafe {
    std::env::set_var("APPFLOWY_LOCAL_JSON", "1");
  }

  let test = EventIntegrationTest::new().await;
  let user = test.init_anon_user().await;
  let uid = user.id;
  let workspace_id = test.get_workspace_id().await.to_string();
  let workspace_root = PathBuf::from(test.user_data_path())
    .join(uid.to_string())
    .join("codex_json")
    .join("workspaces")
    .join(&workspace_id);

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
