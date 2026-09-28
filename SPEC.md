# DunneNote Format 0.18 — draft

> **Draft.** This document is being written from DunneNote's implementation. Sections marked
> *(to be written)* are not yet normative. The schema in `schema/v18.sql` already is.

The key words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described in RFC 2119.

1. Scope and conformance — Reader and Writer levels *(to be written)*
2. Bundle layout: `format.json`, `notebook.db`, `blobs/`, `attachments/`, `.settings/`, `.state/`,
   `.archive/`, the lock file, and which parts are durable *(to be written)*
3. SQLite profile: required PRAGMAs, WAL, the version gate *(to be written)*
4. Schema: `schema/v18.sql` is normative; table-by-table semantics *(to be written)*
5. Identifiers, sibling positions, timestamps *(to be written)*
6. Blobs: addressing, sentinel blobs, reference counts *(to be written)*
7. Canvases: common columns, settings merge rule, reserved settings keys *(to be written)*
8. Payloads: rich text, sketch, tables, calendar, per-kind settings *(to be written)*
9. Roles: captions, forms, templates *(to be written)*
10. Groups, layering, archive *(to be written)*
11. Tags and metadata, fold v1 *(to be written)*
12. The search index as a derived cache *(to be written)*
13. Writer checklist *(to be written)*
14. Versioning policy and changelog *(to be written)*

## Notes gathered so far (not yet normative)

Observed in notebooks written by DunneNote itself (see `fixtures/`), to be folded into the
sections above:

- **`.archive/`** holds one compressed snapshot per archived page or section
  (`<node id>.tar.gz`, holding `manifest.json` with SHA-256 checksums and `nodes.jsonl`). The
  live rows stay the source of truth (`nodes.is_archived` and its
  reason columns); retrieving deletes the snapshot. Readers MAY ignore the folder; writers MUST
  NOT delete it.
- **Archiving a page does not change its canvases.** Their `lifecycle` stays `active`; a reader
  that hides archived content checks the page (and its ancestors) as well as each canvas.
- **Tag `source_kind`.** Every canvas, rich text included, is tagged as `instance`
  (`canvas_instances.id`). The value `canvas` is accepted by the schema but refers to a table
  that no longer exists; writers SHOULD NOT use it.
- **Layering.** New canvases go into the top `z_index` (0 on a fresh page) and take the next
  `z_minor`, so a page's canvases share one major layer until the user moves one.
- **Forms.** A field is a canvas whose settings carry `formField` (`name`, and `label`,
  `required`, `labelDisplay` only when not the default). The answers table is an Editable table
  whose settings carry `formTarget: {"ownedByForm": true}`. The page's settings name it with
  `formDestination: {"kind": "canvas", "id": …}` and give the field order in `formTabOrder`.
  Each submission is one table row; the reserved `Submitted` column holds an RFC 3339 time with
  offset. A drawn answer is stored as the cell text `sketch:<id>` pointing at a canvas with
  `hidden: true`, which is real content and not an orphan.
- **Templates.** A template page has `is_template = 1` and its name ends in ` (template)`.
  Pictures in it point at the placeholder sentinel blob with settings `{"placeholder": true}`;
  text canvases keep their content only when their settings carry `templateKeepContent: true`.
- **Blank Editable tables** are 3 columns × 3 rows with column keys `c0`, `c1`, `c2` and names
  `Column 1` … `Column 3`, and `datasets.source_kind` = `paste`. A blank form answers table
  starts with no columns and records `source_kind` = `csv`.
- **Cleared text** in a template (and in pages made from it) is the empty document
  `{"type":"doc","content":[{"type":"paragraph"}]}`.
