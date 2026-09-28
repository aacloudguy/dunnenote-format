# DunneNote Format 0.18 — draft

> **Draft.** This document is being written from DunneNote's implementation. Sections marked
> *(to be written)* are not yet normative. The schema in `schema/v18.sql` already is.

The key words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described in RFC 2119.

1. Scope and conformance — Reader and Writer levels *(to be written)*
2. Bundle layout: `format.json`, `notebook.db`, `blobs/`, `attachments/`, `.settings/`, `.state/`,
   the lock file, and which parts are durable *(to be written)*
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
