# Publishing a release

Update prompts use the latest **published, stable GitHub release** in
`jakub-tucek/herdr-annotator`. Tags must use `vMAJOR.MINOR.PATCH`. A tag by itself
does not publish an update; draft/prerelease entries do not qualify.

1. Set the same version in `Cargo.toml`, the root package entry in `Cargo.lock`,
   and `mods/annotator/mod.json`.
2. Run the checks:

   ```sh
   cargo fmt --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked
   npm test
   python3 -m unittest discover -s tests -p 'test_*.py'
   herdr-app check mods/annotator
   ```

3. Commit and push the release source. Check that an ordinary installation's
   previous release is an ancestor of this commit. The updater refuses a
   non-fast-forward source history.
4. Create a GitHub release with a tag such as `v0.2.0`, targeting that exact
   commit. Write release notes describing changes and any manual migration.
   Publish it as a stable release, not a draft/prerelease. Source-only releases
   need no binary assets: the updater builds from the pinned source commit.
5. In a clean test installation, choose **Updates → Check now**. Older versions
   should offer the release; the same version should show no upgrade. Verify
   refusal with an unsaved draft and a dirty source tree before testing the
   confirmed install.

Do not move a published tag. A changed tag SHA causes the updater to refuse an
install until the person checks again. Publish a new patch release for fixes.

Existing users need one manual installation of 0.2.0 or newer to get update
prompts. Subsequent higher stable releases can use the in-app Update button.
