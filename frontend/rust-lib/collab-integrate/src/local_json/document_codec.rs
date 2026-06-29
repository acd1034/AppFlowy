use collab_document::blocks::DocumentData;

use super::schema::LocalJsonDocument;
use super::store::{LocalJsonError, LocalJsonResult};

pub fn export_document_data_to_json(
  _view_id: &str,
  _workspace_id: &str,
  _title: &str,
  _data: &DocumentData,
) -> LocalJsonResult<LocalJsonDocument> {
  Err(LocalJsonError::CodecNotImplemented {
    operation: "export DocumentData to LocalJsonDocument",
  })
}

pub fn import_json_to_document_data(_json: &LocalJsonDocument) -> LocalJsonResult<DocumentData> {
  Err(LocalJsonError::CodecNotImplemented {
    operation: "import LocalJsonDocument to DocumentData",
  })
}
