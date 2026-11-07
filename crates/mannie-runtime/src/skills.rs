//! Skills (ADR-0061; docs/features/skills.md): the skills of this computer
//! and of the owner's projects, as the runtime offers them.
//!
//! The composition root builds the [`SkillCatalog`] with the folder of this
//! computer's skills and Mannie's own folders, and hands it to the runtime and
//! to the mention provider [`SkillMentions`]. The catalog keeps the scans of
//! the places for 15 seconds, so a skill that the owner adds appears without a
//! restart, and the owner's switches in memory. For each call it gives the
//! list that agents see, the shelf that `read_skill` reads from and the
//! folders of the skills that are on, which the policy allows reads in. A
//! worker has no catalog.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::points::mentions::{MentionContent, ProviderSuggestions, RenderedMention};
use mannie_store::{RuntimeStoreError, Store, StoredSkillSwitch};
use mannie_tools::{
    FoundSkill, PlaceBounds, PlaceScan, ShelvedSkill, SkillShelf, Verdict, plugin_path_access,
    read_skill_instructions, scan_place, skills_folder_in,
};
use mannie_types::ids::ProjectId;
use mannie_types::labels::clean_label;
use mannie_types::mentions::{MentionEntry, MentionSummary};
use mannie_types::projects::ProjectView;
use mannie_types::skills::{
    SetSkillOn, SkillError, SkillField, SkillPlace, SkillPlaceView, SkillProblemView, SkillView,
    SkillsOverview, is_skill_name,
};
use mannie_types::tools::AccessMode;

use crate::calls::Setup;
use crate::mentions::{MentionContext, MentionProvider};
use crate::{Runtime, lock, texts};

/// The name of the tool that reads skills.
pub const READ_SKILL: &str = "read_skill";

/// The ID of the provider of skills' mentions.
pub const SKILLS: &str = "skills";

/// How long a scan of a place stands for the place.
const SCAN_LIFETIME: Duration = Duration::from_secs(15);

/// The most projects whose scans the catalog keeps, beside this computer's.
const MAX_CACHED_PROJECTS: usize = 4;

/// The most bytes of the list of skills in agents' instructions.
pub(crate) const MAX_LIST_BYTES: usize = 12 * 1024;

/// The most characters of a description in a list that does not fit whole.
const CUT_DESCRIPTION_CHARS: usize = 250;

/// The variant of a skill's mention, which chooses its icon.
const SKILL_VARIANT: &str = "skill";

/// The warning of a project's skill that is off, which "@" still offers.
const OFF_WARNING: &str = "off";

/// The code of a mention that holds the whole skill.
const SUMMARY_WHOLE: &str = "whole";

/// The code of a mention that holds the start of the skill.
const SUMMARY_CUT: &str = "cut";

/// The code of a mention of a skill that the place does not hold.
const SKILL_NOT_FOUND: &str = "skill_not_found";

/// The code of a mention of a skill that could not be read.
const SKILL_UNREADABLE: &str = "skill_unreadable";

/// A place of skills, as the switches and the scans key it.
type PlaceKey = Option<ProjectId>;

/// The owner's choice for one skill.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Switch {
    is_on: bool,
    digest: Option<String>,
}

/// A scan of a place that the catalog keeps.
#[derive(Debug)]
struct Cached {
    place: PlaceKey,
    folder: PathBuf,
    scanned_at: Instant,
    scan: Arc<PlaceScan>,
}

/// The skills of the master (ADR-0061, decision 1).
pub struct SkillCatalog {
    /// The folder of this computer's skills, or `None` without one, such as
    /// in a validation build before a test names one.
    computer_folder: Mutex<Option<PathBuf>>,
    /// Mannie's own folders, in canonical form, which no skill may lie in or
    /// around.
    protected: Vec<PathBuf>,
    /// The owner's switches, by place and name.
    switches: Mutex<HashMap<(PlaceKey, String), Switch>>,
    /// The latest scans.
    cache: Mutex<Vec<Cached>>,
}

impl fmt::Debug for SkillCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkillCatalog")
            .field("computer_folder", &*lock(&self.computer_folder))
            .finish_non_exhaustive()
    }
}

/// A skill with the owner's switch applied.
#[derive(Debug, Clone)]
struct Switched {
    skill: FoundSkill,
    is_on: bool,
    changed: bool,
}

/// The skills of one place, with the owner's switches applied.
#[derive(Debug, Clone)]
struct SwitchedPlace {
    scan: Arc<PlaceScan>,
    skills: Vec<Switched>,
}

/// A skill in agents' list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedSkill {
    pub(crate) name: String,
    pub(crate) description: String,
}

/// A skill that "@" offers in a conversation.
#[derive(Debug, Clone)]
struct Mentionable {
    place: SkillPlace,
    name: String,
    folder: PathBuf,
    is_on: bool,
}

/// What the places of one conversation hold, with the owner's switches
/// applied.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConversationSkills {
    /// The skills that agents' list shows, a project's first.
    pub(crate) listed: Vec<ListedSkill>,
    /// The skills that `read_skill` reads.
    pub(crate) shelf: SkillShelf,
    /// The skills that "@" offers.
    mentionable: Vec<Mentionable>,
}

impl ConversationSkills {
    /// The section `skills` of a call, or `None` when no skill is listed. A
    /// list that does not fit is cut as [`fit_list`] says.
    pub(crate) fn section(&self) -> Option<String> {
        if self.listed.is_empty() {
            return None;
        }
        let (fitted, _left_out) = fit_list(&self.listed);
        Some(texts::skills_section(fitted.iter().map(|skill| {
            (skill.name.as_str(), skill.description.as_str())
        })))
    }
}

/// The bytes that a skill takes in the list.
fn list_bytes(skill: &ListedSkill) -> usize {
    skill.name.len() + skill.description.len() + 5
}

/// The list as it fits in [`MAX_LIST_BYTES`], and how many skills it left
/// out: whole when it fits, else with each description cut to
/// [`CUT_DESCRIPTION_CHARS`], and then without the skills that still do not
/// fit, from the end.
pub(crate) fn fit_list(skills: &[ListedSkill]) -> (Vec<ListedSkill>, usize) {
    let total: usize = skills.iter().map(list_bytes).sum();
    if total <= MAX_LIST_BYTES {
        return (skills.to_vec(), 0);
    }
    let mut fitted = Vec::new();
    let mut used = 0_usize;
    for skill in skills {
        let cut = ListedSkill {
            name: skill.name.clone(),
            description: clean_label(&skill.description, CUT_DESCRIPTION_CHARS).unwrap_or_default(),
        };
        let bytes = list_bytes(&cut);
        if used + bytes > MAX_LIST_BYTES {
            break;
        }
        used += bytes;
        fitted.push(cut);
    }
    let left_out = skills.len() - fitted.len();
    (fitted, left_out)
}

/// Whether a skill is on, and whether a project's skill changed since the
/// owner turned it on: a skill of this computer is on until the owner turns
/// it off, and a project's skill only while the owner's switch is on and its
/// digest is the one that the owner saw (ADR-0061, decision 5).
fn switch_state(place: &PlaceKey, skill: &FoundSkill, switch: Option<&Switch>) -> (bool, bool) {
    match (place, switch) {
        (None, switch) => (switch.is_none_or(|switch| switch.is_on), false),
        (
            Some(_),
            Some(Switch {
                is_on: true,
                digest,
            }),
        ) => {
            let same = digest.as_deref() == Some(skill.digest().as_str());
            (same, !same)
        }
        (Some(_), _) => (false, false),
    }
}

impl SkillCatalog {
    /// A catalog with the folder of this computer's skills, `~/.agents/skills`
    /// or `None`, and Mannie's own folders in canonical form.
    pub fn new(computer_folder: Option<PathBuf>, protected: Vec<PathBuf>) -> Self {
        Self {
            computer_folder: Mutex::new(computer_folder),
            protected,
            switches: Mutex::default(),
            cache: Mutex::default(),
        }
    }

    /// Names the folder of this computer's skills, as a validation build's
    /// test does, and forgets what the catalog scanned.
    pub fn set_computer_folder(&self, folder: Option<PathBuf>) {
        *lock(&self.computer_folder) = folder;
        lock(&self.cache).clear();
    }

    /// Reads the owner's switches.
    pub(crate) async fn load(&self, store: &Store) -> Result<(), RuntimeStoreError> {
        let stored = store.skill_switches().await?;
        let mut switches = lock(&self.switches);
        switches.clear();
        for switch in stored {
            switches.insert(
                (switch.project_id, switch.name),
                Switch {
                    is_on: switch.is_on,
                    digest: switch.digest,
                },
            );
        }
        Ok(())
    }

    /// The folder of a place and the bounds of its scan, or `None` for this
    /// computer without a folder.
    fn place_folder(&self, project: Option<&Path>) -> Option<PathBuf> {
        match project {
            Some(folder) => Some(skills_folder_in(folder)),
            None => lock(&self.computer_folder).clone(),
        }
    }

    /// Scans a place, or takes a scan of it that is younger than
    /// [`SCAN_LIFETIME`] unless `fresh` asks for a new one.
    async fn scan(
        &self,
        place: &PlaceKey,
        workspace: Option<&Path>,
        fresh: bool,
    ) -> Option<Arc<PlaceScan>> {
        let folder = self.place_folder(workspace)?;
        if !fresh {
            let cache = lock(&self.cache);
            let kept = cache.iter().find(|cached| {
                cached.place == *place
                    && cached.folder == folder
                    && cached.scanned_at.elapsed() < SCAN_LIFETIME
            });
            if let Some(cached) = kept {
                return Some(Arc::clone(&cached.scan));
            }
        }
        let protected = self.protected.clone();
        let workspace = workspace.map(Path::to_path_buf);
        let scanned_folder = folder.clone();
        let scan = tokio::task::spawn_blocking(move || {
            scan_place(
                &scanned_folder,
                &PlaceBounds {
                    workspace: workspace.as_deref(),
                    protected: &protected,
                },
            )
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "a scan of skills stopped unexpectedly");
            PlaceScan {
                failure: Some(mannie_types::skills::SkillPlaceFailure::ReadFailed),
                ..PlaceScan::default()
            }
        });
        let scan = Arc::new(scan);
        self.keep(place.clone(), folder, Arc::clone(&scan));
        Some(scan)
    }

    /// Keeps a scan, in place of an older one of the same place, and drops
    /// the oldest project's scan beyond [`MAX_CACHED_PROJECTS`].
    fn keep(&self, place: PlaceKey, folder: PathBuf, scan: Arc<PlaceScan>) {
        let mut cache = lock(&self.cache);
        cache.retain(|cached| cached.place != place);
        cache.push(Cached {
            place,
            folder,
            scanned_at: Instant::now(),
            scan,
        });
        let projects = cache.iter().filter(|cached| cached.place.is_some()).count();
        if projects > MAX_CACHED_PROJECTS
            && let Some(oldest) = cache
                .iter()
                .enumerate()
                .filter(|(_index, cached)| cached.place.is_some())
                .min_by_key(|(_index, cached)| cached.scanned_at)
                .map(|(index, _cached)| index)
        {
            cache.remove(oldest);
        }
    }

    /// A place's skills with the owner's switches applied.
    async fn switched(
        &self,
        place: &PlaceKey,
        workspace: Option<&Path>,
        fresh: bool,
    ) -> Option<SwitchedPlace> {
        let scan = self.scan(place, workspace, fresh).await?;
        let switches = lock(&self.switches);
        let skills = scan
            .skills
            .iter()
            .map(|skill| {
                let switch = switches.get(&(place.clone(), skill.name.clone()));
                let (is_on, changed) = switch_state(place, skill, switch);
                Switched {
                    skill: skill.clone(),
                    is_on,
                    changed,
                }
            })
            .collect();
        Some(SwitchedPlace { scan, skills })
    }

    /// What a conversation's places hold: this computer's, and its project's
    /// when it has one, given by its ID and canonical folder.
    pub(crate) async fn for_conversation(
        &self,
        project: Option<(&ProjectId, &Path)>,
    ) -> ConversationSkills {
        let computer = self.switched(&None, None, false).await;
        let project_place = match project {
            Some((id, folder)) => {
                let place = Some(id.clone());
                self.switched(&place, Some(folder), false).await
            }
            None => None,
        };
        let computer_skills = computer.map(|place| place.skills).unwrap_or_default();
        let project_skills = project_place.map(|place| place.skills).unwrap_or_default();
        conversation_skills(&computer_skills, &project_skills, project.map(|(id, _)| id))
    }

    /// The overview of the settings: this computer's place and every project
    /// that has skills or problems, read fresh.
    pub(crate) async fn overview(&self, projects: &[ProjectView]) -> SkillsOverview {
        let computer = self.switched(&None, None, true).await;
        let computer_folder = lock(&self.computer_folder).clone();
        let computer_skills: Vec<Switched> = computer
            .as_ref()
            .map(|place| place.skills.clone())
            .unwrap_or_default();
        let computer_view = computer.zip(computer_folder).map(|(place, folder)| {
            let listed = conversation_skills(&place.skills, &[], None).listed;
            place_view(
                SkillPlace::Computer,
                None,
                &folder,
                &place,
                fit_list(&listed).1,
            )
        });
        let mut project_views = Vec::new();
        for project in projects {
            let place = Some(project.id.clone());
            let folder = PathBuf::from(&project.folder);
            let Some(switched) = self.switched(&place, Some(&folder), true).await else {
                continue;
            };
            let has_content = !switched.skills.is_empty()
                || !switched.scan.problems.is_empty()
                || switched.scan.failure.is_some();
            if !has_content {
                continue;
            }
            let listed =
                conversation_skills(&computer_skills, &switched.skills, Some(&project.id)).listed;
            project_views.push(place_view(
                SkillPlace::Project {
                    project_id: project.id.clone(),
                },
                Some(project.name.clone()),
                &skills_folder_in(&folder),
                &switched,
                fit_list(&listed).1,
            ));
        }
        SkillsOverview {
            computer: computer_view,
            projects: project_views,
        }
    }

    /// Turns a skill on or off in its place, and returns the new overview. A
    /// project's skill that the owner turns on keeps the digest of what
    /// agents' list shows of it.
    pub(crate) async fn set_on(
        &self,
        store: &Store,
        input: &SetSkillOn,
        projects: &[ProjectView],
    ) -> Result<SkillsOverview, SkillError> {
        if !is_skill_name(&input.name) {
            return Err(SkillError::FieldNotValid {
                field: SkillField::Name,
            });
        }
        let (place, workspace) = match &input.place {
            SkillPlace::Computer => (None, None),
            SkillPlace::Project { project_id } => {
                let project = projects
                    .iter()
                    .find(|project| project.id == *project_id)
                    .ok_or(SkillError::ProjectNotFound)?;
                (
                    Some(project_id.clone()),
                    Some(PathBuf::from(&project.folder)),
                )
            }
        };
        let scan = self
            .scan(&place, workspace.as_deref(), true)
            .await
            .ok_or(SkillError::NotFound)?;
        let skill = scan
            .skills
            .iter()
            .find(|skill| skill.name == input.name)
            .ok_or(SkillError::NotFound)?;
        let digest = (place.is_some() && input.on).then(|| skill.digest());
        store
            .set_skill_switch(&StoredSkillSwitch {
                project_id: place.clone(),
                name: input.name.clone(),
                is_on: input.on,
                digest: digest.clone(),
            })
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "a skill's switch could not be saved");
                SkillError::StorageFailed
            })?;
        lock(&self.switches).insert(
            (place, input.name.clone()),
            Switch {
                is_on: input.on,
                digest,
            },
        );
        Ok(self.overview(projects).await)
    }
}

impl Runtime {
    /// The skills of this computer and of every project, read fresh, for the
    /// settings (docs/features/skills.md, contracts). A machine without skills
    /// has none to list.
    pub async fn list_skills(&self) -> Result<SkillsOverview, SkillError> {
        let Some(catalog) = &self.inner.skills else {
            return Ok(SkillsOverview {
                computer: None,
                projects: Vec::new(),
            });
        };
        let projects = self.skill_projects().await?;
        Ok(catalog.overview(&projects).await)
    }

    /// Turns a skill on or off in its place, from the next model call, and
    /// returns the skills as they are then.
    pub async fn set_skill_on(&self, input: SetSkillOn) -> Result<SkillsOverview, SkillError> {
        let Some(catalog) = &self.inner.skills else {
            return Err(SkillError::NotFound);
        };
        let projects = self.skill_projects().await?;
        catalog.set_on(&self.inner.store, &input, &projects).await
    }

    /// Names the folder of this computer's skills, as a validation build's
    /// test does, so the end-to-end tests never read the owner's own skills.
    #[cfg(feature = "validation")]
    pub fn set_skills_folder(&self, folder: Option<PathBuf>) {
        if let Some(catalog) = &self.inner.skills {
            catalog.set_computer_folder(folder);
        }
    }

    /// The owner's projects, for the skills' commands.
    async fn skill_projects(&self) -> Result<Vec<ProjectView>, SkillError> {
        self.inner.store.projects().await.map_err(|error| {
            tracing::error!(%error, error.code = error.code(), "the projects could not be read for skills");
            SkillError::StorageFailed
        })
    }

    /// What a run's call reads of skills: the skills of its conversation's
    /// places for a run that reads skills, and none for an assigned task
    /// (ADR-0061, decision 1).
    pub(crate) async fn run_skills(
        &self,
        setup: &Setup,
        project: Option<&ProjectView>,
    ) -> ConversationSkills {
        let reads_skills = setup.tool(READ_SKILL).is_some();
        if !reads_skills || setup.assigned.is_some() {
            return ConversationSkills::default();
        }
        self.conversation_skills(project).await
    }

    /// What the places of a conversation hold, for a run that reads skills: an
    /// empty set on a machine without skills.
    pub(crate) async fn conversation_skills(
        &self,
        project: Option<&ProjectView>,
    ) -> ConversationSkills {
        let Some(catalog) = &self.inner.skills else {
            return ConversationSkills::default();
        };
        let project = project.map(|project| (&project.id, Path::new(&project.folder)));
        catalog.for_conversation(project).await
    }
}

/// What a conversation's places hold: the skills that are on, a project's
/// before this computer's and hiding those of the same name, then the
/// project's skills that are off, which "@" and `read_skill` still reach.
fn conversation_skills(
    computer: &[Switched],
    project: &[Switched],
    project_id: Option<&ProjectId>,
) -> ConversationSkills {
    let project_on: Vec<&Switched> = project.iter().filter(|skill| skill.is_on).collect();
    let hidden = |name: &str| project_on.iter().any(|skill| skill.skill.name == name);
    let computer_on: Vec<&Switched> = computer
        .iter()
        .filter(|skill| skill.is_on && !hidden(&skill.skill.name))
        .collect();
    let project_off: Vec<&Switched> = project.iter().filter(|skill| !skill.is_on).collect();

    let listed = project_on
        .iter()
        .chain(&computer_on)
        .filter(|skill| skill.skill.listed)
        .map(|skill| ListedSkill {
            name: skill.skill.name.clone(),
            description: skill.skill.description.clone(),
        })
        .collect();
    let shelved = |skill: &&Switched| ShelvedSkill {
        name: skill.skill.name.clone(),
        folder: skill.skill.folder.clone(),
        is_on: skill.is_on,
    };
    let shelf = SkillShelf::new(
        project_on
            .iter()
            .chain(&computer_on)
            .chain(&project_off)
            .map(shelved)
            .collect(),
    );
    let project_place = project_id.map(|id| SkillPlace::Project {
        project_id: id.clone(),
    });
    let mut mentionable = Vec::new();
    if let Some(place) = &project_place {
        for skill in project.iter().filter(|skill| skill.skill.mentionable) {
            mentionable.push(Mentionable {
                place: place.clone(),
                name: skill.skill.name.clone(),
                folder: skill.skill.folder.clone(),
                is_on: skill.is_on,
            });
        }
    }
    for skill in computer_on.iter().filter(|skill| skill.skill.mentionable) {
        mentionable.push(Mentionable {
            place: SkillPlace::Computer,
            name: skill.skill.name.clone(),
            folder: skill.skill.folder.clone(),
            is_on: true,
        });
    }
    ConversationSkills {
        listed,
        shelf,
        mentionable,
    }
}

/// A place as the settings show it.
fn place_view(
    place: SkillPlace,
    label: Option<String>,
    folder: &Path,
    switched: &SwitchedPlace,
    left_out: usize,
) -> SkillPlaceView {
    SkillPlaceView {
        place,
        label,
        folder: folder.display().to_string(),
        skills: switched
            .skills
            .iter()
            .map(|skill| SkillView {
                name: skill.skill.name.clone(),
                description: skill.skill.description.clone(),
                folder: skill.skill.folder.display().to_string(),
                is_on: skill.is_on,
                changed: skill.changed,
                listed: skill.skill.listed,
                mentionable: skill.skill.mentionable,
                warnings: skill.skill.warnings.clone(),
            })
            .collect(),
        problems: switched
            .scan
            .problems
            .iter()
            .map(|problem| SkillProblemView {
                folder: problem.folder.clone(),
                problem: problem.problem,
            })
            .collect(),
        failure: switched.scan.failure,
        incomplete: switched.scan.incomplete,
        left_out: u32::try_from(left_out).unwrap_or(u32::MAX),
    }
}

/// The token of a skill's mention: `@skill_` and the name with underscores in
/// place of hyphens. It is made of word characters only, so it never holds a
/// project's name as a word, and a skill's name never stands in for the
/// owner's words about a project (T-92).
pub fn skill_token(name: &str) -> String {
    format!("@skill_{}", name.replace('-', "_"))
}

/// The key of a skill's mention: its place and its name.
fn mention_key(place: &SkillPlace, name: &str) -> String {
    match place {
        SkillPlace::Computer => format!("computer:{name}"),
        SkillPlace::Project { .. } => format!("project:{name}"),
    }
}

/// What a skill's mention gives the model.
struct SkillContent {
    name: String,
    folder: String,
    body: String,
}

impl MentionContent for SkillContent {
    fn render(&self, limit: usize) -> RenderedMention {
        let heading = texts::skill_mention_heading(&self.name, &self.folder);
        let whole = format!("{heading}\n\n{}", self.body);
        let total = u32::try_from(self.body.len()).unwrap_or(u32::MAX);
        let (text, summary) = if whole.len() <= limit {
            (
                whole,
                MentionSummary {
                    code: SUMMARY_WHOLE.to_owned(),
                    values: [("total".to_owned(), total)].into(),
                },
            )
        } else {
            // The note's numbers are short, so room for these digits always
            // holds them.
            let note_room = texts::skill_cut_note(u32::MAX, u32::MAX).len();
            let room = limit.saturating_sub(heading.len() + note_room + 2);
            let shown_text = &self.body[..self.body.floor_char_boundary(room)];
            let shown = u32::try_from(shown_text.len()).unwrap_or(u32::MAX);
            (
                format!(
                    "{heading}\n\n{shown_text}{}",
                    texts::skill_cut_note(shown, total)
                ),
                MentionSummary {
                    code: SUMMARY_CUT.to_owned(),
                    values: [("shown".to_owned(), shown), ("total".to_owned(), total)].into(),
                },
            )
        };
        RenderedMention::new(text, self.name.clone(), SKILL_VARIANT.to_owned(), summary)
    }
}

/// The core's provider of skills' mentions (ADR-0061, decision 9). It offers
/// the skills that are on and the project's skills that are off, and reads a
/// skill's instructions only where the policy lets the model read them
/// without asking.
#[derive(Debug, Clone)]
pub struct SkillMentions {
    catalog: Arc<SkillCatalog>,
}

impl SkillMentions {
    /// The provider of the skills of `catalog`.
    pub fn new(catalog: Arc<SkillCatalog>) -> Self {
        Self { catalog }
    }

    async fn skills_of(&self, context: &MentionContext) -> ConversationSkills {
        let project = context.project().zip(context.workspace());
        self.catalog.for_conversation(project).await
    }
}

impl MentionProvider for SkillMentions {
    fn id(&self) -> &str {
        SKILLS
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        true
    }

    fn suggest<'a>(
        &'a self,
        context: &'a MentionContext,
        query: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        async move {
            let skills = self.skills_of(context).await;
            let query = query.to_lowercase();
            let mut matching: Vec<&Mentionable> = skills
                .mentionable
                .iter()
                .filter(|skill| {
                    skill.name.contains(&query) || skill_token(&skill.name).contains(&query)
                })
                .collect();
            // Names that start with what the owner typed come first.
            matching.sort_by_key(|skill| (!skill.name.starts_with(&query), skill.name.clone()));
            let incomplete = matching.len() > limit;
            let entries = matching
                .into_iter()
                .take(limit)
                .map(|skill| MentionEntry {
                    key: mention_key(&skill.place, &skill.name),
                    label: skill.name.clone(),
                    token: skill_token(&skill.name),
                    variant: SKILL_VARIANT.to_owned(),
                    warning: (!skill.is_on).then(|| OFF_WARNING.to_owned()),
                    expands: false,
                })
                .collect();
            Ok(ProviderSuggestions::new(entries, incomplete))
        }
        .boxed()
    }

    fn resolve<'a>(
        &'a self,
        context: &'a MentionContext,
        key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        async move {
            // The key comes from the webview, so it is checked against what
            // the place offers now.
            let skills = self.skills_of(context).await;
            let skill = skills
                .mentionable
                .iter()
                .find(|skill| mention_key(&skill.place, &skill.name) == key)
                .ok_or_else(|| SKILL_NOT_FOUND.to_owned())?;
            let folder = skill.folder.clone();
            let instructions =
                tokio::task::spawn_blocking(move || read_skill_instructions(&folder))
                    .await
                    .map_err(|_stopped| SKILL_UNREADABLE.to_owned())?
                    .map_err(|_failure| SKILL_UNREADABLE.to_owned())?;
            // A mention never asks: the read runs only where the policy lets
            // the model read without asking (ADR-0032, decision 8).
            let access = plugin_path_access(&instructions.path, AccessMode::Read, None)
                .map_err(|_failure| SKILL_UNREADABLE.to_owned())?;
            match context.decide(&[access]) {
                Verdict::Allowed(_) | Verdict::Unrestricted => {}
                Verdict::Ask(_) | Verdict::Refused(_) => return Err(SKILL_UNREADABLE.to_owned()),
            }
            let rendered: Box<dyn MentionContent> = Box::new(SkillContent {
                name: skill.name.clone(),
                folder: skill.folder.display().to_string(),
                body: instructions.body,
            });
            Ok(rendered)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::{ListedSkill, MAX_LIST_BYTES, fit_list, skill_token};

    fn listed(name: &str, description: &str) -> ListedSkill {
        ListedSkill {
            name: name.to_owned(),
            description: description.to_owned(),
        }
    }

    #[test]
    fn a_list_that_fits_stays_whole() {
        let skills = vec![
            listed("pdf", "Fills forms."),
            listed("docx", "Writes documents."),
        ];

        assert_eq!(fit_list(&skills), (skills.clone(), 0));
    }

    #[test]
    fn a_list_that_does_not_fit_cuts_descriptions_then_leaves_skills_out() {
        let long = "a description word ".repeat(60);
        let skills: Vec<ListedSkill> = (0..100)
            .map(|number| listed(&format!("skill-{number:03}"), &long))
            .collect();

        let (fitted, left_out) = fit_list(&skills);

        assert!(left_out > 0);
        assert_eq!(fitted.len() + left_out, skills.len());
        assert_eq!(fitted[0].name, "skill-000");
        assert!(
            fitted
                .iter()
                .all(|skill| skill.description.chars().count() <= 250)
        );
        let bytes: usize = fitted
            .iter()
            .map(|skill| skill.name.len() + skill.description.len() + 5)
            .sum();
        assert!(bytes <= MAX_LIST_BYTES, "{bytes}");
    }

    #[test]
    fn a_token_is_made_of_word_characters() {
        assert_eq!(skill_token("pdf-forms"), "@skill_pdf_forms");
        assert!(
            skill_token("my-project")
                .trim_start_matches('@')
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
        );
    }
}
