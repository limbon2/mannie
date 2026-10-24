// What the server dialog sends (plugins/mcp/docs/README.md, registering a
// server): its fields as the plugin's input, with the arguments one on each
// line and a variable or header whose name looks like a secret's among the
// secrets. The core checks every field again (ADR-0043, decision 5), and
// these rules only spare the owner a refusal.
import type { NamedValue, SecretValue, ServerInput, ServerTransport } from '../generated/bindings.js';

/** Parts of a name that mark a secret, as the core's rule names them in the SDK's plugin rules. */
const SECRET_NAME_PARTS = new Set([
  'KEY',
  'APIKEY',
  'TOKEN',
  'SECRET',
  'PASSWORD',
  'PASSWD',
  'CREDENTIAL',
  'CREDENTIALS',
]);

/** Headers that always carry a credential, in lower case. */
const SECRET_HEADERS = new Set(['authorization', 'cookie', 'proxy-authorization', 'x-api-key']);

/** A row of plain values, as the dialog edits it. */
export interface PlainRow {
  name: string;
  value: string;
}

/** A row of secret values, as the dialog edits it: an empty value of a stored secret keeps it. */
export interface SecretRow {
  name: string;
  /** Whether the core keeps a value of this name. */
  readonly stored: boolean;
  value: string;
}

/** The fields of the dialog. */
export interface ServerFields {
  argumentsText: string;
  clientId: string;
  clientSecret: string;
  folder: string;
  forgetClientSecret: boolean;
  images: boolean;
  instructions: boolean;
  name: string;
  oauth: boolean;
  plain: PlainRow[];
  prefix: string;
  program: string;
  secrets: SecretRow[];
  timeoutSeconds: number;
  transport: ServerTransport;
  url: string;
}

/** Whether a name looks like a secret's, as the core decides it: one of its words is a secret's part. */
export function looksLikeSecret(name: string): boolean {
  return name.split(/[^A-Za-z0-9]/u).some((part) => SECRET_NAME_PARTS.has(part.toUpperCase()));
}

/** Whether a variable or a header of a server of `transport` must be secret. */
export function mustBeSecret(transport: ServerTransport, name: string): boolean {
  const isSecretHeader = transport === 'http' && SECRET_HEADERS.has(name.toLowerCase());
  return isSecretHeader || looksLikeSecret(name);
}

/** The arguments of a text with one on each line, without empty lines. */
export function argumentsOf(text: string): string[] {
  return text.split(/\r?\n/u).filter((line) => line !== '');
}

/** The plain rows that must be secret, by name, which the dialog moves among the secrets. */
export function rowsToMove(fields: Pick<ServerFields, 'plain' | 'transport'>): string[] {
  return fields.plain
    .map((row) => row.name.trim())
    .filter((name) => {
      const hasName = name !== '';
      return hasName && mustBeSecret(fields.transport, name);
    });
}

/** The plugin's input from the dialog's fields. Rows without a name are left out. */
export function inputOf(fields: ServerFields): ServerInput {
  const isStdio = fields.transport === 'stdio';
  const plain: NamedValue[] = fields.plain
    .filter((row) => row.name.trim() !== '')
    .map((row) => ({ name: row.name.trim(), value: row.value }));
  const secrets: SecretValue[] = fields.secrets
    .filter((row) => row.name.trim() !== '')
    .map((row) => {
      const keeps = row.stored && row.value === '';
      return { name: row.name.trim(), value: keeps ? null : row.value };
    });
  const clientSecret = fields.clientSecret === '' ? null : fields.clientSecret;
  return {
    arguments: isStdio ? argumentsOf(fields.argumentsText) : [],
    clientId: isStdio || !fields.oauth ? '' : fields.clientId.trim(),
    clientSecret: isStdio || !fields.oauth ? null : clientSecret,
    folder: isStdio ? fields.folder.trim() : '',
    forgetClientSecret: fields.forgetClientSecret,
    images: fields.images,
    instructions: fields.instructions,
    name: fields.name.trim(),
    oauth: !isStdio && fields.oauth,
    plain,
    prefix: fields.prefix.trim(),
    program: isStdio ? fields.program.trim() : '',
    secrets,
    timeoutSeconds: fields.timeoutSeconds,
    transport: fields.transport,
    url: isStdio ? '' : fields.url.trim(),
  };
}
