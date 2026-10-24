//! The plugin's commands and channel (plugins/notebooks/docs/README.md,
//! contracts), as specs of the plugin SDK (ADR-0034, decision 8), so the host
//! checks their input before the plugin's handlers run and answers
//! `plugin_off` while the plugin is off.

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The longest name of a notebook, in characters.
pub const MAX_NAME_CHARACTERS: usize = 200;

/// The longest title of a pasted text, in characters.
pub const MAX_TITLE_CHARACTERS: usize = 200;

/// The longest name of a dropped file that the plugin takes, in bytes.
pub const MAX_FILE_NAME_BYTES: usize = 1024;

/// The longest name of the dialog's filter, in characters.
pub const MAX_FILTER_NAME_CHARACTERS: usize = 100;

/// The most sections that one read of a source's text returns.
pub const MAX_SECTIONS_PER_READ: u32 = 64;

/// The extensions of the files that the dialog shows.
pub const EXTENSIONS: &[&str] = &["pdf", "md", "markdown", "txt"];

/// The bytes of one part of a dropped file (plugin ADR-0002, decision 3).
pub const UPLOAD_PART_BYTES: usize = 4 * 1024 * 1024;

/// The longest base64 text of one part of a dropped file.
pub(crate) fn max_part_base64_len() -> usize {
    UPLOAD_PART_BYTES.div_ceil(3) * 4
}

/// A limit as a number for the UI's message.
pub(crate) fn limit(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Whether a short text that the owner wrote is not blank, has at most
/// `max_characters` characters and no control characters.
fn is_short_text(text: &str, max_characters: usize) -> bool {
    let trimmed = text.trim();
    let is_blank = trimmed.is_empty();
    let is_long = trimmed.chars().count() > max_characters;
    let has_control = text.chars().any(char::is_control);
    !is_blank && !is_long && !has_control
}

/// The argument of `create_notebook`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateNotebook {
    /// The notebook's name.
    pub name: String,
}

impl Validate for CreateNotebook {
    /// The name is not blank, has at most [`MAX_NAME_CHARACTERS`] characters
    /// and no control characters.
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_short_text(&self.name, MAX_NAME_CHARACTERS) {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// The argument of `rename_notebook`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameNotebook {
    /// The notebook.
    pub notebook_id: String,
    /// Its new name.
    pub name: String,
}

impl Validate for RenameNotebook {
    /// The name follows the rules of [`CreateNotebook`].
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_short_text(&self.name, MAX_NAME_CHARACTERS) {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// The argument of the commands that act on one notebook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotebookRef {
    /// The notebook.
    pub notebook_id: String,
}

impl Validate for NotebookRef {
    /// Any ID decodes: one that the plugin does not hold is
    /// `notebook_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `pick_sources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PickSources {
    /// The notebook that takes the files.
    pub notebook_id: String,
    /// The name of the dialog's filter, such as "Documents", from the UI's
    /// catalog (ADR-0019).
    pub filter_name: String,
}

impl Validate for PickSources {
    /// The filter's name, which the system's dialog shows, is short plain
    /// text.
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_short_text(&self.filter_name, MAX_FILTER_NAME_CHARACTERS) {
            return Err(InputRefusal::new("filterName"));
        }
        Ok(())
    }
}

/// The argument of `start_upload`: a file that the owner dropped on a
/// notebook's page, whose parts follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartUpload {
    /// The notebook that takes the file.
    pub notebook_id: String,
    /// The file's name, as the webview knows it.
    pub name: String,
}

impl Validate for StartUpload {
    /// The name is not blank and has at most [`MAX_FILE_NAME_BYTES`] bytes.
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.name.trim().is_empty() || self.name.len() > MAX_FILE_NAME_BYTES {
            return Err(InputRefusal::new("name"));
        }
        Ok(())
    }
}

/// The result of `start_upload`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UploadStarted {
    /// The upload, which its parts name.
    pub upload_id: String,
}

/// The argument of `add_upload_part`: the next part of a dropped file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddUploadPart {
    /// The upload.
    pub upload_id: String,
    /// The part's number, from 0, which follows the last one that arrived.
    pub index: u32,
    /// The part's bytes, as base64.
    pub data: String,
}

impl Validate for AddUploadPart {
    /// The data is the base64 of at most [`UPLOAD_PART_BYTES`] bytes.
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.data.len() > max_part_base64_len() {
            return Err(InputRefusal::new("data"));
        }
        Ok(())
    }
}

/// The argument of the commands that end an upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadRef {
    /// The upload.
    pub upload_id: String,
}

impl Validate for UploadRef {
    /// Any ID decodes: one that the plugin does not hold is
    /// `upload_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `add_text_source`: a text that the owner wrote or pasted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddTextSource {
    /// The notebook that takes the text.
    pub notebook_id: String,
    /// The text's title.
    pub title: String,
    /// The text, of any length (plugin ADR-0002).
    pub text: String,
}

impl Validate for AddTextSource {
    /// The title is short plain text, and the text is not blank.
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_short_text(&self.title, MAX_TITLE_CHARACTERS) {
            return Err(InputRefusal::new("title"));
        }
        if self.text.trim().is_empty() {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

/// The argument of the commands that act on one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRef {
    /// The source.
    pub source_id: String,
}

impl Validate for SourceRef {
    /// Any ID decodes: one that the plugin does not hold is
    /// `source_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `set_source_included`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSourceIncluded {
    /// The source.
    pub source_id: String,
    /// Whether its passages are searched.
    pub included: bool,
}

impl Validate for SetSourceIncluded {
    /// Any ID decodes.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `read_source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadSource {
    /// The source.
    pub source_id: String,
    /// The first section to read, from 0.
    pub first_section: u32,
}

impl Validate for ReadSource {
    /// Any ID and position decode: a position past the end reads nothing.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// The argument of `locate_passage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocatePassage {
    /// The passage's key, as the context of a conversation holds it:
    /// `<source ID>:<passage number>`.
    pub key: String,
}

impl Validate for LocatePassage {
    /// Any key decodes: one that names no passage is `passage_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

/// How the plugin searches (plugins/notebooks/docs/README.md, search), as
/// the owner sets it. The reranker orders passages and drops none
/// (ADR-0049), so there is no threshold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    /// The search profile of the embedding model, or `None` before the owner
    /// chose one.
    pub embedding_profile_id: Option<String>,
    /// The search profile of the reranker, or `None` to search without one.
    pub rerank_profile_id: Option<String>,
    /// The most passages that one message gets.
    pub max_passages: u32,
}

/// The lowest and highest number of passages that a message may get.
pub const PASSAGE_RANGE: (u32, u32) = (1, 12);

/// The passages that a message gets unless the owner chose another number,
/// as many as the earlier harness sent.
pub const DEFAULT_PASSAGES: u32 = 8;

impl Default for Preferences {
    fn default() -> Self {
        Self {
            embedding_profile_id: None,
            rerank_profile_id: None,
            max_passages: DEFAULT_PASSAGES,
        }
    }
}

impl Validate for Preferences {
    /// The passages are within [`PASSAGE_RANGE`]. A profile that does not
    /// exist is the plugin's own check, `profile_not_found`.
    fn validate(&self) -> Result<(), InputRefusal> {
        let (fewest, most) = PASSAGE_RANGE;
        if !(fewest..=most).contains(&self.max_passages) {
            return Err(InputRefusal::new("maxPassages"));
        }
        Ok(())
    }
}

/// What a source is, as the UI words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A PDF.
    Pdf,
    /// A Markdown file.
    Markdown,
    /// A text file or a pasted text.
    Text,
}

/// Where a source is in its life (plugins/notebooks/docs/README.md,
/// sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// Its text is being read.
    Reading,
    /// It waits for an embedding model.
    Waiting,
    /// Its passages are being embedded.
    Indexing,
    /// It is searched.
    Ready,
    /// It failed, with the reason.
    Failed,
}

/// A source as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// The source.
    pub id: String,
    /// Its notebook.
    pub notebook_id: String,
    /// What it is.
    pub kind: SourceKind,
    /// Its name, such as its file's name. It is content.
    pub name: String,
    /// The size of its file, or of its text, in bytes. A size beyond
    /// `u32::MAX`, which the bindings cannot carry as a number, shows as
    /// `u32::MAX`.
    pub bytes: u32,
    /// How many pages a PDF has.
    pub pages: Option<u32>,
    /// Where it is in its life.
    pub state: SourceState,
    /// Why it failed, or why it waits, as the plugin's code.
    pub failure: Option<String>,
    /// Whether its passages are searched.
    pub included: bool,
    /// How many passages it has.
    pub passages: u32,
    /// How many of them are embedded with the current model.
    pub embedded: u32,
    /// When it was added.
    pub added_at: UtcMillis,
    /// When its indexing ended.
    pub indexed_at: Option<UtcMillis>,
}

/// A notebook as the UI shows it, with its sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotebookView {
    /// The notebook.
    pub id: String,
    /// Its name. It is content.
    pub name: String,
    /// Its sources, in the order they were added.
    pub sources: Vec<SourceView>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it last changed.
    pub updated_at: UtcMillis,
}

/// Every notebook, the newest first, as the channel `follow_notebooks` sends
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotebookList {
    /// The notebooks.
    pub notebooks: Vec<NotebookView>,
}

/// Why the plugin refused a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SourceRefusal {
    /// The notebook holds the file already, byte for byte.
    Duplicate,
    /// The file is not a PDF, a Markdown or a text file.
    UnsupportedType,
    /// The file could not be read.
    Unreadable,
}

/// A file that the plugin refused, with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RefusedFile {
    /// The file's name.
    pub name: String,
    /// Why.
    pub reason: SourceRefusal,
}

/// The result of `pick_sources`: the sources that the files became, and the
/// files that the plugin refused. Both are empty when the owner closed the
/// dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddedSources {
    /// The new sources, in the order of the files.
    pub added: Vec<SourceView>,
    /// The refused files, in their order.
    pub refused: Vec<RefusedFile>,
}

/// Where a section stands in its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SectionPlace {
    /// A page of a PDF, from 1.
    Page {
        /// The page's number.
        number: u32,
    },
    /// A section of a Markdown file, by the path of its headings, which is
    /// empty before the first heading.
    Headings {
        /// The headings, the outermost first. They are content.
        path: Vec<String>,
    },
    /// The whole of a text.
    Whole,
}

/// A page or section of a source's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SectionText {
    /// Its position among the source's sections, from 0.
    pub position: u32,
    /// Where it stands.
    pub place: SectionPlace,
    /// Its text. It is content.
    pub text: String,
}

/// The result of `read_source`: sections of a source's text, from the first
/// asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceText {
    /// The source.
    pub source: SourceView,
    /// At most [`MAX_SECTIONS_PER_READ`] sections.
    pub sections: Vec<SectionText>,
    /// How many sections the source has.
    pub total: u32,
}

/// The result of `locate_passage`: where a passage stands in its source's
/// text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PassagePlace {
    /// The source.
    pub source_id: String,
    /// The section that holds the passage.
    pub section: u32,
    /// The UTF-16 offset of the passage's first character in the section's
    /// text, as the webview counts it.
    pub start: u32,
    /// The UTF-16 offset after its last character.
    pub end: u32,
}

/// A search profile that the settings offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProfileOption {
    /// The profile's ID.
    pub id: String,
    /// Its name, as the owner gave it. It is content.
    pub name: String,
    /// Its model's name on its server. It is content.
    pub model: String,
}

/// The result of `list_search_profiles`: the owner's search profiles of
/// each kind, the oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchProfiles {
    /// The embedding profiles.
    pub embeddings: Vec<ProfileOption>,
    /// The rerank profiles.
    pub rerank: Vec<ProfileOption>,
}

/// Why a command or channel of the plugin failed. The host's refusals, such
/// as `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum NotebooksError {
    /// The notebook does not exist, for example because it was deleted.
    #[error("the notebook does not exist")]
    NotebookNotFound,
    /// The source does not exist.
    #[error("the source does not exist")]
    SourceNotFound,
    /// Another notebook has the name, without regard to case.
    #[error("another notebook has the name")]
    NameTaken,
    /// The upload does not exist: it finished, it was cancelled, or it got
    /// no part for 5 minutes.
    #[error("the upload does not exist")]
    UploadNotFound,
    /// A part of an upload came out of order, which ends the upload.
    #[error("the upload expected part {expected}")]
    UploadPartOutOfOrder {
        /// The number of the part that it expected.
        expected: u32,
    },
    /// A file was refused.
    #[error("the file was refused: {reason:?}")]
    Refused {
        /// Why.
        reason: SourceRefusal,
    },
    /// A search profile that the preferences name does not exist, or is of
    /// another kind.
    #[error("the search profile does not exist")]
    ProfileNotFound,
    /// The passage does not exist, for example because its source was
    /// removed.
    #[error("the passage does not exist")]
    PassageNotFound,
    /// Only a source that failed while it was indexed can be tried again.
    #[error("the source cannot be tried again")]
    NothingToRetry,
    /// The system's file dialog failed.
    #[error("the file dialog failed")]
    DialogFailed,
    /// The search profiles could not be read.
    #[error("the search profiles could not be read")]
    ProfilesUnavailable,
    /// The store did not keep or read what the plugin asked.
    #[error("the notebooks could not be read or saved")]
    StorageFailed,
}

impl PluginError for NotebooksError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::NotebookNotFound => "notebooks.notebook_not_found",
            Self::SourceNotFound => "notebooks.source_not_found",
            Self::NameTaken => "notebooks.name_taken",
            Self::UploadNotFound => "notebooks.upload_not_found",
            Self::UploadPartOutOfOrder { .. } => "notebooks.upload_part_out_of_order",
            Self::Refused { .. } => "notebooks.refused",
            Self::ProfileNotFound => "notebooks.profile_not_found",
            Self::PassageNotFound => "notebooks.passage_not_found",
            Self::NothingToRetry => "notebooks.nothing_to_retry",
            Self::DialogFailed => "notebooks.dialog_failed",
            Self::ProfilesUnavailable => "notebooks.profiles_unavailable",
            Self::StorageFailed => "notebooks.storage_failed",
        }
    }
}

/// Declares a command of the plugin.
macro_rules! command {
    ($spec:ident, $name:literal, $docs:literal, $input:ty, $output:ty) => {
        #[doc = concat!("`", $name, "`: ", $docs)]
        #[derive(Debug)]
        pub enum $spec {}

        impl CommandSpec for $spec {
            const NAME: &'static str = $name;
            const DOCS: &'static str = $docs;
            type Input = $input;
            type Output = $output;
            type Error = NotebooksError;
        }
    };
}

command!(
    CreateNotebookCommand,
    "create_notebook",
    "Creates a notebook with a name that no other notebook has.",
    CreateNotebook,
    NotebookView
);
command!(
    RenameNotebookCommand,
    "rename_notebook",
    "Gives a notebook a name that no other notebook has.",
    RenameNotebook,
    NotebookView
);
command!(
    DeleteNotebookCommand,
    "delete_notebook",
    "Deletes a notebook with its sources and their text.",
    NotebookRef,
    ()
);
command!(
    PickSourcesCommand,
    "pick_sources",
    "Shows the system's file dialog for documents and adds the chosen files to a notebook, which the plugin then reads and indexes.",
    PickSources,
    AddedSources
);
command!(
    StartUploadCommand,
    "start_upload",
    "Starts the upload of a file that the owner dropped on a notebook's page, whose parts follow.",
    StartUpload,
    UploadStarted
);
command!(
    FinishUploadCommand,
    "finish_upload",
    "Adds an uploaded file to its notebook, which the plugin then reads and indexes.",
    UploadRef,
    SourceView
);
command!(
    CancelUploadCommand,
    "cancel_upload",
    "Drops an upload and the parts that arrived.",
    UploadRef,
    ()
);
command!(
    RemoveSourceCommand,
    "remove_source",
    "Removes a source with its text and passages.",
    SourceRef,
    ()
);
command!(
    RetrySourceCommand,
    "retry_source",
    "Indexes again a source that failed while it was indexed.",
    SourceRef,
    SourceView
);
command!(
    SetSourceIncludedCommand,
    "set_source_included",
    "Switches whether a source's passages are searched.",
    SetSourceIncluded,
    SourceView
);
command!(
    ReadSourceCommand,
    "read_source",
    "Reads sections of a source's text, from the first asked for.",
    ReadSource,
    SourceText
);
command!(
    LocatePassageCommand,
    "locate_passage",
    "Finds where a passage stands in its source's text.",
    LocatePassage,
    PassagePlace
);
command!(
    ReadPreferencesCommand,
    "read_preferences",
    "Reads how the plugin searches.",
    (),
    Preferences
);
command!(
    SavePreferencesCommand,
    "save_preferences",
    "Saves how the plugin searches; a new embedding model indexes every source again.",
    Preferences,
    Preferences
);
command!(
    ListSearchProfilesCommand,
    "list_search_profiles",
    "Lists the owner's embedding and rerank search profiles.",
    (),
    SearchProfiles
);

/// `add_upload_part`: the next part of a dropped file.
#[derive(Debug)]
pub enum AddUploadPartCommand {}

impl CommandSpec for AddUploadPartCommand {
    const NAME: &'static str = "add_upload_part";
    const DOCS: &'static str = "Adds the next part of an uploaded file, in order.";
    /// The base64 text of one part, with room for the upload's ID and the
    /// part's number.
    const MAX_INPUT_BYTES: u32 = 6 * 1024 * 1024;
    type Input = AddUploadPart;
    type Output = ();
    type Error = NotebooksError;
}

/// `add_text_source`: a text that the owner wrote or pasted.
#[derive(Debug)]
pub enum AddTextSourceCommand {}

impl CommandSpec for AddTextSourceCommand {
    const NAME: &'static str = "add_text_source";
    const DOCS: &'static str = "Adds a text that the owner wrote or pasted to a notebook.";
    /// A text of any length (plugin ADR-0002), as far as one message of the
    /// webview carries it.
    const MAX_INPUT_BYTES: u32 = u32::MAX;
    type Input = AddTextSource;
    type Output = SourceView;
    type Error = NotebooksError;
}

/// `follow_notebooks`: every notebook with its sources, then again after
/// each change.
#[derive(Debug)]
pub enum FollowNotebooksChannel {}

impl ChannelSpec for FollowNotebooksChannel {
    const NAME: &'static str = "follow_notebooks";
    const DOCS: &'static str = "Every notebook with its sources and their states, then again after each change, until the plugin stops.";
    type Input = ();
    type Item = NotebookList;
    type Error = NotebooksError;
}

/// The plugin's commands, its channel and the limits that the UI checks.
pub fn interface() -> Interface {
    Interface::new()
        .command::<CreateNotebookCommand>()
        .command::<RenameNotebookCommand>()
        .command::<DeleteNotebookCommand>()
        .command::<PickSourcesCommand>()
        .command::<StartUploadCommand>()
        .command::<AddUploadPartCommand>()
        .command::<FinishUploadCommand>()
        .command::<CancelUploadCommand>()
        .command::<AddTextSourceCommand>()
        .command::<RemoveSourceCommand>()
        .command::<RetrySourceCommand>()
        .command::<SetSourceIncludedCommand>()
        .command::<ReadSourceCommand>()
        .command::<LocatePassageCommand>()
        .command::<ReadPreferencesCommand>()
        .command::<SavePreferencesCommand>()
        .command::<ListSearchProfilesCommand>()
        .channel::<FollowNotebooksChannel>()
        .constant(
            "MAX_NAME_CHARACTERS",
            "The longest name of a notebook, in characters.",
            limit(MAX_NAME_CHARACTERS),
        )
        .constant(
            "MAX_TITLE_CHARACTERS",
            "The longest title of a pasted text, in characters.",
            limit(MAX_TITLE_CHARACTERS),
        )
        .constant(
            "UPLOAD_PART_BYTES",
            "The bytes of one part of a dropped file.",
            limit(UPLOAD_PART_BYTES),
        )
        .constant(
            "MAX_SECTIONS_PER_READ",
            "The most sections of a source's text that one read returns.",
            MAX_SECTIONS_PER_READ,
        )
        .constant(
            "FEWEST_PASSAGES",
            "The fewest passages that the preferences let one message get.",
            PASSAGE_RANGE.0,
        )
        .constant(
            "MOST_PASSAGES",
            "The most passages that the preferences let one message get.",
            PASSAGE_RANGE.1,
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::{CommandSpec, InputRefusal, PluginError, Validate};

    use super::{
        AddTextSource, AddUploadPart, AddUploadPartCommand, CreateNotebook, MAX_FILE_NAME_BYTES,
        MAX_NAME_CHARACTERS, NotebooksError, Preferences, StartUpload, max_part_base64_len,
    };

    #[test]
    fn a_name_is_short_plain_text() {
        let named = |name: &str| CreateNotebook {
            name: name.to_owned(),
        };

        assert_eq!(named(" Physics ").validate(), Ok(()));
        assert_eq!(named(&"é".repeat(MAX_NAME_CHARACTERS)).validate(), Ok(()));
        for refused in [
            String::new(),
            "  ".to_owned(),
            "a\nb".to_owned(),
            "x".repeat(MAX_NAME_CHARACTERS + 1),
        ] {
            assert_eq!(
                named(&refused).validate(),
                Err(InputRefusal::new("name")),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_pasted_text_has_a_title_and_text_of_any_length() {
        let text = |title: &str, text: String| AddTextSource {
            notebook_id: "n1".to_owned(),
            title: title.to_owned(),
            text,
        };

        assert_eq!(text("Notes", "Hello".to_owned()).validate(), Ok(()));
        assert_eq!(
            text("Notes", "a".repeat(2 * 1024 * 1024)).validate(),
            Ok(())
        );
        assert_eq!(
            text("", "Hello".to_owned()).validate(),
            Err(InputRefusal::new("title"))
        );
        assert_eq!(
            text("Notes", " \n".to_owned()).validate(),
            Err(InputRefusal::new("text"))
        );
    }

    #[test]
    fn an_upload_s_name_is_short_and_each_part_fits_the_host_s_limit() {
        let named = |name: String| StartUpload {
            notebook_id: "n1".to_owned(),
            name,
        };
        let part = |data: String| AddUploadPart {
            upload_id: "u1".to_owned(),
            index: 0,
            data,
        };

        assert_eq!(named("a.pdf".to_owned()).validate(), Ok(()));
        assert_eq!(
            named("x".repeat(MAX_FILE_NAME_BYTES + 1)).validate(),
            Err(InputRefusal::new("name"))
        );
        assert_eq!(part("A".repeat(max_part_base64_len())).validate(), Ok(()));
        assert_eq!(
            part("A".repeat(max_part_base64_len() + 4)).validate(),
            Err(InputRefusal::new("data"))
        );
        let largest_input = max_part_base64_len() + 256;
        assert!(largest_input < usize::try_from(AddUploadPartCommand::MAX_INPUT_BYTES).unwrap());
    }

    #[test]
    fn preferences_keep_a_number_of_passages_in_range() {
        let preferences = |max_passages: u32| Preferences {
            max_passages,
            ..Preferences::default()
        };

        assert_eq!(Preferences::default().validate(), Ok(()));
        assert_eq!(Preferences::default().max_passages, 8);
        for passages in [0, 13] {
            assert_eq!(
                preferences(passages).validate(),
                Err(InputRefusal::new("maxPassages"))
            );
        }
    }

    #[test]
    fn errors_serialize_with_their_code_and_fields() {
        assert_eq!(
            serde_json::to_value(NotebooksError::UploadPartOutOfOrder { expected: 3 }).unwrap(),
            serde_json::json!({ "code": "upload_part_out_of_order", "expected": 3 })
        );
        assert_eq!(
            serde_json::to_value(NotebooksError::Refused {
                reason: super::SourceRefusal::Duplicate
            })
            .unwrap(),
            serde_json::json!({ "code": "refused", "reason": "duplicate" })
        );
        assert_eq!(
            NotebooksError::NotebookNotFound.code(),
            "notebooks.notebook_not_found"
        );
    }
}
