use std::collections::{BTreeMap, HashMap};

use collab_document::blocks::{Block, DocumentData, DocumentMeta};
use collab_document::document_data::{PAGE, generate_id, page_id_from_document_id};
use serde_json::{Value, json};

use super::schema::{LocalJsonBlock, LocalJsonBlockAppFlowy, LocalJsonDocument};
use super::store::LocalJsonResult;

const APPFLOWY_TEXT_EXTERNAL_TYPE: &str = "text";
const LOCAL_UNSUPPORTED_BLOCK_TYPE: &str = "unsupported";

const INTERNAL_PARAGRAPH: &str = "paragraph";
const INTERNAL_HEADING: &str = "heading";
const INTERNAL_BULLETED_LIST: &str = "bulleted_list";
const INTERNAL_NUMBERED_LIST: &str = "numbered_list";
const INTERNAL_TODO_LIST: &str = "todo_list";
const INTERNAL_QUOTE: &str = "quote";
const INTERNAL_CODE: &str = "code";
const INTERNAL_DIVIDER: &str = "divider";

const LOCAL_HEADING_1: &str = "heading_1";
const LOCAL_HEADING_2: &str = "heading_2";
const LOCAL_HEADING_3: &str = "heading_3";
const LOCAL_TODO: &str = "todo";

pub fn export_document_data_to_json(
  view_id: &str,
  workspace_id: &str,
  title: &str,
  data: &DocumentData,
) -> LocalJsonResult<LocalJsonDocument> {
  let mut document = LocalJsonDocument::new(view_id, workspace_id, title, None);
  document.page_id = Some(data.page_id.clone());

  if let Some(root) = data.blocks.get(&data.page_id) {
    let children = data
      .meta
      .children_map
      .get(&root.children)
      .cloned()
      .unwrap_or_default();

    for block_id in children {
      if let Some(block) = export_block(&block_id, data, &mut document.unsupported_blocks) {
        document.blocks.push(block);
      }
    }
  }

  Ok(document)
}

pub fn import_json_to_document_data(json: &LocalJsonDocument) -> LocalJsonResult<DocumentData> {
  json.validate()?;

  let page_id = json
    .page_id
    .clone()
    .or_else(|| page_id_from_document_id(&json.view_id))
    .unwrap_or_else(generate_id);

  let root = Block {
    id: page_id.clone(),
    ty: PAGE.to_string(),
    parent: String::new(),
    children: page_id.clone(),
    external_id: None,
    external_type: None,
    data: HashMap::new(),
  };

  let mut blocks = HashMap::from([(page_id.clone(), root)]);
  let mut children_map = HashMap::new();
  let mut text_map = HashMap::new();

  let root_children = import_children(
    &json.blocks,
    &page_id,
    &mut blocks,
    &mut children_map,
    &mut text_map,
  );
  children_map.insert(page_id.clone(), root_children);

  Ok(DocumentData {
    page_id,
    blocks,
    meta: DocumentMeta {
      children_map,
      text_map: Some(text_map),
    },
  })
}

fn export_block(
  block_id: &str,
  data: &DocumentData,
  unsupported_blocks: &mut Vec<Value>,
) -> Option<LocalJsonBlock> {
  let block = data.blocks.get(block_id)?;
  let (local_ty, raw_type) = local_type_from_internal(block);
  let raw = raw_type
    .as_ref()
    .and_then(|_| serde_json::to_value(block).ok());

  if let Some(raw) = raw.clone() {
    unsupported_blocks.push(raw);
  }

  let delta = block
    .external_id
    .as_ref()
    .and_then(|external_id| data.meta.text_map.as_ref()?.get(external_id))
    .map(|delta_json| parse_delta_value(delta_json));
  let text = delta.as_ref().map(delta_plain_text);

  let children = data
    .meta
    .children_map
    .get(&block.children)
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .filter_map(|child_id| export_block(&child_id, data, unsupported_blocks))
    .collect();

  let mut extra = BTreeMap::new();
  match block.ty.as_str() {
    INTERNAL_TODO_LIST => {
      if let Some(checked) = block.data.get("checked") {
        extra.insert("checked".to_string(), checked.clone());
      }
    },
    INTERNAL_CODE => {
      if let Some(language) = block.data.get("language") {
        extra.insert("language".to_string(), language.clone());
      }
    },
    _ => {},
  }

  Some(LocalJsonBlock {
    id: Some(block.id.clone()),
    ty: local_ty,
    raw_type,
    text,
    delta,
    children,
    appflowy: Some(LocalJsonBlockAppFlowy {
      ty: Some(block.ty.clone()),
      parent: Some(block.parent.clone()),
      children_key: Some(block.children.clone()),
      external_id: block.external_id.clone(),
      external_type: block.external_type.clone(),
      data: hashmap_to_btree(&block.data),
      extra: BTreeMap::new(),
    }),
    raw,
    extra,
  })
}

fn import_children(
  json_blocks: &[LocalJsonBlock],
  parent_id: &str,
  blocks: &mut HashMap<String, Block>,
  children_map: &mut HashMap<String, Vec<String>>,
  text_map: &mut HashMap<String, String>,
) -> Vec<String> {
  let mut imported_ids = Vec::with_capacity(json_blocks.len());

  for json_block in json_blocks {
    let id = json_block.id.clone().unwrap_or_else(generate_id);
    let appflowy = json_block.appflowy.as_ref();
    let children_key = appflowy
      .and_then(|appflowy| appflowy.children_key.clone())
      .unwrap_or_else(generate_id);
    let (internal_ty, mut data) = internal_type_and_data(json_block);

    let should_store_text = is_text_capable_internal_type(&internal_ty)
      || appflowy
        .and_then(|appflowy| appflowy.external_id.as_ref())
        .is_some();
    let (external_id, external_type) = if should_store_text {
      let external_id = appflowy
        .and_then(|appflowy| appflowy.external_id.clone())
        .unwrap_or_else(generate_id);
      let delta = delta_from_local_block(json_block);
      text_map.insert(external_id.clone(), stringify_json_value(&delta));
      (
        Some(external_id),
        Some(APPFLOWY_TEXT_EXTERNAL_TYPE.to_string()),
      )
    } else {
      (
        appflowy.and_then(|appflowy| appflowy.external_id.clone()),
        appflowy.and_then(|appflowy| appflowy.external_type.clone()),
      )
    };

    if internal_ty == INTERNAL_TODO_LIST {
      data
        .entry("checked".to_string())
        .or_insert_with(|| Value::Bool(false));
    }

    let block = Block {
      id: id.clone(),
      ty: internal_ty,
      parent: parent_id.to_string(),
      children: children_key.clone(),
      external_id,
      external_type,
      data,
    };
    blocks.insert(id.clone(), block);

    let child_ids = import_children(&json_block.children, &id, blocks, children_map, text_map);
    children_map.insert(children_key, child_ids);
    imported_ids.push(id);
  }

  imported_ids
}

fn local_type_from_internal(block: &Block) -> (String, Option<String>) {
  match block.ty.as_str() {
    INTERNAL_PARAGRAPH
    | INTERNAL_BULLETED_LIST
    | INTERNAL_NUMBERED_LIST
    | INTERNAL_QUOTE
    | INTERNAL_CODE
    | INTERNAL_DIVIDER => (block.ty.clone(), None),
    INTERNAL_TODO_LIST => (LOCAL_TODO.to_string(), None),
    INTERNAL_HEADING => match heading_level(block) {
      1 => (LOCAL_HEADING_1.to_string(), None),
      2 => (LOCAL_HEADING_2.to_string(), None),
      3 => (LOCAL_HEADING_3.to_string(), None),
      _ => (
        LOCAL_UNSUPPORTED_BLOCK_TYPE.to_string(),
        Some(block.ty.clone()),
      ),
    },
    _ => (
      LOCAL_UNSUPPORTED_BLOCK_TYPE.to_string(),
      Some(block.ty.clone()),
    ),
  }
}

fn internal_type_and_data(json_block: &LocalJsonBlock) -> (String, HashMap<String, Value>) {
  let mut data = json_block
    .appflowy
    .as_ref()
    .map(|appflowy| btree_to_hashmap(&appflowy.data))
    .unwrap_or_default();

  match json_block.ty.as_str() {
    LOCAL_HEADING_1 => {
      data.insert("level".to_string(), json!(1));
      (INTERNAL_HEADING.to_string(), data)
    },
    LOCAL_HEADING_2 => {
      data.insert("level".to_string(), json!(2));
      (INTERNAL_HEADING.to_string(), data)
    },
    LOCAL_HEADING_3 => {
      data.insert("level".to_string(), json!(3));
      (INTERNAL_HEADING.to_string(), data)
    },
    LOCAL_TODO => {
      if let Some(checked) = json_block.extra.get("checked") {
        data.insert("checked".to_string(), checked.clone());
      }
      (INTERNAL_TODO_LIST.to_string(), data)
    },
    INTERNAL_CODE => {
      if let Some(language) = json_block.extra.get("language") {
        data.insert("language".to_string(), language.clone());
      }
      (INTERNAL_CODE.to_string(), data)
    },
    INTERNAL_PARAGRAPH
    | INTERNAL_BULLETED_LIST
    | INTERNAL_NUMBERED_LIST
    | INTERNAL_QUOTE
    | INTERNAL_DIVIDER => (json_block.ty.clone(), data),
    LOCAL_UNSUPPORTED_BLOCK_TYPE => {
      let fallback_ty = json_block
        .raw_type
        .clone()
        .or_else(|| {
          json_block
            .appflowy
            .as_ref()
            .and_then(|appflowy| appflowy.ty.clone())
        })
        .unwrap_or_else(|| INTERNAL_PARAGRAPH.to_string());
      (fallback_ty, data)
    },
    _ => {
      let fallback_ty = json_block
        .appflowy
        .as_ref()
        .and_then(|appflowy| appflowy.ty.clone())
        .unwrap_or_else(|| INTERNAL_PARAGRAPH.to_string());
      (fallback_ty, data)
    },
  }
}

fn heading_level(block: &Block) -> i64 {
  block.data.get("level").and_then(Value::as_i64).unwrap_or(1)
}

fn is_text_capable_internal_type(ty: &str) -> bool {
  matches!(
    ty,
    INTERNAL_PARAGRAPH
      | INTERNAL_HEADING
      | INTERNAL_BULLETED_LIST
      | INTERNAL_NUMBERED_LIST
      | INTERNAL_TODO_LIST
      | INTERNAL_QUOTE
      | INTERNAL_CODE
  )
}

fn parse_delta_value(delta_json: &str) -> Value {
  serde_json::from_str(delta_json).unwrap_or_else(|_| Value::String(delta_json.to_string()))
}

fn delta_from_local_block(block: &LocalJsonBlock) -> Value {
  block
    .delta
    .clone()
    .unwrap_or_else(|| json!([{ "insert": block.text.clone().unwrap_or_default() }]))
}

fn delta_plain_text(delta: &Value) -> String {
  match delta {
    Value::Array(ops) => ops
      .iter()
      .filter_map(|op| op.get("insert"))
      .map(insert_plain_text)
      .collect::<Vec<_>>()
      .join(""),
    Value::String(text) => text.clone(),
    _ => String::new(),
  }
}

fn insert_plain_text(insert: &Value) -> String {
  match insert {
    Value::String(text) => text.clone(),
    Value::Null => String::new(),
    value => value.to_string(),
  }
}

fn stringify_json_value(value: &Value) -> String {
  serde_json::to_string(value).unwrap_or_else(|_| "[]".to_string())
}

fn hashmap_to_btree(map: &HashMap<String, Value>) -> BTreeMap<String, Value> {
  map
    .iter()
    .map(|(key, value)| (key.clone(), value.clone()))
    .collect()
}

fn btree_to_hashmap(map: &BTreeMap<String, Value>) -> HashMap<String, Value> {
  map
    .iter()
    .map(|(key, value)| (key.clone(), value.clone()))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::local_json::schema::{DOCUMENT_SCHEMA, SCHEMA_VERSION};

  fn document_with_blocks(blocks: Vec<LocalJsonBlock>) -> LocalJsonDocument {
    let mut document = LocalJsonDocument::new("view-1", "workspace-1", "Title", None);
    document.blocks = blocks;
    document
  }

  fn local_block(ty: &str, text: &str) -> LocalJsonBlock {
    let mut block = LocalJsonBlock::paragraph(text);
    block.ty = ty.to_string();
    block
  }

  fn root_children<'a>(data: &'a DocumentData) -> Vec<&'a Block> {
    let root = data.blocks.get(&data.page_id).unwrap();
    data
      .meta
      .children_map
      .get(&root.children)
      .unwrap()
      .iter()
      .map(|id| data.blocks.get(id).unwrap())
      .collect()
  }

  fn text_for_block(data: &DocumentData, block: &Block) -> String {
    let external_id = block.external_id.as_ref().unwrap();
    let delta = data
      .meta
      .text_map
      .as_ref()
      .unwrap()
      .get(external_id)
      .unwrap();
    let delta: Value = serde_json::from_str(delta).unwrap();
    delta_plain_text(&delta)
  }

  #[test]
  fn imports_and_exports_empty_document() {
    let document = document_with_blocks(vec![]);

    let data = import_json_to_document_data(&document).unwrap();
    let root = data.blocks.get(&data.page_id).unwrap();
    assert_eq!(root.ty, PAGE);
    assert!(
      data
        .meta
        .children_map
        .get(&root.children)
        .unwrap()
        .is_empty()
    );

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.page_id, Some(data.page_id));
    assert!(exported.blocks.is_empty());
  }

  #[test]
  fn round_trips_paragraph_text() {
    let document = document_with_blocks(vec![LocalJsonBlock::paragraph("hello from llm")]);

    let data = import_json_to_document_data(&document).unwrap();
    let children = root_children(&data);
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].ty, INTERNAL_PARAGRAPH);
    assert_eq!(text_for_block(&data, children[0]), "hello from llm");

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, INTERNAL_PARAGRAPH);
    assert_eq!(exported.blocks[0].text.as_deref(), Some("hello from llm"));
    assert_eq!(
      exported.blocks[0].delta,
      Some(json!([{ "insert": "hello from llm" }]))
    );
  }

  #[test]
  fn preserves_delta_when_text_is_also_present() {
    let mut block = LocalJsonBlock::paragraph("plain fallback");
    block.delta = Some(json!([
      { "insert": "rich", "attributes": { "bold": true } },
      { "insert": " text" }
    ]));
    let document = document_with_blocks(vec![block]);

    let data = import_json_to_document_data(&document).unwrap();
    let child = root_children(&data)[0];
    let external_id = child.external_id.as_ref().unwrap();
    let stored_delta = data
      .meta
      .text_map
      .as_ref()
      .unwrap()
      .get(external_id)
      .unwrap();
    assert_eq!(
      serde_json::from_str::<Value>(stored_delta).unwrap(),
      json!([
        { "insert": "rich", "attributes": { "bold": true } },
        { "insert": " text" }
      ])
    );

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].text.as_deref(), Some("rich text"));
  }

  #[test]
  fn imports_heading_levels() {
    let document = document_with_blocks(vec![
      local_block(LOCAL_HEADING_1, "one"),
      local_block(LOCAL_HEADING_2, "two"),
      local_block(LOCAL_HEADING_3, "three"),
    ]);

    let data = import_json_to_document_data(&document).unwrap();
    let children = root_children(&data);
    assert_eq!(
      children
        .iter()
        .map(|block| block.ty.as_str())
        .collect::<Vec<_>>(),
      vec![INTERNAL_HEADING, INTERNAL_HEADING, INTERNAL_HEADING]
    );
    assert_eq!(children[0].data.get("level"), Some(&json!(1)));
    assert_eq!(children[1].data.get("level"), Some(&json!(2)));
    assert_eq!(children[2].data.get("level"), Some(&json!(3)));

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(
      exported
        .blocks
        .iter()
        .map(|block| block.ty.as_str())
        .collect::<Vec<_>>(),
      vec![LOCAL_HEADING_1, LOCAL_HEADING_2, LOCAL_HEADING_3]
    );
  }

  #[test]
  fn imports_list_blocks() {
    let document = document_with_blocks(vec![
      local_block(INTERNAL_BULLETED_LIST, "bullet"),
      local_block(INTERNAL_NUMBERED_LIST, "number"),
    ]);

    let data = import_json_to_document_data(&document).unwrap();
    let children = root_children(&data);
    assert_eq!(children[0].ty, INTERNAL_BULLETED_LIST);
    assert_eq!(children[1].ty, INTERNAL_NUMBERED_LIST);

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, INTERNAL_BULLETED_LIST);
    assert_eq!(exported.blocks[1].ty, INTERNAL_NUMBERED_LIST);
  }

  #[test]
  fn preserves_nested_children_order() {
    let mut parent = local_block(INTERNAL_BULLETED_LIST, "parent");
    parent.children = vec![
      local_block(INTERNAL_PARAGRAPH, "child-1"),
      local_block(INTERNAL_PARAGRAPH, "child-2"),
    ];
    let document = document_with_blocks(vec![parent]);

    let data = import_json_to_document_data(&document).unwrap();
    let parent = root_children(&data)[0];
    let child_ids = data.meta.children_map.get(&parent.children).unwrap();
    let child_text = child_ids
      .iter()
      .map(|id| text_for_block(&data, data.blocks.get(id).unwrap()))
      .collect::<Vec<_>>();
    assert_eq!(child_text, vec!["child-1", "child-2"]);

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].children.len(), 2);
    assert_eq!(
      exported.blocks[0].children[0].text.as_deref(),
      Some("child-1")
    );
    assert_eq!(
      exported.blocks[0].children[1].text.as_deref(),
      Some("child-2")
    );
  }

  #[test]
  fn imports_todo_checked_state() {
    let mut checked = local_block(LOCAL_TODO, "done");
    checked.extra.insert("checked".to_string(), json!(true));
    let mut unchecked = local_block(LOCAL_TODO, "later");
    unchecked.extra.insert("checked".to_string(), json!(false));
    let document = document_with_blocks(vec![checked, unchecked]);

    let data = import_json_to_document_data(&document).unwrap();
    let children = root_children(&data);
    assert_eq!(children[0].ty, INTERNAL_TODO_LIST);
    assert_eq!(children[0].data.get("checked"), Some(&json!(true)));
    assert_eq!(children[1].data.get("checked"), Some(&json!(false)));

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, LOCAL_TODO);
    assert_eq!(exported.blocks[0].extra.get("checked"), Some(&json!(true)));
  }

  #[test]
  fn imports_code_block_language() {
    let mut code = local_block(INTERNAL_CODE, "fn main() {}");
    code.extra.insert("language".to_string(), json!("rust"));
    let document = document_with_blocks(vec![code]);

    let data = import_json_to_document_data(&document).unwrap();
    let child = root_children(&data)[0];
    assert_eq!(child.ty, INTERNAL_CODE);
    assert_eq!(child.data.get("language"), Some(&json!("rust")));

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, INTERNAL_CODE);
    assert_eq!(
      exported.blocks[0].extra.get("language"),
      Some(&json!("rust"))
    );
  }

  #[test]
  fn imports_unsupported_block_without_crashing() {
    let mut block = local_block("mystery_box", "kept as text");
    block.raw = Some(json!({ "type": "mystery_box", "payload": { "x": 1 } }));
    let document = document_with_blocks(vec![block]);

    let data = import_json_to_document_data(&document).unwrap();
    let child = root_children(&data)[0];
    assert_eq!(child.ty, INTERNAL_PARAGRAPH);
    assert_eq!(text_for_block(&data, child), "kept as text");

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, INTERNAL_PARAGRAPH);
  }

  #[test]
  fn exports_unknown_internal_block_as_unsupported() {
    let page_id = "page-1".to_string();
    let custom_id = "custom-id".to_string();
    let custom_children = "custom-children".to_string();
    let data = DocumentData {
      page_id: page_id.clone(),
      blocks: HashMap::from([
        (
          page_id.clone(),
          Block {
            id: page_id.clone(),
            ty: PAGE.to_string(),
            parent: String::new(),
            children: page_id.clone(),
            external_id: None,
            external_type: None,
            data: HashMap::new(),
          },
        ),
        (
          custom_id.clone(),
          Block {
            id: custom_id.clone(),
            ty: "custom_internal".to_string(),
            parent: page_id.clone(),
            children: custom_children.clone(),
            external_id: None,
            external_type: None,
            data: HashMap::new(),
          },
        ),
      ]),
      meta: DocumentMeta {
        children_map: HashMap::from([(page_id, vec![custom_id]), (custom_children, vec![])]),
        text_map: Some(HashMap::new()),
      },
    };

    let exported = export_document_data_to_json("view-1", "workspace-1", "Title", &data).unwrap();
    assert_eq!(exported.blocks[0].ty, LOCAL_UNSUPPORTED_BLOCK_TYPE);
    assert_eq!(
      exported.blocks[0].raw_type.as_deref(),
      Some("custom_internal")
    );
    assert_eq!(exported.unsupported_blocks.len(), 1);
  }

  #[test]
  fn schema_round_trip_preserves_unknown_fields() {
    let value = json!({
      "schema": DOCUMENT_SCHEMA,
      "schema_version": SCHEMA_VERSION,
      "view_id": "view-1",
      "workspace_id": "workspace-1",
      "title": "Title",
      "top_level_extra": "kept",
      "blocks": [{
        "type": "paragraph",
        "text": "hello",
        "block_extra": { "nested": true }
      }]
    });

    let document: LocalJsonDocument = serde_json::from_value(value).unwrap();
    assert_eq!(document.extra.get("top_level_extra"), Some(&json!("kept")));
    assert_eq!(
      document.blocks[0].extra.get("block_extra"),
      Some(&json!({ "nested": true }))
    );

    let serialized = serde_json::to_value(document).unwrap();
    assert_eq!(serialized.get("top_level_extra"), Some(&json!("kept")));
    assert_eq!(
      serialized
        .get("blocks")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("block_extra")),
      Some(&json!({ "nested": true }))
    );
  }

  #[test]
  fn rejects_invalid_schema_version() {
    let mut document = document_with_blocks(vec![]);
    document.schema_version = SCHEMA_VERSION + 1;

    let error = import_json_to_document_data(&document).unwrap_err();
    assert!(matches!(
      error,
      crate::local_json::LocalJsonError::UnsupportedSchemaVersion { .. }
    ));
  }
}
