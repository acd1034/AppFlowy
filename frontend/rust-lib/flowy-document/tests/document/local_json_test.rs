use std::fs;

use collab_document::document_data::default_document_data;
use collab_integrate::local_json::{
  JsonStorageConfig, LocalJsonBlock, LocalJsonDocument, LocalJsonStore,
};

use crate::document::util::{DocumentTest, enable_local_json_for_test, gen_document_id};

#[tokio::test]
async fn local_json_imports_document_before_open() {
  enable_local_json_for_test();

  let test = DocumentTest::new();
  let doc_id = gen_document_id();
  let uid = test.user_service.user_id().unwrap();
  test.create_document(uid, &doc_id, None).await.unwrap();

  let workspace_id = test.workspace_id().to_string();
  let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    test.user_data_dir(),
    uid,
    &workspace_id,
  ));
  let mut document =
    LocalJsonDocument::new(doc_id.to_string(), workspace_id, "Imported title", None);
  document.last_writer = "codex".to_string();
  document.blocks = vec![LocalJsonBlock::paragraph("hello from local json")];
  store.write_document(&document).unwrap();

  test.open_document(&doc_id).await.unwrap();

  let text = test.get_document_text(&doc_id).await.unwrap();
  assert_eq!(text, "hello from local json");
}

#[tokio::test]
async fn malformed_local_json_falls_back_to_existing_document() {
  enable_local_json_for_test();

  let test = DocumentTest::new();
  let doc_id = gen_document_id();
  let uid = test.user_service.user_id().unwrap();
  let data = default_document_data(&doc_id.to_string());
  test
    .create_document(uid, &doc_id, Some(data.clone()))
    .await
    .unwrap();

  let workspace_id = test.workspace_id().to_string();
  let store = LocalJsonStore::new(JsonStorageConfig::from_user_data_dir(
    test.user_data_dir(),
    uid,
    workspace_id,
  ));
  fs::create_dir_all(store.documents_dir()).unwrap();
  fs::write(
    store.document_path(&doc_id.to_string()),
    b"{ malformed json",
  )
  .unwrap();

  test.open_document(&doc_id).await.unwrap();

  let opened_data = test.get_document_data(&doc_id).await.unwrap();
  assert_eq!(opened_data.page_id, data.page_id);
  assert!(fs::read_dir(store.backups_dir()).unwrap().next().is_some());
}
