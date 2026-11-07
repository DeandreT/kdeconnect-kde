# Contributing to this fork

This independently maintained fork accepts contributions at
https://github.com/DeandreT/kdeconnect-kde. Do not submit fork changes to KDE's
upstream project on behalf of this fork.

Keep changes focused, preserve existing copyright and license notices, and
include regression tests for changed behavior. Use conventional commit subjects.
Do not add contributor sign-off or attribution trailers.

For Rust changes, run formatting, Clippy, workspace tests, and a workspace build.
Test graphical changes on an isolated display. Never use real messages, contacts,
credentials, or paired-device identifiers in fixtures or screenshots.

Changes require owner review before merging. Do not replace the installed KDE
Connect service, migrate pairing credentials, or send messages during automated
tests. Keep the original implementation available while Rust components are
being developed.
