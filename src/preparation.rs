use crate::{
    Comment, CommentView, Result, Scope, Snapshot, ThreadRef, ViewSpec, git, github, model,
    store::Store,
};
use std::path::Path;

/// A visible preview and its anchor-validation context share one revision.
/// Keep the returned Store alive through validation and delivery: it owns the lock.
pub(crate) struct PreparedReview {
    pub visible: Snapshot,
    validation: Snapshot,
}

impl PreparedReview {
    pub fn open(state_dir: &Path, scope: &Scope, view: &ViewSpec) -> Result<(Self, Store)> {
        let mut visible = git::snapshot(scope, view)?;
        let store = Store::open(state_dir, &visible.scope)?;
        let paths = store
            .comments
            .iter()
            .map(|c| c.path.clone())
            .collect::<Vec<_>>();
        git::hydrate(&mut visible, &paths)?;
        git::seal(&mut visible)?;
        let mut validation = git::validation(&visible, &store.comments)?;
        git::seal(&mut validation)?;
        visible.revision =
            model::hash(format!("{}:{}", visible.revision, validation.revision).as_bytes());
        validation.revision = visible.revision.clone();
        Ok((
            Self {
                visible,
                validation,
            },
            store,
        ))
    }

    pub fn comments(&self, comments: &[Comment]) -> Vec<CommentView> {
        crate::views(&self.validation, comments)
    }

    pub fn feedback(
        &self,
        comments: &[Comment],
        include_local: bool,
        cache: Option<&github::Cache>,
        threads: &[ThreadRef],
    ) -> Result<String> {
        crate::combined_feedback(&self.validation, comments, include_local, cache, threads)
    }
}
