# Local JSON Document Storage

This document describes the MVP local JSON interface for AppFlowy Document
pages. The feature is meant for local-only editing by tools such as Codex while
keeping AppFlowy's runtime Collab/CRDT document model intact.

## Enable

Set the environment variable before starting AppFlowy:

```sh
APPFLOWY_LOCAL_JSON=1
```

When the variable is absent or false, local JSON import and export are no-ops.
No cloud sync or remote API is required.

## Cloud Sync Boundary

The local JSON feature is independent from AppFlowy Cloud sync. It does not add
AppFlowy Cloud dependencies, does not call remote sync APIs, and does not create
collab cloud sync plugins for JSON import/export.

Existing AppFlowy Cloud code remains in place. The JSON sidecar path is gated by
`APPFLOWY_LOCAL_JSON` and uses only local values supplied by the runtime user
services: profile uid, workspace id, and `user_data_dir`. This keeps the feature
usable with the Local provider while avoiding broad upstream merge conflicts
from deleting cloud-related code.

The Step 7 validation flow should use a Local/anonymous profile. Cloud login,
remote collaboration, AppFlowy Cloud workspace sync, and self-hosted sync are
outside the MVP scope.

## Storage Layout

JSON files live next to the existing user data area, not inside the RocksDB
`collab_db` directory:

```text
{user_data_dir}/codex_json/
  workspaces/
    {workspace_id}/
      manifest.json
      documents/
        {view_id}.json
      backups/
```

Document JSON files are pretty-printed UTF-8. Writes use the atomic writer in
`collab-integrate::local_json::LocalJsonStore`.

## Hook Points

Document JSON import runs before the Collab-backed document instance is opened.
Valid JSON is converted to `DocumentData`, encoded as normal Collab document
state, and flushed through the existing local persistence path. The editor still
uses the same runtime Collab/CRDT model after open.

Document JSON export runs after AppFlowy has successfully created, opened, or
edited a Document page. Edit-driven exports are debounced. Folder/View metadata
exports regenerate `manifest.json` from the Folder tree after workspace
initialization and view create, rename, move, trash, restore, or permanent
delete operations.

## MVP Sync Boundary

The MVP detects external edits only at document startup boundaries:

- AppFlowy creates or edits a Document page, then exports
  `documents/{view_id}.json`.
- AppFlowy opens or reopens a Document page, then imports a valid existing JSON
  file before loading the runtime document.
- If no JSON file exists on open, AppFlowy opens the existing internal document
  and exports JSON for Codex to edit later.
- If JSON is malformed, AppFlowy moves a recovery copy to the backup path,
  keeps the existing internal document, and logs a warning.

The MVP intentionally does not run a filesystem watcher, polling task, live
currently-open document update, UI notification, or conflict dialog. Those are
follow-up features.

## Conflict Behavior

The MVP keeps the existing AppFlowy local persistence as the safety net. JSON is
treated as an external sidecar interface:

- AppFlowy-originated edits are exported to JSON after internal persistence has
  succeeded.
- A valid document JSON file found at open/reopen is imported into
  `DocumentData` and then flushed to the existing local Collab persistence.
- A missing document JSON file does not block open; AppFlowy opens the internal
  document and exports a fresh sidecar.
- A malformed document or manifest JSON file is copied to `backups/`, ignored
  for that import/export pass, and the existing AppFlowy document state is kept.

The schema contains sync metadata and content-hash fields for detecting external
changes, but the MVP does not provide a live conflict dialog or automatic
currently-open document merge. If AppFlowy and an external editor both change a
document while it is open, close and reopen the document to exercise the MVP
import boundary. Live conflict handling is a follow-up feature.

## Schema Contract

Document files use this schema:

```json
{
  "schema": "appflowy.codex_json.document",
  "schema_version": 1,
  "view_id": "document-view-id",
  "workspace_id": "workspace-id",
  "title": "Example",
  "layout": "document",
  "blocks": [
    {
      "type": "paragraph",
      "text": "Editable text"
    }
  ],
  "unsupported_blocks": []
}
```

The primary representation is semantic JSON, not an opaque base64 CRDT blob.
Codex may edit `blocks[*].text`, block order, and simple block types. Existing
AppFlowy block ids and metadata are preserved when present.

## Supported Blocks

The MVP codec supports:

- `paragraph`
- `heading_1`
- `heading_2`
- `heading_3`
- `bulleted_list`
- `numbered_list`
- `todo`
- `quote`
- `code`
- `divider`

Unsupported blocks must not panic during import or export. They are represented
as unsupported JSON blocks or diagnostic entries while the existing internal
persistence remains the fallback source of truth.

## Manifest

`manifest.json` is the discovery index for Codex. In the MVP it is exported so
Document page ids, titles, layouts, and paths can be found without reading
AppFlowy's internal databases.

The manifest is regenerated from the current Folder/View tree when the workspace
is initialized and after Document view create, rename, move, trash, restore, or
permanent delete operations. Trash entries are excluded from discovery; restored
Document views are exported again.

Each Document entry includes:

- `view_id`
- `title`
- `layout`
- `parent_view_id`
- `sort_index`
- `path`

When a document JSON file already exists, AppFlowy title changes are mirrored to
that file's top-level `title` field. The folder tree remains the title source of
truth.

Full title import from `manifest.json` or document JSON is a follow-up feature.
Folder/View metadata remains the title source of truth.

## Recovery

When JSON parsing or schema validation fails, AppFlowy does not overwrite the
last valid internal document. The bad file is copied to the workspace backup
directory using a timestamped name:

```text
{user_data_dir}/codex_json/workspaces/{workspace_id}/backups/
```

Inspect the backup to recover manual edits, fix the active JSON file, and reopen
the document. If the active JSON is removed entirely, AppFlowy falls back to its
existing local Collab persistence and can export a new sidecar on open.

## Why Collab Remains

AppFlowy's editor runtime expects Collab/CRDT document data. The local JSON
interface is a persistence boundary for external tools, not a replacement for
the in-memory editor model. Import converts JSON into `DocumentData`, then the
normal local persistence path stores the resulting Collab document.

## Codex Editing Example

1. Start AppFlowy with `APPFLOWY_LOCAL_JSON=1`.
2. Create or open a Document page once so AppFlowy exports
   `manifest.json` and `documents/{view_id}.json`.
3. Use `manifest.json` to find the target Document entry:

```json
{
  "view_id": "9f9fb1c4-43b4-4b00-8384-9d1e78de172a",
  "title": "Example page",
  "layout": "document",
  "path": "documents/9f9fb1c4-43b4-4b00-8384-9d1e78de172a.json"
}
```

4. Edit the referenced document JSON. For a plain paragraph edit, changing
   `blocks[*].text` is enough:

```json
{
  "schema": "appflowy.codex_json.document",
  "schema_version": 1,
  "view_id": "9f9fb1c4-43b4-4b00-8384-9d1e78de172a",
  "workspace_id": "workspace-id",
  "title": "Example page",
  "layout": "document",
  "blocks": [
    {
      "type": "paragraph",
      "text": "hello from codex"
    }
  ],
  "unsupported_blocks": []
}
```

5. Reopen the document in AppFlowy. The MVP imports valid JSON at open/reopen,
   then continues using the normal Collab-backed editor runtime.

Avoid editing JSON for a document that is currently open if you need immediate
UI feedback. Live currently-open document updates are intentionally left to the
follow-up feature set.

## Limitations

- Document pages are the only supported layout.
- Database, Calendar, attachments, comments, and complex page-level features are
  not JSON-primary in the MVP.
- Title export is implemented, but title import from `manifest.json` or
  document JSON is not.
- JSON files alone do not create new AppFlowy views.
- Live filesystem watching and live UI notification are not implemented.
- Unsupported blocks are preserved as unsupported JSON entries where possible,
  but full semantic editing is limited to the supported block list above.

## Follow-up Features

These are not part of the MVP:

- Polling or file watcher based live external edit detection.
- Creating a new AppFlowy view from JSON files alone.
- Full manifest/document title import.
- Live update of the currently open document and UI notifications.
