// The files of a paste or a drop on the message box (ADR-0033, decision 7):
// a paste that carries text pastes the text, so that it never becomes an
// image, and each file goes to the first image source that takes it.

/** The parts of a `DataTransfer` that these rules read. */
export interface Transfer {
  readonly files: ArrayLike<File>;
  readonly getData: (format: string) => string;
  readonly types: readonly string[];
}

/** A source as these rules see it. */
export interface TakingSource {
  readonly id: string;
  readonly takes: (file: File) => boolean;
}

/** The files of a paste, or none when it carries text, which pastes as text. */
export function pastedFiles(transfer: Transfer): File[] {
  if (transfer.getData('text/plain') !== '') {
    return [];
  }
  return Array.from(transfer.files);
}

/** Whether a drag carries files, as opposed to text or a link. */
export function carriesFiles(transfer: Pick<Transfer, 'types'> | null): boolean {
  return transfer?.types.includes('Files') === true;
}

/** Which source takes each file, in the order of the sources, and the files that none takes. */
export interface AssignedFiles<S> {
  readonly assigned: ReadonlyMap<S, File[]>;
  readonly refused: readonly File[];
}

/** Gives each file to the first source that takes it. */
export function assignFiles<S extends TakingSource>(files: readonly File[], sources: readonly S[]): AssignedFiles<S> {
  const assigned = new Map<S, File[]>();
  const refused: File[] = [];
  for (const file of files) {
    const source = sources.find((candidate) => candidate.takes(file));
    if (source === undefined) {
      refused.push(file);
      continue;
    }
    const taken = assigned.get(source) ?? [];
    taken.push(file);
    assigned.set(source, taken);
  }
  return { assigned, refused };
}
