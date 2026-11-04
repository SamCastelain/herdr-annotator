use crate::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub repo: String,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default, skip_serializing_if = "Comparison::is_default")]
    pub comparison: Comparison,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffMode {
    #[default]
    Uncommitted,
    Branch,
    LastCommit,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comparison {
    #[serde(default)]
    pub mode: DiffMode,
    #[serde(default)]
    pub base: Option<String>,
}

impl Comparison {
    pub fn is_default(&self) -> bool {
        self.mode == DiffMode::Uncommitted && self.base.is_none()
    }
}

fn yes() -> bool {
    true
}
fn context_lines() -> u32 {
    3
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSpec {
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default = "context_lines")]
    pub context: u32,
}
impl Default for ViewSpec {
    fn default() -> Self {
        Self {
            file: None,
            context: 3,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadRef {
    pub id: String,
    pub fingerprint: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderingRunner {
    #[default]
    Opencode,
    Pi,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Order {
        scope: Scope,
        revision: String,
        #[serde(default)]
        runner: OrderingRunner,
        #[serde(default)]
        view: ViewSpec,
    },
    Snapshot {
        scope: Scope,
        #[serde(default)]
        view: ViewSpec,
    },
    Add {
        scope: Scope,
        revision: String,
        path: String,
        selection: Option<Selection>,
        body: String,
        #[serde(default)]
        reviewed: Option<ReviewedDraft>,
        #[serde(default)]
        view: ViewSpec,
    },
    Edit {
        scope: Scope,
        revision: String,
        id: String,
        expected_body: String,
        body: String,
        #[serde(default)]
        view: ViewSpec,
    },
    Delete {
        scope: Scope,
        revision: String,
        id: String,
        expected_body: String,
        #[serde(default)]
        view: ViewSpec,
    },
    Ask {
        scope: Scope,
        revision: String,
        target: String,
        question_id: String,
        path: String,
        context: QuestionContext,
        body: String,
        #[serde(default)]
        retry: bool,
        #[serde(default)]
        view: ViewSpec,
    },
    Resolve {
        scope: Scope,
        id: String,
        resolved: bool,
        #[serde(default)]
        view: ViewSpec,
    },
    Read {
        scope: Scope,
        id: String,
        read: bool,
        #[serde(default)]
        view: ViewSpec,
    },
    GithubRead {
        scope: Scope,
        thread: ThreadRef,
        read: bool,
    },
    GithubReply {
        scope: Scope,
        thread: ThreadRef,
        body: String,
        reply_id: String,
        #[serde(default)]
        retry: bool,
    },
    Export {
        scope: Scope,
        revision: String,
        #[serde(default)]
        note: String,
        #[serde(default)]
        threads: Vec<ThreadRef>,
        #[serde(default = "yes")]
        include_local: bool,
        #[serde(default)]
        view: ViewSpec,
    },
    Send {
        scope: Scope,
        revision: String,
        target: String,
        #[serde(default)]
        note: String,
        #[serde(default)]
        threads: Vec<ThreadRef>,
        #[serde(default = "yes")]
        include_local: bool,
        #[serde(default)]
        view: ViewSpec,
    },
    Github {
        scope: Scope,
        #[serde(default)]
        force: bool,
        #[serde(default)]
        poll_seconds: Option<u64>,
    },
}

impl Request {
    pub fn scope(&self) -> &Scope {
        match self {
            Self::Snapshot { scope, .. }
            | Self::Order { scope, .. }
            | Self::Add { scope, .. }
            | Self::Edit { scope, .. }
            | Self::Delete { scope, .. }
            | Self::Ask { scope, .. }
            | Self::Resolve { scope, .. }
            | Self::Read { scope, .. }
            | Self::GithubRead { scope, .. }
            | Self::GithubReply { scope, .. }
            | Self::Export { scope, .. }
            | Self::Send { scope, .. }
            | Self::Github { scope, .. } => scope,
        }
    }
    pub fn view(&self) -> ViewSpec {
        match self {
            Self::Snapshot { view, .. }
            | Self::Order { view, .. }
            | Self::Add { view, .. }
            | Self::Edit { view, .. }
            | Self::Delete { view, .. }
            | Self::Ask { view, .. }
            | Self::Resolve { view, .. }
            | Self::Read { view, .. }
            | Self::Export { view, .. }
            | Self::Send { view, .. } => view.clone(),
            Self::Github { .. } | Self::GithubRead { .. } | Self::GithubReply { .. } => {
                ViewSpec::default()
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionContext {
    File,
    Selection { selection: Selection },
    Hunk { row: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub scope: Scope,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub revision: String,
    pub files: Vec<ChangedFile>,
    pub limited: bool,
    pub bases: Vec<String>,
    pub merge_base: Option<String>,
    pub baseline: Option<String>,
    pub view: ViewSpec,
}

impl Snapshot {
    pub fn require_revision(&self, revision: &str) -> Result<()> {
        if self.revision != revision {
            return Err("Diff changed; refresh before saving or sending".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub rows: Vec<DiffLine>,
    pub note: Option<String>,
    pub truncated: bool,
    pub loaded: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Old,
    New,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub side: Side,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineAnchor {
    pub selection: Selection,
    pub selected: Vec<String>,
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub lines: Option<LineAnchor>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedDraft {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub baseline: Option<String>,
    pub anchor: Anchor,
}

impl ReviewedDraft {
    pub(crate) fn current_anchor(
        &self,
        snapshot: &Snapshot,
        file: &ChangedFile,
        selection: Option<Selection>,
    ) -> Result<Anchor> {
        if self.branch != snapshot.branch || (self.branch.is_none() && self.head != snapshot.head) {
            return Err("The reviewed branch changed. Your draft is kept; return to that branch or select its intended code.".into());
        }
        if snapshot.scope.comparison.mode != DiffMode::Uncommitted
            && self.baseline != snapshot.baseline
        {
            return Err("The reviewed comparison changed. Your draft is kept; select its intended code again.".into());
        }
        match (&self.anchor.lines, &selection) {
            (None, None) => (),
            (Some(lines), Some(selected))
                if &lines.selection == selected
                    && selected.start > 0
                    && selected.end >= selected.start
                    && selected.end - selected.start < 200
                    && lines.selected.len() == (selected.end - selected.start + 1) as usize
                    && lines.before.len() <= 2
                    && lines.after.len() <= 2 =>
            {}
            _ => return Err("Invalid reviewed selection".into()),
        }
        let selection = self.anchor.locate(file).ok_or("The selected code changed or is ambiguous. Your draft is kept; select its intended lines again, or choose Whole file.")?;
        Anchor::create(file, selection)
    }
}

fn source(file: &ChangedFile, side: Side) -> Vec<(u32, &str)> {
    file.rows
        .iter()
        .filter_map(|r| {
            let n = match side {
                Side::Old => r.old,
                Side::New => r.new,
            };
            n.map(|n| (n, r.text.as_str()))
        })
        .collect()
}

impl Anchor {
    pub fn create(file: &ChangedFile, selection: Option<Selection>) -> Result<Self> {
        let Some(selection) = selection else {
            return Ok(Self { lines: None });
        };
        if file.truncated {
            return Err("Line comments are disabled for truncated diffs; use a file note".into());
        }
        if selection.start == 0
            || selection.end < selection.start
            || selection.end - selection.start >= 200
        {
            return Err("Select 1–200 consecutive source lines on one diff side".into());
        }
        let lines = source(file, selection.side);
        let start = lines
            .iter()
            .position(|(n, _)| *n == selection.start)
            .ok_or("Start line is not in the diff")?;
        let len = (selection.end - selection.start + 1) as usize;
        let selected = lines
            .get(start..start + len)
            .ok_or("Selection extends past the diff")?;
        if !selected
            .iter()
            .enumerate()
            .all(|(i, (n, _))| *n == selection.start + i as u32)
        {
            return Err("Selection crosses hidden context; select one contiguous hunk".into());
        }
        let before = lines[..start]
            .iter()
            .rev()
            .take(2)
            .rev()
            .map(|(_, s)| (*s).to_owned())
            .collect();
        let after = lines[start + len..]
            .iter()
            .take(2)
            .map(|(_, s)| (*s).to_owned())
            .collect();
        Ok(Self {
            lines: Some(LineAnchor {
                selection,
                selected: selected.iter().map(|(_, s)| (*s).to_owned()).collect(),
                before,
                after,
            }),
        })
    }

    // Some(None): current file-level note; None: stale; Some(Some(..)): current line range.
    pub fn locate(&self, file: &ChangedFile) -> Option<Option<Selection>> {
        let Some(anchor) = &self.lines else {
            return Some(None);
        };
        if file.truncated || anchor.selected.is_empty() {
            return None;
        }
        let lines = source(file, anchor.selection.side);
        let len = anchor.selected.len();
        let candidates: Vec<usize> = lines
            .windows(len)
            .enumerate()
            .filter_map(|(i, window)| {
                let contiguous = window
                    .iter()
                    .enumerate()
                    .all(|(j, (n, _))| *n == window[0].0 + j as u32);
                (contiguous
                    && window
                        .iter()
                        .zip(&anchor.selected)
                        .all(|((_, a), b)| *a == b))
                .then_some(i)
            })
            .collect();
        let contextual: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|i| {
                let before = i
                    .checked_sub(anchor.before.len())
                    .and_then(|start| lines.get(start..*i));
                let after = lines.get(i + len..i + len + anchor.after.len());
                before.is_some_and(|s| {
                    s.iter()
                        .map(|(_, s)| *s)
                        .eq(anchor.before.iter().map(String::as_str))
                }) && after.is_some_and(|s| {
                    s.iter()
                        .map(|(_, s)| *s)
                        .eq(anchor.after.iter().map(String::as_str))
                })
            })
            .collect();
        let start = if contextual.len() == 1 {
            contextual[0]
        } else if candidates.len() == 1 {
            candidates[0]
        } else {
            return None;
        };
        Some(Some(Selection {
            side: anchor.selection.side,
            start: lines[start].0,
            end: lines[start + len - 1].0,
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub path: String,
    pub branch: Option<String>,
    pub body: String,
    pub anchor: Anchor,
    pub resolved: bool,
    #[serde(default)]
    pub read: bool,
    pub delivery: Option<Delivery>,
    #[serde(default = "context_lines")]
    pub context: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Sent,
    Uncertain,
}

#[derive(Debug, Serialize)]
pub struct CommentView {
    #[serde(flatten)]
    pub comment: Comment,
    pub stale: bool,
    pub position: Option<Option<Selection>>,
}
