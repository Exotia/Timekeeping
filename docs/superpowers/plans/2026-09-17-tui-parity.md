# TUI parity (backup key, projects screen, range marking) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring three command-line-only operations into the `tk` TUI: a backup key, a projects screen (archive / unarchive / rename / add), and marking a range of days with one day-type key.

**Architecture:** The TUI is tui-realm + ratatui. Key presses are mapped to `Msg` values by small `AppComponent`s in `src/tui/components/`; `Model::update` in `src/tui/model.rs` handles each `Msg`, sending `StoreCmd`s to a worker thread (`src/tui/worker.rs::handle`) which answers with `StoreReply`s; `Model::draw` dispatches on `Screen` to pure drawing functions in `src/tui/view/` that are tested with ratatui's `TestBackend`. Each feature adds its `Msg`/`StoreCmd`/`StoreReply` variants, a component key mapping, model handlers, a worker arm, drawing, README rows and tests. No storage or core-logic changes: every store function needed already exists.

**Tech Stack:** Rust 2024, tuirealm 4.1, ratatui 0.30, rusqlite, chrono. Toolchain only inside `nix develop`.

**Spec:** No separate spec file. The approved design is the user's answers recorded in this plan's task descriptions (2026-09-17): quieter days off / bolder work rows (already done, commit 9682ac6), `B` backup, `P` projects screen, `V` range anchor. Export overlay, absence-label prompt and clock-out-onto-another-project were explicitly declined — do not add them.

## Global Constraints

- **Parallel execution.** The three tasks are implemented at the same time by three different people, each in their own git worktree and branch off `feat/tui-v1`, and merged afterwards. So: touch only what your task needs, never reformat or reorder code you are not changing, and **append** your new enum variants at the end of `Msg`, `StoreCmd`, `StoreReply`, `Confirm`, `Screen`, `Id` under a comment line `// --- <your feature> ---`, and append new `match` arms at the end of their match (before a `_ =>` catch-all if there is one). That keeps the later merge mechanical.
- **Toolchain.** There is no cargo on PATH. Run every cargo command as `nix develop --command cargo ...` from the worktree root (the `Git tree is dirty` warning from nix is normal). `nix develop --command cargo fmt` and `nix develop --command cargo clippy --all-targets -- -D warnings` must pass before each commit, and the full `nix develop --command cargo test` must be green.
- **TDD.** Write the failing test, run it and see it fail for the expected reason, then implement. The report must carry RED and GREEN evidence.
- **Keys.** Uppercase letters (`B`, `P`, `V`) arrive as `Key::Char('B')` etc. with `KeyModifiers::SHIFT` — match on the char only, the way every other binding does. No existing binding uses an uppercase letter. Existing bindings in the month screen: `q ↑ k ↓ j [ ] PgUp PgDn t Enter s c i o b v f x p w u ? Esc`, Ctrl+C.
- **80 columns.** The bottom hint row of every screen must fit into 80 columns; `tests/snapshots.rs::key_hints_fit_the_minimum_terminal` enforces it (it loops over the `Screen` variants — a new screen must be added to that loop). The month hint row has no room left, so month-screen keys added by this plan go into the `?` help list (`Model::help_keys`) only, not into `Model::key_hints`.
- **Status messages** are set with `Model::set_status(msg, is_error)`. A `StoreReply::Changed(String)` from the worker is shown as a status and triggers `reload_after_write`; a `StoreReply::Failed(String)` (any `Err` returned by `handle`) is shown as an error. Error text is a plain sentence, no trailing period, matching existing ones like `Day has entries; delete them first`.
- **README** is user documentation and is updated in the same task as the code: the key tables under `### Month view` (README.md:250-269), a new section for a new screen, and the CLI command table rows (README.md:186-204) get a note where the TUI now offers the same thing.
- **Module rules.** `core` has no I/O. `store` is the only module that imports rusqlite. `cli` never imports from `tui` (`tui` may use `crate::cli::Ctx` and `crate::cli::commands` helpers, it already does).
- **Commits.** Small, one per red-green cycle or per coherent slice, messages in the repository's style (`feat(tui): ...`, `docs: ...`, imperative, lower case after the prefix). End every commit message with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  ```
- **No new dependencies.**

## File map

```
src/tui/msg.rs                 Msg / StoreCmd / StoreReply / Confirm enums, data structs (all three tasks append here)
src/tui/ids.rs                 Id enum: one per mounted component (Task 2 adds Projects, Prompt)
src/tui/mod.rs                 mounts full-screen components in run() (Task 2 mounts ProjectsScreen)
src/tui/model.rs               Model state, update(), on_store(), draw(), key_hints(), help_keys(), confirm_text()
src/tui/worker.rs              handle(): one arm per StoreCmd (all three tasks)
src/tui/components/month.rs    month key map (Task 1: B, Task 3: V, Task 2: P)
src/tui/components/projects.rs NEW (Task 2): projects screen key map
src/tui/components/text_prompt.rs NEW (Task 2): one-field name box overlay
src/tui/view/month.rs          month table drawing (Task 3: range highlight)
src/tui/view/projects.rs       NEW (Task 2): projects screen drawing
src/cli/commands.rs            Task 1 extracts write_backup() so CLI and TUI share it
tests/snapshots.rs             Task 2 adds Screen::Projects to the hint-row test
README.md                      all tasks
```

---

### Task 1: `B` writes a backup from the month view

**Files:**
- Modify: `src/cli/commands.rs:409-415` (extract `write_backup`)
- Modify: `src/tui/msg.rs` (`Msg::Backup`, `StoreCmd::Backup`)
- Modify: `src/tui/components/month.rs` (key map + test)
- Modify: `src/tui/model.rs` (`update` arm, `help_keys` month list, test)
- Modify: `src/tui/worker.rs` (`handle` arm + test)
- Modify: `README.md` (month key table, `tk backup` row, "Moving your data around")

**Interfaces:**
- Produces: `pub fn write_backup(ctx: &Ctx, now: NaiveDateTime) -> anyhow::Result<PathBuf>` in `src/cli/commands.rs`, used by both `Command::Backup` and the TUI worker.

**Behaviour:** Pressing `B` on the month screen copies the database to `<home>/backups/tk-YYYYmmdd-HHMMSS.db` (same file name the CLI writes) and shows `Backup written to <path>` in the status bar. No confirmation: a backup changes nothing. The `Changed` reply makes the model reload the month, which is harmless.

- [ ] **Step 1: Failing component test** — in `src/tui/components/month.rs` tests module, next to `b_asks_for_a_break`:

```rust
#[test]
fn shift_b_writes_a_backup() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('B'), KeyModifiers::SHIFT));
    assert_eq!(c.on(&ev), Some(Msg::Backup));
}
```

- [ ] **Step 2: Run it, expect a compile error** (`no variant named Backup`):
`nix develop --command cargo test --lib tui::components::month`

- [ ] **Step 3: Add the variants and the key**

`src/tui/msg.rs`, at the end of `Msg`:
```rust
    // --- backup key ---
    /// `B` on the month screen: copy the database into `<home>/backups/`.
    Backup,
```
at the end of `StoreCmd`:
```rust
    // --- backup key ---
    Backup,
```
`src/tui/components/month.rs`, inside the `Key::Char` match: `Key::Char('B') => Msg::Backup,`

`src/tui/model.rs` `update`: `Msg::Backup => self.send(StoreCmd::Backup),` and in `help_keys` for `Screen::Month` add `("B", "back up the database")` after the `("u", ...)` line. The `match cmd` in `worker.rs::handle` will not compile until Step 6 — add a temporary arm there returning `StoreReply::Changed(String::new())` only if you need the build green before Step 5; otherwise go straight on.

- [ ] **Step 4: Failing model test** — in `src/tui/model.rs` tests, using the `model(today)` helper:

```rust
#[test]
fn backup_key_sends_the_backup_command() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::Backup);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::Backup));
}
```
Run: `nix develop --command cargo test --lib tui::model::tests::backup_key` — expect it to fail (or the crate not to compile) until the arm exists; then pass.

- [ ] **Step 5: Failing worker test** — in `src/tui/worker.rs` tests, using the existing `ctx(home)` helper and `tempfile::tempdir()`:

```rust
#[test]
fn backup_writes_a_file_under_home_backups() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let reply = handle(&ctx, StoreCmd::Backup).unwrap();
    let StoreReply::Changed(msg) = reply else { panic!("expected Changed, got {reply:?}") };
    assert!(msg.starts_with("Backup written to "), "{msg}");
    let files: Vec<_> = std::fs::read_dir(home.path().join("backups")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with("tk-") && files[0].ends_with(".db"), "{files:?}");
    assert!(msg.ends_with(&files[0]), "{msg}");
}
```
Run: `nix develop --command cargo test --lib tui::worker::tests::backup_writes` — expect failure: no `Backup` arm (non-exhaustive match compile error) or wrong reply.

- [ ] **Step 6: Extract the helper and add the worker arm**

`src/cli/commands.rs`: add above `run` (or next to `now_local`):
```rust
/// Copy the database to `<home>/backups/tk-YYYYmmdd-HHMMSS.db` and return the path.
pub fn write_backup(ctx: &Ctx, now: NaiveDateTime) -> anyhow::Result<PathBuf> {
    let dir = ctx.home.join("backups");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("tk-{}.db", now.format("%Y%m%d-%H%M%S")));
    ctx.store.backup_to(&path)?;
    Ok(path)
}
```
and make `Command::Backup` use it:
```rust
Command::Backup => {
    let path = write_backup(ctx, now)?;
    writeln!(out, "Backup written to {}", path.display())?;
}
```
`src/tui/worker.rs` `handle`, appended arm:
```rust
StoreCmd::Backup => {
    let path = crate::cli::commands::write_backup(ctx, crate::cli::commands::now_local())?;
    StoreReply::Changed(format!("Backup written to {}", path.display()))
}
```
(`now_local` already exists at `src/cli/commands.rs:13`.)

- [ ] **Step 7: Run the three focused tests, then the full suite, fmt, clippy**
`nix develop --command cargo test` — all green, `tests/cli.rs` backup test unchanged and passing.

- [ ] **Step 8: README**
  - Month view table (README.md:250-269): add a row `| \`B\` | Write a backup of the database to \`<home>/backups/\` (same as \`tk backup\`) |` after the `u` row.
  - CLI table row for `tk backup` (README.md:203): append "`B` in the TUI does the same." to its description.
  - "Moving your data around" (README.md:446+): after the `tk backup` code line mention `B` in one sentence.

- [ ] **Step 9: Commit**
```bash
git add -A
git commit -m "feat(tui): B writes a backup from the month view"
```

---

### Task 2: `P` opens a projects screen to archive, unarchive, rename and add projects

**Files:**
- Create: `src/tui/components/projects.rs`, `src/tui/components/text_prompt.rs`, `src/tui/view/projects.rs`
- Modify: `src/tui/components/mod.rs` (declare the two modules), `src/tui/view/mod.rs` (declare `projects`)
- Modify: `src/tui/msg.rs`, `src/tui/ids.rs`, `src/tui/mod.rs`, `src/tui/model.rs`, `src/tui/worker.rs`, `src/tui/components/month.rs`
- Modify: `tests/snapshots.rs` (add `Screen::Projects` to the hint-row loop)
- Modify: `README.md`

**Interfaces:**
- Consumes (already exist): `Store::list_projects(include_archived: bool) -> StoreResult<Vec<Project>>`, `Store::add_project(&str) -> StoreResult<Project>`, `Store::archive_project(&str, archived: bool) -> StoreResult<()>`, `Store::rename_project(old: &str, new: &str) -> StoreResult<()>` (refuses when `new` exists, `NotFound` when `old` does not), `Project { id: i64, name: String, color_index: u8, archived: bool }`, `Store::days_in(from, to, &HolidayCalendar) -> StoreResult<Vec<Day>>`, `core::day_stats(&Day, &Rules, &HolidayCalendar, &TodayCtx) -> DayStats` (with `entry_nets: Vec<Minutes>` parallel to `day.entries`), `FieldForm` / `FieldSpec` / `FieldFormEvent` in `src/tui/components/field_form.rs`, the `ProjectPicker` mount/umount pattern in `src/tui/model.rs::open_clock_picker` / `close_clock_picker` (model.rs:611-629).
- Produces: `Screen::Projects`, `Id::Projects`, `Id::Prompt`, `ProjectsData`, the `Msg`/`StoreCmd`/`StoreReply` variants below.

**Behaviour (the approved design):**
- `P` on the month screen opens a full screen titled `Projects`: one row per project, archived ones included, sorted active first then by name (case-insensitive). Columns: `PROJECT` (name, bold in its project color, muted when archived), `NET` (total net minutes worked on it from `start_date` to today, `fmt_unsigned` in the current hours format), `STATUS` (`archived` in muted text, or empty). The cursor row gets `▶` in the first column and the `bg_selected` background, like the month table. While data is loading the body says `loading…`. A line under the table reads `N projects · M archived`.
- Keys on that screen: `↑`/`k`, `↓`/`j` move; `a` archives the highlighted project, or unarchives it if it is archived; `r` opens a name box prefilled with the current name; `n` opens an empty name box for a new project; `u` toggles hours format; `?` help; `Esc`/`q` back to the month view (which reloads, because names may have changed). Ctrl+C quits.
- The name box is a small centered one-field overlay (title `Rename project` or `New project`, field label `Name`). `Enter` submits, `Esc` cancels. A blank name is refused with the status `Name must not be empty` and the box stays open. Renaming to the same name just closes the box. Store errors (`project 'X' already exists`) arrive as `Failed` and show in the status bar.
- After archive / rename / add, the screen reloads itself and the cursor stays on the same row index (clamped).
- The month screen's project picker already reads active projects only, so an archived project disappears from it on the next `i`.

- [ ] **Step 1: Messages, data and ids**

`src/tui/msg.rs`, appended:
```rust
// --- projects screen ---
/// Everything the projects screen shows: every project, archived ones
/// included, and the net minutes worked on each from `start_date` to today.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectsData {
    pub projects: Vec<Project>,
    pub net_by_project: BTreeMap<String, Minutes>,
}
```
end of `Msg`:
```rust
    // --- projects screen ---
    OpenProjects,
    ProjectsSelect(i32),
    /// `a`: archive the highlighted project, or unarchive it if it is archived.
    ProjectsToggleArchive,
    /// `r`: open the name box prefilled with the highlighted project's name.
    ProjectsRename,
    /// `n`: open an empty name box.
    ProjectsAdd,
    PromptChanged,
    PromptSubmit(String),
    PromptCancel,
```
end of `StoreCmd`:
```rust
    // --- projects screen ---
    LoadProjects,
    ArchiveProject { name: String, archived: bool },
    RenameProject { old: String, new: String },
    AddProject(String),
```
end of `StoreReply`:
```rust
    // --- projects screen ---
    Projects(ProjectsData),
```
`src/tui/ids.rs`: append `Projects,` and `Prompt,`.
`src/tui/model.rs`: append `Projects,` to `Screen`; add fields `pub projects: Option<ProjectsData>`, `pub projects_cursor: usize`, `pub prompt: Option<PromptKind>` (initialise them where the other fields are, in `Model::new` and in `testing::model`), and
```rust
/// What the one-field name box is for.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptKind {
    RenameProject { old: String },
    AddProject,
}
```
Build will fail on non-exhaustive matches; that is expected until Step 5.

- [ ] **Step 2: Failing component tests** — create `src/tui/components/projects.rs` with the tests first (module skeleton copied from `src/tui/components/stats.rs`: `#[derive(Default, Component)] pub struct ProjectsScreen { component: KeyOnly }` and `impl AppComponent<Msg, UserEvent>`):

```rust
#[test]
fn letters_map_to_project_actions() {
    let mut c = ProjectsScreen::default();
    let press = |c: &mut ProjectsScreen, ch| c.on(&Event::Keyboard(KeyEvent::new(Key::Char(ch), KeyModifiers::NONE)));
    assert_eq!(press(&mut c, 'a'), Some(Msg::ProjectsToggleArchive));
    assert_eq!(press(&mut c, 'r'), Some(Msg::ProjectsRename));
    assert_eq!(press(&mut c, 'n'), Some(Msg::ProjectsAdd));
    assert_eq!(press(&mut c, 'j'), Some(Msg::ProjectsSelect(1)));
    assert_eq!(press(&mut c, 'k'), Some(Msg::ProjectsSelect(-1)));
    assert_eq!(press(&mut c, 'u'), Some(Msg::ToggleHours));
    assert_eq!(press(&mut c, 'q'), Some(Msg::Back));
    assert_eq!(press(&mut c, 'z'), None);
}
```
and in `src/tui/components/month.rs` tests:
```rust
#[test]
fn shift_p_opens_the_projects_screen() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('P'), KeyModifiers::SHIFT));
    assert_eq!(c.on(&ev), Some(Msg::OpenProjects));
}
```
Declare `pub mod projects;` and `pub mod text_prompt;` in `src/tui/components/mod.rs`. Run `nix develop --command cargo test --lib tui::components` — expect compile failures / assertion failures.

- [ ] **Step 3: Implement the key maps**

`ProjectsScreen::on`: Ctrl+C → `Msg::Quit`; `Up`/`k` → `ProjectsSelect(-1)`; `Down`/`j` → `ProjectsSelect(1)`; `a` → `ProjectsToggleArchive`; `r` → `ProjectsRename`; `n` → `ProjectsAdd`; `u` → `ToggleHours`; `?` → `ToggleHelp`; `Esc`/`q` → `Back`; otherwise `None`. Month: `Key::Char('P') => Msg::OpenProjects,`.

- [ ] **Step 4: The name box component** — `src/tui/components/text_prompt.rs`, modelled on `src/tui/components/project_picker.rs` minus the filter list:

```rust
//! One-field name box: rename a project, or name a new one.

pub struct TextPrompt { title: String, form: FieldForm }

impl TextPrompt {
    pub fn new(title: String, label: &str, initial: &str) -> Self {
        Self { title, form: FieldForm::new(&[FieldSpec::new(label, "")], &[initial.to_string()]) }
    }
    pub fn with_theme(mut self, t: &Theme) -> Self { self.form = self.form.with_theme(t); self }
}
```
`MockComponent::view` draws `self.form.view(f, area, &self.title, None)`; `AppComponent::on` maps `FieldFormEvent::Submit` → `Msg::PromptSubmit(self.form.value(0).to_string())`, `Cancel` → `Msg::PromptCancel`, `Changed` → `Msg::PromptChanged`, `Quit` → `Msg::Quit`, `Ignored` → `None`. Copy the exact `MockComponent` boilerplate (`query`, `attr`, `state`, `perform`) from `ProjectPicker`. Check `FieldSpec::new`'s real signature in `field_form.rs` before using it. Add one test: Enter emits `PromptSubmit` with the typed text; Esc emits `PromptCancel`.

- [ ] **Step 5: Failing model tests** — in `src/tui/model.rs` tests:

```rust
fn projects_data() -> ProjectsData {
    let p = |id, name: &str, archived| Project { id, name: name.into(), color_index: 0, archived };
    ProjectsData {
        projects: vec![p(1, "Alpha", false), p(2, "Old", true)],
        net_by_project: BTreeMap::from([("Alpha".to_string(), Minutes(600))]),
    }
}

#[test]
fn shift_p_opens_the_projects_screen_and_loads_it() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::OpenProjects);
    assert_eq!(m.screen, Screen::Projects);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadProjects));
}

#[test]
fn a_flips_the_archived_flag_of_the_highlighted_project() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::OpenProjects);
    let _ = rx.try_recv();
    m.on_store(StoreReply::Projects(projects_data()));
    m.update(Msg::ProjectsSelect(1));
    m.update(Msg::ProjectsToggleArchive);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ArchiveProject { name, archived: false } if name == "Old"));
}

#[test]
fn r_renames_through_the_name_box_and_blank_names_are_refused() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::OpenProjects);
    let _ = rx.try_recv();
    m.on_store(StoreReply::Projects(projects_data()));
    m.update(Msg::ProjectsRename);
    assert_eq!(m.prompt, Some(PromptKind::RenameProject { old: "Alpha".into() }));
    m.update(Msg::PromptSubmit("   ".into()));
    assert!(m.prompt.is_some(), "blank name keeps the box open");
    assert!(m.status.as_ref().is_some_and(|s| s.0.contains("must not be empty")));
    m.update(Msg::PromptSubmit("Alpha2".into()));
    assert_eq!(m.prompt, None);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::RenameProject { old, new } if old == "Alpha" && new == "Alpha2"));
}

#[test]
fn n_adds_a_project_through_the_name_box() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::OpenProjects);
    let _ = rx.try_recv();
    m.update(Msg::ProjectsAdd);
    assert_eq!(m.prompt, Some(PromptKind::AddProject));
    m.update(Msg::PromptSubmit("Gamma".into()));
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::AddProject(n) if n == "Gamma"));
}

#[test]
fn leaving_the_projects_screen_reloads_the_month() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.update(Msg::OpenProjects);
    let _ = rx.try_recv();
    m.update(Msg::Back);
    assert_eq!(m.screen, Screen::Month);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadMonth { .. }));
}
```
Adjust the status assertion to however `Model` stores its status (look at `set_status`; if the field is private, add a `#[cfg(test)]` accessor or assert through an existing pattern in the tests). Run: `nix develop --command cargo test --lib tui::model::tests` — expect failures.

- [ ] **Step 6: Model handlers**

In `update`, appended arms:
```rust
Msg::OpenProjects => {
    self.screen = Screen::Projects;
    self.projects = None;
    self.projects_cursor = 0;
    self.focus(Id::Projects);
    self.send(StoreCmd::LoadProjects);
}
Msg::ProjectsSelect(n) => {
    let len = self.projects.as_ref().map_or(0, |p| p.projects.len());
    if len > 0 {
        self.projects_cursor = (self.projects_cursor as i32 + n).clamp(0, len as i32 - 1) as usize;
    }
}
Msg::ProjectsToggleArchive => {
    if let Some(p) = self.highlighted_project().cloned() {
        self.send(StoreCmd::ArchiveProject { name: p.name, archived: !p.archived });
    }
}
Msg::ProjectsRename => {
    if let Some(p) = self.highlighted_project().cloned() {
        self.open_prompt("Rename project".into(), &p.name, PromptKind::RenameProject { old: p.name.clone() });
    }
}
Msg::ProjectsAdd => self.open_prompt("New project".into(), "", PromptKind::AddProject),
Msg::PromptChanged => {}
Msg::PromptSubmit(text) => {
    let name = text.trim().to_string();
    if name.is_empty() {
        self.set_status("Name must not be empty", true);
        return;
    }
    let Some(kind) = self.prompt.clone() else { return };
    self.close_prompt();
    match kind {
        PromptKind::RenameProject { old } if old == name => {}
        PromptKind::RenameProject { old } => self.send(StoreCmd::RenameProject { old, new: name }),
        PromptKind::AddProject => self.send(StoreCmd::AddProject(name)),
    }
}
Msg::PromptCancel => self.close_prompt(),
```
Helpers next to `open_clock_picker`:
```rust
fn highlighted_project(&self) -> Option<&Project> {
    self.projects.as_ref()?.projects.get(self.projects_cursor)
}
fn open_prompt(&mut self, title: String, initial: &str, kind: PromptKind) {
    let _ = self.app.umount(&Id::Prompt);
    let _ = self.app.mount(
        Id::Prompt,
        Box::new(components::text_prompt::TextPrompt::new(title, "Name", initial).with_theme(&self.theme)),
        vec![],
    );
    self.prompt = Some(kind);
    self.focus(Id::Prompt);
}
fn close_prompt(&mut self) {
    self.prompt = None;
    let _ = self.app.umount(&Id::Prompt);
    self.focus_screen();
}
```
Other places in `model.rs` that need a `Projects` arm or line:
- `on_store`: `StoreReply::Projects(d) => { self.projects_cursor = self.projects_cursor.min(d.projects.len().saturating_sub(1)); self.projects = Some(d); }`
- `reload_after_write`: add `if self.screen == Screen::Projects { self.send(StoreCmd::LoadProjects); }`
- `Msg::Back` (model.rs:759-769): reload the month when coming back from Day **or** Projects: `let reload = matches!(self.screen, Screen::Day | Screen::Projects);`
- `focus_screen`: `Screen::Projects => Id::Projects`
- `draw`: `Screen::Projects => super::view::projects::draw(self, f, body)`
- `view`: draw the prompt overlay like the picker: `let prompt_open = self.prompt.is_some();` … `if prompt_open { self.app.view(&Id::Prompt, f, area); }`
- `key_hints`: `Screen::Projects => &[("↑↓", "project"), ("a", "archive"), ("r", "rename"), ("n", "new"), ("u", "units"), ("?", "help"), ("Esc", "back")]`
- `help_keys`: `Screen::Projects => &[("↑ ↓ j k", "select project"), ("a", "archive / unarchive"), ("r", "rename"), ("n", "new project"), ("u", "toggle h:mm / decimal hours"), ("Esc", "back")]`; and in the **Month** help list add `("P", "projects: archive, rename, add")` after the `("c", "settings")` line.
- Any `match self.screen` elsewhere (search for `Screen::Stats =>`) gets a `Projects` arm that behaves like `Stats` unless the text above says otherwise.
- `src/tui/mod.rs::run`: mount `Id::Projects` with `components::projects::ProjectsScreen::default()` next to the `Stats` mount.

Sort order: the worker returns projects sorted; the model does not sort.

- [ ] **Step 7: Failing worker tests** — `src/tui/worker.rs` tests:

```rust
fn projects(ctx: &Ctx) -> ProjectsData {
    match handle(ctx, StoreCmd::LoadProjects).unwrap() {
        StoreReply::Projects(p) => p,
        other => panic!("expected Projects, got {other:?}"),
    }
}

#[test]
fn load_projects_lists_archived_ones_and_sums_net_minutes() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    ctx.store.add_project("Beta").unwrap();
    ctx.store.add_project("Alpha").unwrap();
    ctx.store.add_project("Old").unwrap();
    ctx.store.archive_project("Old", true).unwrap();
    // 08:00–12:00 on Alpha: four hours, one session, 18 minutes of break.
    handle(&ctx, StoreCmd::AddEntry { date: d(2026, 9, 14), start: NaiveTime::from_hms_opt(8, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(12, 0, 0).unwrap(), project: "Alpha".into(), comment: String::new() }).unwrap();
    let p = projects(&ctx);
    let names: Vec<&str> = p.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "Beta", "Old"], "active first, then by name");
    assert!(p.projects[2].archived);
    assert_eq!(p.net_by_project.get("Alpha"), Some(&Minutes(222)));
    assert_eq!(p.net_by_project.get("Beta"), None);
}

#[test]
fn archive_rename_and_add_reply_with_a_status_line() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    ctx.store.add_project("Alpha").unwrap();
    assert_eq!(handle(&ctx, StoreCmd::ArchiveProject { name: "Alpha".into(), archived: true }).unwrap(), StoreReply::Changed("Alpha archived".into()));
    assert!(ctx.store.project_by_name("Alpha").unwrap().unwrap().archived);
    assert_eq!(handle(&ctx, StoreCmd::ArchiveProject { name: "Alpha".into(), archived: false }).unwrap(), StoreReply::Changed("Alpha unarchived".into()));
    assert_eq!(handle(&ctx, StoreCmd::RenameProject { old: "Alpha".into(), new: "Alef".into() }).unwrap(), StoreReply::Changed("Alpha renamed to Alef".into()));
    assert_eq!(handle(&ctx, StoreCmd::AddProject("Gamma".into())).unwrap(), StoreReply::Changed("Gamma added".into()));
    assert!(handle(&ctx, StoreCmd::AddProject("Gamma".into())).is_err(), "duplicates bubble up as errors");
}
```
(`AddEntry`'s exact field list is in `msg.rs:146-152`; the `ctx()` helper's `start_date` is 2025-01-01 and `today` inside `handle` comes from the real clock, so keep the entry date in the past.) If `StoreReply` does not derive `PartialEq`, match with `matches!` instead. Run: `nix develop --command cargo test --lib tui::worker::tests` — expect failures.

- [ ] **Step 8: Worker arms**

```rust
StoreCmd::LoadProjects => {
    let mut projects = ctx.store.list_projects(true)?;
    projects.sort_by(|a, b| a.archived.cmp(&b.archived).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let cal = ctx.config.calendar();
    let clocked_in = ctx.store.session()?.is_some();
    let mut net_by_project: BTreeMap<String, Minutes> = BTreeMap::new();
    for day in ctx.store.days_in(ctx.config.start_date, today, &cal)? {
        let s = day_stats(&day, &rules, &cal, &TodayCtx { today, clocked_in });
        for (idx, e) in day.entries.iter().enumerate() {
            *net_by_project.entry(e.project.clone()).or_default() += s.entry_nets[idx];
        }
    }
    StoreReply::Projects(ProjectsData { projects, net_by_project })
}
StoreCmd::ArchiveProject { name, archived } => {
    ctx.store.archive_project(&name, archived)?;
    StoreReply::Changed(format!("{name} {}", if archived { "archived" } else { "unarchived" }))
}
StoreCmd::RenameProject { old, new } => {
    ctx.store.rename_project(&old, &new)?;
    StoreReply::Changed(format!("{old} renamed to {new}"))
}
StoreCmd::AddProject(name) => {
    ctx.store.add_project(&name)?;
    StoreReply::Changed(format!("{name} added"))
}
```
`today` and `rules` are built the same way `handle`'s `LoadMonth`/`LoadStats` arms and `balance_through` build them — read those first and reuse the same expressions (do not invent a second way of building `Rules`).

- [ ] **Step 9: Failing view test** — create `src/tui/view/projects.rs` with `pub fn draw(m: &Model, f: &mut Frame, area: Rect)` and a pure `pub fn draw_table(f, area, t: &Theme, data: &ProjectsData, cursor: usize, fmt: HoursFormat)`, test first (helpers `render`, `contains`, `style_of` in `crate::tui::view::testing`):

```rust
#[test]
fn renders_projects_with_hours_cursor_and_archived_marker() {
    let t = Theme::dark();
    let data = ProjectsData { projects: vec![
        Project { id: 1, name: "Alpha".into(), color_index: 0, archived: false },
        Project { id: 2, name: "Old".into(), color_index: 1, archived: true },
    ], net_by_project: BTreeMap::from([("Alpha".to_string(), Minutes(222))]) };
    let rows = render(80, 20, |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm));
    assert!(contains(&rows, "PROJECT"));
    assert!(contains(&rows, "03:42"));
    assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Old") && r.contains("archived")));
    assert!(contains(&rows, "2 projects · 1 archived"));
    let alpha = style_of(80, 20, |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm), "Alpha");
    assert_eq!(alpha.fg, Some(t.project_color(0)));
    assert!(alpha.add_modifier.contains(Modifier::BOLD));
    let old = style_of(80, 20, |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm), "Old");
    assert_eq!(old.fg, Some(t.muted));
}
```
Run and see it fail, then implement the drawing: a `Table` inside `block(t, Some("Projects"))` with header cells `PROJECT`, `NET`, `STATUS` in `t.muted` bold (as the month table does), widths `Min(10)`, `Length(9)`, `Length(10)`; cursor row styled `bg(t.bg_selected)` with `▶` in `t.accent`; a one-line `Paragraph` under the table with `N projects · M archived` in `t.muted`. `draw(m, ..)` shows `loading…` in `t.muted` when `m.projects` is `None`, otherwise calls `draw_table` with `m.projects_cursor` and `m.hours`.

- [ ] **Step 10: Hint-row snapshot test** — in `tests/snapshots.rs::key_hints_fit_the_minimum_terminal` add `Screen::Projects` to the `for screen in [...]` array (its way out is `Esc back`, already the default branch). Run `nix develop --command cargo test --test snapshots`.

- [ ] **Step 11: Full suite, fmt, clippy** — `nix develop --command cargo test`, then fmt and clippy per Global Constraints.

- [ ] **Step 12: README**
  - Month view table: row `| \`P\` | Projects: archive, unarchive, rename, add |` after the `s` row.
  - New section `### Projects` between `### Statistics` and `### Overlays` with a key table (`↑`/`k` `↓`/`j` move · `a` archive / unarchive · `r` rename · `n` new project · `u` units · `?` help · `Esc`, `q` back) and one paragraph: what the columns mean (net hours since `start_date`), that archiving only hides a project from the picker and keeps its entries, that renaming follows every entry and refuses a name that already exists, and that a blank name is refused.
  - Overlays table: row `| Name box (\`r\` / \`n\` on the projects screen) | \`Enter\` saves; \`Esc\` cancels |`.
  - CLI table rows `tk projects add|archive|unarchive|rename` (README.md:198-201): append "In the TUI: `P`." to the `tk projects list` row's description only, once.

- [ ] **Step 13: Commit** (one or several commits along the way are fine; the last one):
```bash
git add -A
git commit -m "feat(tui): a projects screen on P to archive, rename and add projects"
```

---

### Task 3: `V` anchors a range so one day-type key marks every weekday in it

**Files:**
- Modify: `src/tui/msg.rs` (`Msg::ToggleAnchor`, `StoreCmd::SetKindRange`, `Confirm::SetKindRange`)
- Modify: `src/tui/components/month.rs` (key + test)
- Modify: `src/tui/model.rs` (field `anchor`, handlers, `confirm_text`, `help_keys`, tests)
- Modify: `src/tui/view/month.rs` (`draw_table` gains a `range` parameter; highlight; tests)
- Modify: `src/tui/worker.rs` (arm + tests)
- Modify: `README.md`

**Interfaces:**
- Consumes: `core::is_working_day(NaiveDate) -> bool`, `Store::set_day_kind(NaiveDate, &DayKind)` (deletes the row for `Work`, refuses a non-work kind on a day with entries), `Store::days_in(from, to, &cal)`, `Store::stored_kinds_in(from, to)` (see `src/store/days.rs:53` for its return type), the existing `Confirm::SetKind` flow in `model.rs:839-871`.
- Produces: `draw_table(..., selected: NaiveDate, range: Option<(NaiveDate, NaiveDate)>, today: NaiveDate, ...)` — the new parameter sits right after `selected`.

**Behaviour (the approved design):**
- `V` on the month screen drops an anchor on the selected day and shows the status `Range from Mon 14 Sep: v f x p w mark every weekday up to the cursor, V or Esc clears`. Pressing `V` again, or `Esc` (with no overlay open), clears it with the status `Range cleared`. Moving the cursor, changing month and `t` keep the anchor; the range is always anchor..cursor in either order.
- While an anchor is set, every row whose date lies between anchor and cursor (inclusive) is drawn with the `bg_selected` background; the cursor row keeps its `▶`. Week-footer rows are never highlighted.
- With an anchor set, `v` `f` `x` `p` `w` open the confirm `Set 5 weekdays, 2026-09-14 to 2026-09-18, to vacation?` (count = weekdays in the range per `is_working_day`). `y`/`Enter` sends one `StoreCmd::SetKindRange` and clears the anchor; `n`/`Esc` keeps the anchor. The single-day pre-checks (entries, weekend) are **not** run in the model for a range: the worker checks.
- The worker collects the weekdays in the range (weekends skipped: they carry no target, same rule as the single-day key). No weekdays at all → error `No weekdays between 2026-09-05 and 2026-09-06`. For a non-work kind, if any of those days has entries → error `2026-09-15 has entries; delete them first` **and nothing is changed**. Otherwise every weekday is set and the reply is `Changed("5 weekdays set to vacation")`. `w` resets the range to work days the same way (`set_day_kind` with `DayKind::Work` on each).

- [ ] **Step 1: Failing component test** — `src/tui/components/month.rs` tests:
```rust
#[test]
fn shift_v_toggles_the_range_anchor() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('V'), KeyModifiers::SHIFT));
    assert_eq!(c.on(&ev), Some(Msg::ToggleAnchor));
}
```
Run `nix develop --command cargo test --lib tui::components::month` — compile error expected.

- [ ] **Step 2: Variants and key**

`msg.rs`, end of `Msg`: `// --- range marking ---` then `/// \`V\`: drop or clear the anchor of a date range for the day-type keys.` `ToggleAnchor,`. End of `StoreCmd`: `// --- range marking ---` `SetKindRange { from: NaiveDate, to: NaiveDate, kind: DayKind },`. End of `Confirm`: `// --- range marking ---` `SetKindRange { from: NaiveDate, to: NaiveDate, kind: DayKind },`. Month component: `Key::Char('V') => Msg::ToggleAnchor,`.

- [ ] **Step 3: Failing model tests** — `src/tui/model.rs` tests (reuse the `model`/`month_data` helpers and the style of `model.rs:1574-1592`):

```rust
fn work_days(from: NaiveDate, to: NaiveDate) -> Vec<Day> {
    from.iter_days().take_while(|d| *d <= to).map(|date| Day { date, kind: DayKind::Work, entries: vec![] }).collect()
}

#[test]
fn v_drops_and_clears_the_anchor() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, _rx) = model(today);
    m.month = Some(month_data(today, work_days(d(2026, 9, 1), d(2026, 9, 30)), None));
    m.update(Msg::ToggleAnchor);
    assert_eq!(m.anchor, Some(today));
    m.update(Msg::SelectDay(3));
    assert_eq!(m.anchor, Some(today), "moving keeps the anchor");
    m.update(Msg::ToggleAnchor);
    assert_eq!(m.anchor, None);
}

#[test]
fn a_day_type_key_with_an_anchor_confirms_the_whole_range_in_date_order() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();
    let (mut m, rx) = model(today);
    m.month = Some(month_data(today, work_days(d(2026, 9, 1), d(2026, 9, 30)), None));
    m.update(Msg::ToggleAnchor);            // anchor Fri 18
    m.update(Msg::SelectDay(-4));           // cursor Mon 14
    m.update(Msg::SetKind(DayKind::Vacation));
    assert_eq!(m.confirm, Some(Confirm::SetKindRange { from: d(2026, 9, 14), to: d(2026, 9, 18), kind: DayKind::Vacation }));
    assert_eq!(m.confirm_text(m.confirm.as_ref().unwrap()), "Set 5 weekdays, 2026-09-14 to 2026-09-18, to vacation?");
    m.update(Msg::ConfirmNo);
    assert_eq!(m.anchor, Some(d(2026, 9, 18)), "declining keeps the anchor");
    m.update(Msg::SetKind(DayKind::Vacation));
    m.update(Msg::ConfirmYes);
    assert!(matches!(rx.try_recv().unwrap(), StoreCmd::SetKindRange { from, to, kind: DayKind::Vacation } if from == d(2026, 9, 14) && to == d(2026, 9, 18)));
    assert_eq!(m.anchor, None, "applying clears the anchor");
}

#[test]
fn esc_clears_the_anchor_before_anything_else() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, _rx) = model(today);
    m.month = Some(month_data(today, work_days(d(2026, 9, 1), d(2026, 9, 30)), None));
    m.update(Msg::ToggleAnchor);
    m.update(Msg::Back);
    assert_eq!(m.anchor, None);
    assert_eq!(m.screen, Screen::Month);
}
```
(`d(y, m, dd)` — add the same tiny helper the other test modules use if the model tests lack one; `SelectDay` may already be clamped to the loaded month, which is fine for these dates.) Run `nix develop --command cargo test --lib tui::model::tests` — expect failures.

- [ ] **Step 4: Model changes**

- Field `pub anchor: Option<NaiveDate>` (initialised `None` in `Model::new` and `testing::model`).
- `update` arms:
```rust
Msg::ToggleAnchor => {
    if self.anchor.take().is_some() {
        self.set_status("Range cleared", false);
    } else {
        self.anchor = Some(self.selected);
        self.set_status(
            format!("Range from {}: v f x p w mark every weekday up to the cursor, V or Esc clears", self.selected.format("%a %d %b")),
            false,
        );
    }
}
```
- `Msg::SetKind(kind)` (model.rs:839-855): at the very top, before the entries/weekend checks:
```rust
if let Some(a) = self.anchor {
    let (from, to) = (a.min(self.selected), a.max(self.selected));
    self.open_confirm(Confirm::SetKindRange { from, to, kind });
    return;
}
```
- `Msg::ConfirmYes` match: `Confirm::SetKindRange { from, to, kind } => { self.anchor = None; self.send(StoreCmd::SetKindRange { from, to, kind }); }`
- `Msg::Back`: first branch stays (`help`/`confirm` open → `close_overlay`); add `else if self.screen == Screen::Month && self.anchor.is_some() { self.anchor = None; self.set_status("Range cleared", false); }` before the existing `else`.
- `confirm_text`: `Confirm::SetKindRange { from, to, kind } => { let n = from.iter_days().take_while(|d| d <= to).filter(|d| crate::core::is_working_day(*d)).count(); format!("Set {n} weekdays, {from} to {to}, to {}?", kind.display_name().to_lowercase()) }`
- `help_keys` Month list: add `("V", "start / clear a range for the day-type keys")` after the `("w", ...)` line.
- `view::month::draw` passes `m.anchor.map(|a| (a.min(m.selected), a.max(m.selected)))` as the new `range` argument (Step 6).

- [ ] **Step 5: Failing view test** — `src/tui/view/month.rs` tests (use the existing `fixture()`, `render`, `style_of`):

```rust
#[test]
fn rows_between_anchor_and_cursor_share_the_selection_background() {
    let (data, rules, cal) = fixture();
    let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
    let t = Theme::dark();
    let draw = |f: &mut Frame| {
        draw_table(f, f.area(), &t, &v, &data, d(2026, 9, 10), Some((d(2026, 9, 8), d(2026, 9, 10))), d(2026, 9, 15), true, HoursFormat::Hm)
    };
    assert_eq!(style_of(100, 40, draw, "Tue 08").bg, Some(t.bg_selected));
    assert_eq!(style_of(100, 40, draw, "Wed 09").bg, Some(t.bg_selected));
    assert_eq!(style_of(100, 40, draw, "Thu 10").bg, Some(t.bg_selected));
    assert_ne!(style_of(100, 40, draw, "Fri 11").bg, Some(t.bg_selected));
    assert_ne!(style_of(100, 40, draw, "Mon 07").bg, Some(t.bg_selected));
    let rows = render(100, 40, draw);
    assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Thu 10")), "cursor keeps its mark");
    assert!(!rows.iter().any(|r| r.contains('▶') && r.contains("Tue 08")));
}
```
Run — compile error (arity). Then:

- [ ] **Step 6: `draw_table` range parameter**

Add `range: Option<(NaiveDate, NaiveDate)>` right after `selected` in `draw_table`'s signature (month.rs:291-300); update `draw()` and **every** existing test call site to pass `None` (there are several in `month.rs` tests and possibly in `tests/snapshots.rs` — `grep -n "draw_table(" src tests`). In the row loop, after `let sel = row_is_selected(v, r, selected);` add
```rust
let in_range = range.is_some_and(|(from, to)| {
    !matches!(r.kind, RowKind::WeekFooter { .. }) && r.date >= from && r.date <= to
});
```
and change the styling at the bottom to `if sel || in_range { row = row.style(Style::default().bg(t.bg_selected)); }`. The `▶` mark stays tied to `sel` only. Note the collapsed weekend row carries the Saturday's date; that is fine.

- [ ] **Step 7: Failing worker tests** — `src/tui/worker.rs` tests:

```rust
#[test]
fn set_kind_range_marks_the_weekdays_and_skips_the_weekend() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let reply = handle(&ctx, StoreCmd::SetKindRange { from: d(2026, 9, 11), to: d(2026, 9, 15), kind: DayKind::Vacation }).unwrap();
    assert_eq!(reply, StoreReply::Changed("3 weekdays set to vacation".into()));
    let days = ctx.store.days_in(d(2026, 9, 11), d(2026, 9, 15), &ctx.config.calendar()).unwrap();
    let kinds: Vec<DayKind> = days.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(kinds, vec![DayKind::Vacation, DayKind::Work, DayKind::Work, DayKind::Vacation, DayKind::Vacation]);
}

#[test]
fn set_kind_range_refuses_when_a_day_has_entries_and_changes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    handle(&ctx, StoreCmd::AddEntry { date: d(2026, 9, 15), start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(), end: NaiveTime::from_hms_opt(10, 0, 0).unwrap(), project: "Alpha".into(), comment: String::new() }).unwrap();
    let err = handle(&ctx, StoreCmd::SetKindRange { from: d(2026, 9, 14), to: d(2026, 9, 16), kind: DayKind::Sick }).unwrap_err();
    assert_eq!(err.to_string(), "2026-09-15 has entries; delete them first");
    let days = ctx.store.days_in(d(2026, 9, 14), d(2026, 9, 16), &ctx.config.calendar()).unwrap();
    assert!(days.iter().all(|d| d.kind == DayKind::Work), "nothing was changed: {days:?}");
}

#[test]
fn set_kind_range_over_a_weekend_only_is_an_error() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let err = handle(&ctx, StoreCmd::SetKindRange { from: d(2026, 9, 5), to: d(2026, 9, 6), kind: DayKind::Flex }).unwrap_err();
    assert_eq!(err.to_string(), "No weekdays between 2026-09-05 and 2026-09-06");
}
```
(If a weekend `Day` from `days_in` is not reported as `Work`, adjust the expected `kinds` vector to whatever the store reports for an unmarked Saturday/Sunday — check `days_in`; the point is that the two weekend days are untouched. If `StoreReply` lacks `PartialEq`, use `matches!`.) Run — non-exhaustive match compile error expected.

- [ ] **Step 8: Worker arm**

```rust
StoreCmd::SetKindRange { from, to, kind } => {
    let weekdays: Vec<NaiveDate> = from
        .iter_days()
        .take_while(|d| *d <= to)
        .filter(|d| is_working_day(*d))
        .collect();
    if weekdays.is_empty() {
        bail!("No weekdays between {from} and {to}");
    }
    if kind != DayKind::Work {
        let cal = ctx.config.calendar();
        for day in ctx.store.days_in(from, to, &cal)? {
            if weekdays.contains(&day.date) && !day.entries.is_empty() {
                bail!("{} has entries; delete them first", day.date);
            }
        }
    }
    for d in &weekdays {
        ctx.store.set_day_kind(*d, &kind)?;
    }
    StoreReply::Changed(format!("{} weekdays set to {}", weekdays.len(), kind.display_name().to_lowercase()))
}
```
Import `anyhow::bail` and `crate::core::is_working_day` if not already imported in `worker.rs`.

- [ ] **Step 9: Full suite, fmt, clippy** — `nix develop --command cargo test`; the snapshot tests must still pass (the hint row is unchanged).

- [ ] **Step 10: README**
  - Month view table: row `| \`V\` | Start or clear a range: the day-type keys then apply to every weekday from the anchor to the cursor |` after the `w` row.
  - After the paragraph beginning `Changing a day type asks for confirmation first` (README.md:270-272), add a paragraph: `V` drops an anchor on the selected day and the rows up to the cursor light up; `v` `f` `x` `p` `w` then ask once (`Set 5 weekdays, 2026-09-14 to 2026-09-18, to vacation?`) and mark every weekday in between, skipping weekends; a day with entries anywhere in the range refuses the whole range and changes nothing; `V` again or `Esc` clears the anchor; declining the confirm keeps it. Mention this is the TUI's equivalent of `tk day DATE KIND --to DATE`.
  - CLI table row `tk day` (README.md:194): append "In the TUI: `V` then a day-type key."

- [ ] **Step 11: Commit**
```bash
git add -A
git commit -m "feat(tui): V anchors a range so one day-type key marks every weekday in it"
```
