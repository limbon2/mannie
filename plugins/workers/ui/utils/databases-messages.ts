// Messages for the codes and states of the feature Databases (ADR-0019;
// plugins/workers/docs/databases.md): the commands' errors with the
// service's errors inside them, a file's state, a table's kind, a filter's
// comparison and the failures of the tools' calls. The UI shows the
// catalog's message for a code and never the core's description. Names,
// statements and SQLite's messages are content: they enter a message as its
// arguments and render as text.
import { hostRefusalMessage, ipcFailureMessage } from '@mannie/plugin-sdk/messages';
import type { MentionSummary } from '@mannie/plugin-sdk/slots';

import type { FileRefusal, FileState, FilterOperator, SqliteFilesError, TableKind } from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { DatabasesFailure } from '../types/databases.js';

/** Why a command of Databases failed, and what the owner can do. */
export function databasesFailureMessage(failure: DatabasesFailure): string {
  if (failure.kind === 'ipc') {
    return ipcFailureMessage();
  }
  const { error } = failure;
  if (error.origin === 'host') {
    return hostRefusalMessage(error, m.workers_feature_databases_name());
  }
  switch (error.code) {
    case 'files': {
      return filesErrorMessage(error.error);
    }
    case 'name_taken': {
      return m.workers_databases_error_name_taken();
    }
    case 'stopped': {
      return m.workers_databases_error_stopped();
    }
  }
}

/** Why the core refused a file that the owner chose or accepted. */
function fileRefusalMessage(reason: FileRefusal): string {
  switch (reason) {
    case 'companion': {
      return m.workers_databases_error_file_refused_companion();
    }
    case 'link': {
      return m.workers_databases_error_file_refused_link();
    }
    case 'mannie_folder': {
      return m.workers_databases_error_file_refused_mannie_folder();
    }
    case 'network': {
      return m.workers_databases_error_file_refused_network();
    }
    case 'not_a_database': {
      return m.workers_databases_error_file_refused_not_a_database();
    }
    case 'not_a_file': {
      return m.workers_databases_error_file_refused_not_a_file();
    }
  }
}

/** What an error of the service `sqlite-files` means. */
export function filesErrorMessage(error: SqliteFilesError): string {
  switch (error.code) {
    case 'already_registered': {
      return m.workers_databases_error_already_registered();
    }
    case 'already_run': {
      return m.workers_databases_error_already_run();
    }
    case 'busy': {
      return m.workers_databases_error_busy();
    }
    case 'constraint': {
      return m.workers_databases_error_constraint({ message: error.message });
    }
    case 'failed': {
      return m.workers_databases_error_failed();
    }
    case 'file_changed': {
      return m.workers_databases_error_file_changed();
    }
    case 'file_missing': {
      return m.workers_databases_error_file_missing();
    }
    case 'file_read_only': {
      return m.workers_databases_error_file_read_only();
    }
    case 'file_refused': {
      return fileRefusalMessage(error.reason);
    }
    case 'invalid_filter': {
      return m.workers_databases_error_invalid_filter();
    }
    case 'invalid_name': {
      return m.workers_databases_error_invalid_name();
    }
    case 'invalid_statement': {
      return m.workers_databases_error_invalid_statement({ message: error.message });
    }
    case 'invalid_value': {
      return m.workers_databases_error_invalid_value();
    }
    case 'marked_read_only': {
      return m.workers_databases_error_marked_read_only();
    }
    case 'no_dialog': {
      return m.workers_databases_error_no_dialog();
    }
    case 'no_key': {
      return m.workers_databases_error_no_key();
    }
    case 'not_a_database': {
      return m.workers_databases_error_not_a_database();
    }
    case 'not_a_read': {
      return m.workers_databases_error_not_a_read();
    }
    case 'not_allowed': {
      return m.workers_databases_error_not_allowed();
    }
    case 'not_one_statement': {
      return m.workers_databases_error_not_one_statement();
    }
    case 'not_registered': {
      return m.workers_databases_error_not_registered();
    }
    case 'parameters': {
      return m.workers_databases_error_parameters({ expected: error.expected, given: error.given });
    }
    case 'read_only_host': {
      return m.workers_databases_error_read_only_host();
    }
    case 'refused': {
      return m.workers_databases_error_refused({ message: error.message });
    }
    case 'row_changed': {
      return m.workers_databases_error_row_changed();
    }
    case 'schema_too_large': {
      return m.workers_databases_error_schema_too_large();
    }
    case 'schema_unconfirmed': {
      return m.workers_databases_error_schema_unconfirmed();
    }
    case 'timed_out': {
      return m.workers_databases_error_timed_out();
    }
    case 'too_many_files': {
      return m.workers_databases_error_too_many_files();
    }
    case 'too_many_statements': {
      return m.workers_databases_error_too_many_statements();
    }
  }
}

/** A file's state, as a database's row and page name it. */
export function databaseStateLabel(state: FileState): string {
  switch (state) {
    case 'available': {
      return m.workers_databases_state_available();
    }
    case 'changed': {
      return m.workers_databases_state_changed();
    }
    case 'missing': {
      return m.workers_databases_state_missing();
    }
    case 'unreadable': {
      return m.workers_databases_state_unreadable();
    }
  }
}

/** What a database's page says of a file that cannot run statements, or `null` for one that can. */
export function unusableMessage(state: FileState): string | null {
  switch (state) {
    case 'available': {
      return null;
    }
    case 'changed': {
      return m.workers_databases_unusable_changed();
    }
    case 'missing': {
      return m.workers_databases_unusable_missing();
    }
    case 'unreadable': {
      return m.workers_databases_unusable_unreadable();
    }
  }
}

/** A table's kind. */
export function tableKindLabel(kind: TableKind): string {
  switch (kind) {
    case 'shadow': {
      return m.workers_databases_kind_shadow();
    }
    case 'table': {
      return m.workers_databases_kind_table();
    }
    case 'view': {
      return m.workers_databases_kind_view();
    }
    case 'virtual': {
      return m.workers_databases_kind_virtual();
    }
  }
}

/** How a condition of the Data tab compares its column. */
export function operatorLabel(operator: FilterOperator): string {
  switch (operator) {
    case 'equal': {
      return m.workers_databases_operator_equal();
    }
    case 'greater': {
      return m.workers_databases_operator_greater();
    }
    case 'greater_or_equal': {
      return m.workers_databases_operator_greater_or_equal();
    }
    case 'is_not_null': {
      return m.workers_databases_operator_is_not_null();
    }
    case 'is_null': {
      return m.workers_databases_operator_is_null();
    }
    case 'less': {
      return m.workers_databases_operator_less();
    }
    case 'less_or_equal': {
      return m.workers_databases_operator_less_or_equal();
    }
    case 'like': {
      return m.workers_databases_operator_like();
    }
    case 'not_equal': {
      return m.workers_databases_operator_not_equal();
    }
  }
}

/**
 * The failure codes of the tools' calls: the plugin's own, and the
 * service's, which the plugin passes without their prefix and without the
 * error's fields, which a card does not carry.
 */
const TOOL_FAILURES: ReadonlyMap<string, () => string> = new Map(
  Object.entries({
    already_run: () => m.workers_databases_error_already_run(),
    busy: () => m.workers_databases_error_busy(),
    constraint: () => m.workers_databases_tool_constraint(),
    database_unusable: () => m.workers_databases_tool_database_unusable(),
    failed: () => m.workers_databases_error_failed(),
    file_changed: () => m.workers_databases_error_file_changed(),
    file_missing: () => m.workers_databases_error_file_missing(),
    file_read_only: () => m.workers_databases_error_file_read_only(),
    invalid_arguments: () => m.workers_databases_tool_invalid_arguments(),
    invalid_statement: () => m.workers_databases_tool_invalid_statement(),
    invalid_value: () => m.workers_databases_error_invalid_value(),
    marked_read_only: () => m.workers_databases_error_marked_read_only(),
    no_key: () => m.workers_databases_error_no_key(),
    not_a_database: () => m.workers_databases_error_not_a_database(),
    not_a_read: () => m.workers_databases_error_not_a_read(),
    not_allowed: () => m.workers_databases_error_not_allowed(),
    not_one_statement: () => m.workers_databases_error_not_one_statement(),
    not_registered: () => m.workers_databases_error_not_registered(),
    parameters: () => m.workers_databases_tool_parameters(),
    refused: () => m.workers_databases_tool_refused(),
    row_changed: () => m.workers_databases_error_row_changed(),
    schema_unconfirmed: () => m.workers_databases_error_schema_unconfirmed(),
    timed_out: () => m.workers_databases_error_timed_out(),
    too_many_statements: () => m.workers_databases_error_too_many_statements(),
    unknown_database: () => m.workers_databases_tool_unknown_database(),
  }),
);

/** Why a call of a database tool failed, by its card's code, or `null` for a code that the plugin does not know. */
export function databasesToolFailure(code: string): string | null {
  return TOOL_FAILURES.get(code)?.() ?? null;
}

/** The code that the mention provider gives a key that names no registered database. */
const DATABASE_NOT_FOUND = 'database_not_found';

/** Why "@" could not give a database, or `null` for a code that the provider does not give. */
export function databaseMentionRefusal(code: string): string | null {
  return code === DATABASE_NOT_FOUND ? m.workers_databases_mention_not_found() : null;
}

/** What the model got of a mentioned database, or `null` for a summary that the provider does not give. */
export function databaseMentionSummary(summary: MentionSummary): string | null {
  return summary.code === 'database' ? m.workers_databases_mention_summary() : null;
}

/** What an entry of "@" is, for screen readers. */
export function databaseMentionVariantName(variant: string): string | null {
  return variant === 'database' ? m.workers_databases_mention_variant() : null;
}

/** The words of an entry's warning, or `null` for a code that the provider does not give. */
export function databaseMentionWarning(code: string): string | null {
  return code === 'not_usable' ? m.workers_databases_mention_not_usable() : null;
}
