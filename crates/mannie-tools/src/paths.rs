//! The paths that tools receive (docs/features/system-tools/
//! tools-and-results.md, paths).
//!
//! A path goes through three steps. [`check_text`] refuses paths that Mannie
//! never accesses, before anything touches the file system, so a refused path
//! never reaches a network host (T-17). [`locate`] makes the path absolute
//! against the workspace and removes `.` and `..` lexically. [`resolve`] then
//! walks the path one component at a time and resolves its links itself, so
//! every link target passes the same refusals before Mannie follows it.

use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};

use mannie_types::tools::{PathRefusal, ToolFailure};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// The longest path Mannie accepts, in characters: the longest path that
/// Windows handles.
const MAX_PATH_CHARACTERS: usize = 32_767;

/// The most links that one resolution follows, as Linux's `MAXSYMLINKS`.
const MAX_LINKS: u32 = 40;

/// Refuses a path that Mannie never accesses. The check reads only the text.
pub(crate) fn check_text(text: &str) -> Result<(), PathRefusal> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err(PathRefusal::Empty);
    }
    if text.chars().count() > MAX_PATH_CHARACTERS {
        return Err(PathRefusal::TooLong);
    }
    platform::check_text(text)
}

/// The absolute form of a path that a tool received: an absolute path as it
/// is, a relative one below the workspace. `.` and `..` are removed lexically,
/// before any link is resolved, so `link/..` names the folder that holds the
/// link on every platform.
pub(crate) fn locate(workspace: Option<&Path>, text: &str) -> Result<PathBuf, ToolFailure> {
    check_text(text).map_err(|reason| ToolFailure::PathRefused { reason })?;
    let path = Path::new(text);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.ok_or(ToolFailure::NoWorkspace)?.join(path)
    };
    Ok(normalize(&absolute))
}

/// Removes `.` and `..` components lexically. `..` never goes above the root.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normal.push(component);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
        }
    }
    normal
}

/// Whether [`resolve`] follows a link at the end of the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinalLink {
    /// Follow it, as a read does.
    Follow,
    /// Keep it, as `file_info` and a delete do: the link itself is the target.
    Keep,
}

/// What [`resolve`] found.
#[derive(Debug)]
pub(crate) struct Resolved {
    /// The path in canonical form, with every link it went through resolved.
    pub(crate) path: PathBuf,
    /// Whether the resolution followed a link.
    pub(crate) through_link: bool,
    /// What is at the path, without following a final link, or `None` when
    /// nothing is.
    pub(crate) metadata: Option<Metadata>,
}

impl Resolved {
    /// Whether something exists at the path.
    pub(crate) fn exists(&self) -> bool {
        self.metadata.is_some()
    }

    /// Whether the path is a folder.
    pub(crate) fn is_folder(&self) -> bool {
        self.metadata.as_ref().is_some_and(Metadata::is_dir)
    }

    /// Whether the path is a link that was kept.
    pub(crate) fn is_link(&self) -> bool {
        self.metadata.as_ref().is_some_and(is_link)
    }

    /// The canonical path as text.
    pub(crate) fn text(&self) -> Result<String, ToolFailure> {
        path_text(&self.path)
    }
}

/// One step of a walk.
#[derive(Debug)]
enum Step {
    /// Start again at a root: `/`, or a drive such as `C:\`.
    Root(PathBuf),
    /// Go up to the parent folder.
    Parent,
    /// Go into a name.
    Name(OsString),
}

/// The steps of a path, in order.
fn steps(path: &Path) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut root = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component),
            Component::CurDir => {}
            Component::ParentDir => steps.push(Step::Parent),
            Component::Normal(name) => steps.push(Step::Name(name.to_owned())),
        }
    }
    // A prefix and a root directory always come first.
    if !root.as_os_str().is_empty() {
        steps.insert(0, Step::Root(root));
    }
    steps
}

/// Walks an absolute path one component at a time and resolves its links.
///
/// Mannie reads each link's target itself and checks it like a path the model
/// wrote before it follows it, so a link cannot lead a resolution to a network
/// host. A link that ends the path is followed or kept as `final_link` says.
/// The part of the path that exists comes out in canonical form; the rest is
/// kept as written.
pub(crate) fn resolve(path: &Path, final_link: FinalLink) -> Result<Resolved, ToolFailure> {
    let mut pending = steps(path);
    pending.reverse();
    let mut current = PathBuf::new();
    // How many components at the end of `current` do not exist.
    let mut missing = 0_usize;
    let mut links = 0_u32;
    let mut through_link = false;
    while let Some(step) = pending.pop() {
        match step {
            Step::Root(root) => {
                current = root;
                missing = 0;
            }
            Step::Parent => {
                current.pop();
                missing = missing.saturating_sub(1);
            }
            Step::Name(name) => {
                let candidate = current.join(&name);
                if missing > 0 {
                    current = candidate;
                    missing += 1;
                    continue;
                }
                let keep_link = pending.is_empty() && final_link == FinalLink::Keep;
                match fs::symlink_metadata(&candidate) {
                    Ok(metadata) if is_link(&metadata) && !keep_link => {
                        links += 1;
                        if links > MAX_LINKS {
                            return Err(ToolFailure::IoError {
                                kind: "the path goes through too many links".to_owned(),
                            });
                        }
                        through_link = true;
                        let target = fs::read_link(&candidate)
                            .map_err(|error| io_failure(&error, &candidate))?;
                        let target = platform::link_target(&target)
                            .map_err(|reason| ToolFailure::PathRefused { reason })?;
                        // The target's steps come next. A relative target goes
                        // on from the link's folder, which `current` still is.
                        let mut target_steps = steps(&target);
                        target_steps.reverse();
                        pending.extend(target_steps);
                    }
                    Ok(_) => current = candidate,
                    Err(error) if is_missing(&error) => {
                        current = candidate;
                        missing = 1;
                    }
                    Err(error) => return Err(io_failure(&error, &candidate)),
                }
            }
        }
    }
    let metadata = if missing == 0 {
        Some(fs::symlink_metadata(&current).map_err(|error| io_failure(&error, &current))?)
    } else {
        None
    };
    let keeps_link = metadata.as_ref().is_some_and(is_link);
    let path = platform::canonical(&current, missing, keeps_link)?;
    Ok(Resolved {
        path,
        through_link,
        metadata,
    })
}

/// Whether metadata from `symlink_metadata` describes a link. On Windows only
/// symbolic links and junctions count, so cloud files such as `OneDrive`
/// placeholders are ordinary files.
pub(crate) fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Whether an error says that a path does not exist.
fn is_missing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

/// A path for a read: its links are resolved, so the policy sees where the
/// read goes.
pub(crate) fn for_read(workspace: Option<&Path>, text: &str) -> Result<Resolved, ToolFailure> {
    resolve(&locate(workspace, text)?, FinalLink::Follow)
}

/// A path for `file_info`: links on the way are resolved, and a final link is
/// described instead of followed.
pub(crate) fn for_info(workspace: Option<&Path>, text: &str) -> Result<Resolved, ToolFailure> {
    resolve(&locate(workspace, text)?, FinalLink::Keep)
}

/// A path for a change. A change never goes through a link: a path that does
/// is refused with its resolved form, so the model can use that path instead.
/// With [`FinalLink::Keep`], the path may end in a link, which the change
/// then acts on itself, as a delete or a move does.
pub(crate) fn for_change(
    workspace: Option<&Path>,
    text: &str,
    final_link: FinalLink,
) -> Result<Resolved, ToolFailure> {
    let located = locate(workspace, text)?;
    let resolved = resolve(&located, FinalLink::Keep)?;
    let ends_in_link = resolved.is_link() && final_link == FinalLink::Follow;
    if resolved.through_link || ends_in_link {
        return Err(through_link(&located));
    }
    Ok(resolved)
}

/// The failure of a change through a link, which names the resolved path.
fn through_link(located: &Path) -> ToolFailure {
    match resolve(located, FinalLink::Follow).and_then(|resolved| resolved.text()) {
        Ok(resolved) => ToolFailure::PathThroughLink { resolved },
        Err(failure) => failure,
    }
}

/// A path as text. Accesses and results carry text, so a path that is not
/// valid Unicode cannot be authorized and is refused.
pub(crate) fn path_text(path: &Path) -> Result<String, ToolFailure> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ToolFailure::IoError {
            kind: "the path is not valid Unicode".to_owned(),
        })
}

/// The failure for an error of the operating system at `path`.
pub(crate) fn io_failure(error: &io::Error, path: &Path) -> ToolFailure {
    let path = path.display().to_string();
    if platform::is_sharing_violation(error) {
        return ToolFailure::InUse { path };
    }
    match error.kind() {
        io::ErrorKind::NotFound => ToolFailure::NotFound { path },
        io::ErrorKind::PermissionDenied => ToolFailure::PermissionDenied { path },
        io::ErrorKind::AlreadyExists => ToolFailure::AlreadyExists { path },
        io::ErrorKind::NotADirectory => ToolFailure::NotAFolder { path },
        io::ErrorKind::IsADirectory => ToolFailure::NotAFile { path },
        io::ErrorKind::DirectoryNotEmpty => ToolFailure::FolderNotEmpty { path },
        io::ErrorKind::CrossesDevices => ToolFailure::DifferentVolume,
        io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy => {
            ToolFailure::InUse { path }
        }
        _ => ToolFailure::IoError {
            kind: error.to_string(),
        },
    }
}

/// Whether `path` is `folder` or lies below it. Both are canonical. Windows
/// compares names without regard to case, as its file systems do.
pub(crate) fn is_inside(path: &Path, folder: &Path) -> bool {
    let mut path_components = path.components();
    for folder_component in folder.components() {
        let Some(path_component) = path_components.next() else {
            return false;
        };
        if !platform::same_name(path_component.as_os_str(), folder_component.as_os_str()) {
            return false;
        }
    }
    true
}

/// Whether two canonical paths name the same file or folder.
pub(crate) fn is_same_path(left: &Path, right: &Path) -> bool {
    left.components().count() == right.components().count() && is_inside(left, right)
}

/// Why a folder cannot be a workspace.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// The path was refused before any access to the file system.
    #[error("the path is not allowed: {reason:?}")]
    Refused {
        /// Why.
        reason: PathRefusal,
    },
    /// Nothing exists at the path.
    #[error("the folder does not exist")]
    NotFound,
    /// The path is not a folder.
    #[error("the path is not a folder")]
    NotAFolder,
    /// The folder could not be opened or resolved.
    #[error("the folder could not be read: {kind}")]
    Unreadable {
        /// What failed, in English, for logs.
        kind: String,
    },
}

impl FolderError {
    /// The stable code of the error, for logs, which name no path.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Refused { .. } => "folder.refused",
            Self::NotFound => "folder.not_found",
            Self::NotAFolder => "folder.not_a_folder",
            Self::Unreadable { .. } => "folder.unreadable",
        }
    }
}

/// Checks a folder that the owner chose for a project, and returns it in
/// canonical form with its links resolved.
pub fn workspace_folder(text: &str) -> Result<String, FolderError> {
    check_text(text).map_err(|reason| FolderError::Refused { reason })?;
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(FolderError::Refused {
            reason: PathRefusal::NotAbsolute,
        });
    }
    canonical_folder(&normalize(path))
}

/// The canonical form of a folder that exists, with its links resolved, such
/// as Mannie's data folder, which the policy protects.
pub fn canonical_folder(path: &Path) -> Result<String, FolderError> {
    let resolved = resolve(path, FinalLink::Follow).map_err(|failure| match failure {
        ToolFailure::PathRefused { reason } => FolderError::Refused { reason },
        other => FolderError::Unreadable {
            kind: other.to_string(),
        },
    })?;
    match &resolved.metadata {
        None => Err(FolderError::NotFound),
        Some(metadata) if !metadata.is_dir() => Err(FolderError::NotAFolder),
        Some(_) => resolved.text().map_err(|failure| FolderError::Unreadable {
            kind: failure.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests;
