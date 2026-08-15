---
title: 'Sortable table columns'
type: 'feature'
created: '2026-08-15'
status: 'done'
review_loop_iteration: 0
context: []
baseline_commit: 'd07a4ca4d2b9cfbe6249792e7aadb89ca5090515'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** The file table has no user-controlled ordering, so users cannot alphabetize records by the name or tag column they are comparing.

**Approach:** Make every text-data header clickable. The first click sorts that column ascending, subsequent clicks toggle ascending/descending, and the active header displays a small adjacent up/down arrow matching the applied direction.

## Boundaries & Constraints

**Always:** Sort case-insensitively over the complete filtered result set; keep one active sort per category in memory; reapply it after search, filter, edit, rescan, and async result refreshes; keep rendered order identical to the order used by selection and row actions; use name then path as deterministic tie-breakers; preserve existing header context menus and Alt-click tag deletion.

**Ask First:** Adding a third “unsorted” click state, persisting sort preferences across launches, or introducing locale-aware/natural-number collation.

**Never:** Sort only the virtualized visible slice, make the add-column action sortable, mutate the database/backend record order, or replace the custom table with `DataTable`.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| First activation | Click an inactive name or tag header | Full result set sorts ascending and only that header shows an up arrow | N/A |
| Direction toggle | Click the active header again | Full result set sorts descending and the icon becomes a down arrow | N/A |
| Column switch | Click another data header | New column becomes ascending; previous indicator disappears | N/A |
| Mixed case and ties | Values differ only by case or repeat | Compare lowercase text, then name and path for stable deterministic order | N/A |
| Multi-value tag cell | Cell contains several displayed tag strings | Compare their displayed-order values joined as one text value | N/A |
| Missing tag value | Some records lack the active tag | Blank sorts before text ascending and after text descending | N/A |
| Results rebuilt | Active sort plus search/filter/edit/rescan completion | Rebuilt results retain and reapply the active category sort | Stale async generations remain governed by existing guards |
| Tag key changed | Active tag column is renamed or removed | Rename follows the key; removal clears that category’s sort to default ordering | Backend failure leaves existing UI state unchanged |

</frozen-after-approval>

## Code Map

- `src/model.rs:94,288-294,435-464` -- `FileRecord`, per-category result state, and current case-insensitive search/name ordering; add small sort value types and category sort state here.
- `src/library.rs:391-430,643-688,1345-1426,1524-1536` -- async/sync result rebuilds and tag-key mutations; centralize comparator application and expose a toggle API so every rebuild preserves the behavioral row order.
- `src/ui/table.rs:592-639,802-854,1551-1598` -- tag width calculation/cache and range selection; reserve compact indicator space without diverging from shared result order.
- `src/ui/table/render.rs:11-64,377-395,547-699,779-898` -- virtualized rows, displayed values, and current name/tag headers; attach unique left-click targets and render the active direction icon beside the label.
- `src/library/tests.rs` -- established Library fixtures and search-order tests; cover sort transitions, comparators, refresh persistence, and tag-key lifecycle.
- `src/ui/table/tests.rs:1-126` -- existing pure table unit-test style; add only presentation-helper coverage that cannot live with Library behavior.
- `src/ui/settings_menu.rs:320-389` -- proven `IconName::ArrowUp`/`ArrowDown` usage to reuse.

## Tasks & Acceptance

**Execution:**
- [x] `src/model.rs` -- define the active column/direction state with an unsorted default -- keep sorting explicit and category-local.
- [x] `src/library.rs` -- implement toggle/comparison/reapplication and reconcile renamed or removed tag keys -- make the shared result vector authoritative for UI and row actions.
- [x] `src/ui/table.rs`, `src/ui/table/render.rs` -- wire header clicks, reserve indicator width, and render only the active up/down icon -- deliver the requested interaction without breaking existing header gestures.
- [x] `src/library/tests.rs` -- test state transitions and all matrix edge cases with lightweight unit tests -- prevent ordering and interaction-state regressions; no table-only presentation helper required.

**Acceptance Criteria:**
- Given any populated table, when the user clicks a text-data header repeatedly, then rows toggle between ascending and descending alphabetical order and the adjacent arrow always depicts the applied direction.
- Given a sorted table, when the user searches, filters, edits metadata, or receives refreshed results, then the same active sort remains applied to the complete matching set.
- Given existing selection, drag, Alt-click, and context-menu interactions, when sorting is used, then those behaviors operate on the displayed order and remain available.

## Spec Change Log

## Design Notes

Sort the owned `CategoryState.results` once when state changes; rendering must consume it directly. For tag cells, derive a normalized joined display string, compare that primary key in the selected direction, and keep deterministic tie-breakers ascending so equal visible values do not jump unpredictably.

## Verification

**Commands:**
- `cargo test` -- all existing and new unit tests pass.
- `cargo fmt --check` -- modified Rust files satisfy repository formatting.

**Manual checks (if no CLI):**
- Run normal `cargo run`; click name and tag headers and confirm order, single arrow placement/direction, no clipping, search/filter persistence, Alt-click deletion, and right-click menus.

## Suggested Review Order

**Sort state and result boundary**

- Centralizes per-category direction changes and normalized tag-column identity.
  [`library.rs:234`](../../src/library.rs#L234)

- Applies the active sort after every filtered result rebuild.
  [`library.rs:1561`](../../src/library.rs#L1561)

- Defines deterministic case-insensitive ordering and tag-cell value handling.
  [`library.rs:1578`](../../src/library.rs#L1578)

- Reconciles active tag sorting when keys are renamed or removed.
  [`library.rs:1611`](../../src/library.rs#L1611)

**Rendered header interaction**

- Makes the complete name header clickable while preserving right-click visibility behavior.
  [`render.rs:802`](../../src/ui/table/render.rs#L802)

- Makes tag headers clickable, arrow-bearing, and compatible with Alt-delete/context menus.
  [`render.rs:848`](../../src/ui/table/render.rs#L848)

- Reserves width for the active indicator before rendering tag columns.
  [`table.rs:601`](../../src/ui/table.rs#L601)

**Supporting types and verification**

- Stores explicit category-local sort state with a two-direction toggle.
  [`model.rs:52`](../../src/model.rs#L52)

- Covers comparator edge cases, refreshes, async searches, category isolation, and key lifecycle.
  [`tests.rs:73`](../../src/library/tests.rs#L73)
