# Herdr Annotator

Reviewing checkout changes and delivering feedback to the source session.

## Language

**Draft**: Unsaved feedback text with anchoring intent and the revision where that intent was chosen. Invalidated line intent requires reselection or explicit conversion to a file note.

**File note**: Feedback about an entire file, chosen without a line anchor.

**Visible snapshot**: The review preview at the requested file and context depth.

**Validation snapshot**: Expanded context used to validate saved anchors, distinct from the visible snapshot but sharing its coherent revision safeguard.

**Source session**: The agent session belonging to the originating machine, worktree, and tab. Delivery requires an idle source session.

**Uncertain delivery**: Persisted delivery intent without confirmed submission. Never retried automatically.
