//! The entries of a folder at a revision, as `git ls-tree -z` writes them
//! with [`TREE_FORMAT`]: each entry's kind, size and name, ended by NUL, so
//! no name is quoted.

use serde::Serialize;

/// The format of [`parse_tree`]: the object's type, its size, which a
/// folder or a submodule has none of, and the entry's name.
pub(crate) const TREE_FORMAT: &str = "--format=%(objecttype)%x1f%(objectsize)%x1f%(path)";

/// What an entry of a folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TreeEntryKind {
    File,
    Folder,
    Submodule,
}

/// An entry of a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TreeEntry {
    pub(crate) kind: TreeEntryKind,
    pub(crate) name: String,
    /// A file's size in bytes.
    pub(crate) size: Option<u64>,
}

/// Reads the output of `git ls-tree -z` with [`TREE_FORMAT`]. An entry of a
/// type that this version does not know is skipped.
pub(crate) fn parse_tree(output: &[u8]) -> Vec<TreeEntry> {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let mut fields = record.splitn(3, '\u{1f}');
            let (Some(kind), Some(size), Some(name)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return None;
            };
            let kind = match kind {
                "blob" => TreeEntryKind::File,
                "tree" => TreeEntryKind::Folder,
                "commit" => TreeEntryKind::Submodule,
                _other => return None,
            };
            Some(TreeEntry {
                kind,
                name: name.to_owned(),
                size: size.trim().parse().ok(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TreeEntry, TreeEntryKind, parse_tree};

    #[test]
    fn files_folders_and_submodules_are_read_with_the_sizes_of_files() {
        let output = b"blob\x1f1234\x1fmain file.rs\0tree\x1f-\x1fparse\0commit\x1f-\x1fvendor\0tag\x1f-\x1fodd\0";

        let entries = parse_tree(output);

        assert_eq!(
            entries,
            [
                TreeEntry {
                    kind: TreeEntryKind::File,
                    name: "main file.rs".to_owned(),
                    size: Some(1234),
                },
                TreeEntry {
                    kind: TreeEntryKind::Folder,
                    name: "parse".to_owned(),
                    size: None,
                },
                TreeEntry {
                    kind: TreeEntryKind::Submodule,
                    name: "vendor".to_owned(),
                    size: None,
                },
            ]
        );
    }
}
