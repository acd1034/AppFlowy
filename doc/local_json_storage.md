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

## Why Collab Remains

AppFlowy's editor runtime expects Collab/CRDT document data. The local JSON
interface is a persistence boundary for external tools, not a replacement for
the in-memory editor model. Import converts JSON into `DocumentData`, then the
normal local persistence path stores the resulting Collab document.

## Follow-up Features

These are not part of the MVP:

- Polling or file watcher based live external edit detection.
- Creating a new AppFlowy view from JSON files alone.
- Full manifest/document title import.
- Live update of the currently open document and UI notifications.
