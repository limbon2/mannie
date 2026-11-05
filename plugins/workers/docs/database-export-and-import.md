# Database export and import

|              |                                                                                                                                                                                                             |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Status       | Idea                                                                                                                                                                                                        |
| Milestone    | Unscheduled                                                                                                                                                                                                 |
| Last updated | 2026-11-01                                                                                                                                                                                                  |
| Part of      | [Workers](README.md)                                                                                                                                                                                        |
| Related      | [Databases](databases.md), [ADR-0040](../../../docs/adr/0040-export-complete-conversation-records.md), [ADR-0056](../../../docs/adr/0056-let-plugins-work-with-the-owners-sqlite-files-through-the-core.md) |

## Intent

Adminer exports a table or a query's result as CSV or SQL and imports such files. The first version of [databases](databases.md) copies a page as CSV and has no export or import of files. The owner would save a table, a result or a whole database as CSV or SQL, and load a CSV or SQL file into a table.

## Open questions

- How the owner chooses the destination and the source: through the native dialog, as conversation export does ([ADR-0040](../../../docs/adr/0040-export-complete-conversation-records.md), decision 5), which needs a dialog for saving that the `file-dialog` service does not offer.
- Whether an import is one owner operation or one for each statement, and how it reports the rows that failed.
- How an export or an import works for a worker's database: on the worker's host, or streamed to the master over the link.
