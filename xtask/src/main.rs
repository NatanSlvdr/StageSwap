use anyhow::{Context, Result, bail};
use dialoguer::console::{Key, Term};
use std::cell::RefCell;
#[cfg(test)]
use std::cmp::max;
use std::collections::VecDeque;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

const REQUIRED_WINDOWS_SDK: &str = "10.0.22621.0";
const APP_PACKAGE: &str = "stageswap";
const MEDIA_SOURCE_PACKAGE: &str = "stageswap-media-source";
const APP_EXECUTABLE: &str = "StageSwap.exe";
const MEDIA_SOURCE_DLL: &str = "stageswap_media_source.dll";
const RELEASE_PREFIX: &str = "StageSwap_win64_v";
#[cfg(test)]
const RELEASE_SUFFIX: &str = ".exe.sha256";

fn main() -> Result<()> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("validate-pe") => validate_pe(
            arguments.next().context("missing PE path")?,
            arguments.next().context("missing architecture")?,
        ),
        Some("publish-release") => {
            if arguments.next().is_some() {
                bail!("publish-release does not accept positional arguments");
            }
            publish_release_cli()
        }
        Some(command) => bail!("unknown xtask command: {command}"),
        None => bail!("usage: cargo xtask <validate-pe PATH x64 | publish-release>"),
    }
}

fn build_release(release_version: ReleaseVersion, progress: &ReleaseProgress) -> Result<PathBuf> {
    let workspace = workspace_root();
    let _windows_sdk = selected_windows_sdk()?;
    let architecture = "x64";
    let target = "x86_64-pc-windows-msvc";
    let manifest = workspace.join("Cargo.toml");
    let application_version = read_workspace_version(&manifest)?;
    if application_version != release_version {
        bail!(
            "workspace version {application_version} does not match release version {release_version}"
        );
    }
    let (_, executable) = build_release_pair(&workspace, target, architecture, progress)?;
    Ok(executable)
}

fn package_release(
    executable: &Path,
    release_version: ReleaseVersion,
    output: &Path,
    workspace: &Path,
) -> Result<PathBuf> {
    let windows_sdk = selected_windows_sdk()?;
    let architecture = "x64";
    let executable_bytes =
        fs::read(executable).with_context(|| format!("could not read {}", executable.display()))?;
    let digest = sha256(&executable_bytes);
    let artifact = format!("{RELEASE_PREFIX}{release_version}.exe");
    let revision = capture(git(workspace, &["rev-parse", "HEAD"]))?;
    let checksum = format!(
        "# applicationVersion={release_version}\n# releaseVersion={release_version}\n# sourceRevision={}\n# architecture={architecture}\n# configuration=Release\n# windowsSdk={windows_sdk}\n{} *{artifact}\n",
        revision.trim(),
        hex(&digest)
    );
    publish_release(output, &artifact, &executable_bytes, checksum.as_bytes())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
        .to_owned()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReleaseTrack {
    Development,
    Release,
}

const SUBSTEP_LIMIT: usize = 5;
const STAGE_COLUMN_WIDTH: usize = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReleaseStage {
    Infos,
    Checks,
    Preparation,
    Build,
    Publish,
}

impl ReleaseStage {
    const ALL: [Self; 5] = [
        Self::Infos,
        Self::Checks,
        Self::Preparation,
        Self::Build,
        Self::Publish,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Infos => "Infos",
            Self::Checks => "Checks",
            Self::Preparation => "Preparation",
            Self::Build => "Build",
            Self::Publish => "Publish",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Infos => 0,
            Self::Checks => 1,
            Self::Preparation => 2,
            Self::Build => 3,
            Self::Publish => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageStatus {
    Pending,
    Active,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubstepStatus {
    Active,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SubstepState {
    label: String,
    status: SubstepStatus,
    progress: Option<(usize, usize)>,
}

struct StageState {
    status: StageStatus,
    substeps: VecDeque<SubstepState>,
}

struct ReleaseProgressState {
    stages: [StageState; ReleaseStage::ALL.len()],
    current_stage: Option<ReleaseStage>,
    frame_lines: usize,
    prompt_lines: usize,
    prompt_active: bool,
}

impl ReleaseProgressState {
    fn new() -> Self {
        Self {
            stages: std::array::from_fn(|_| StageState {
                status: StageStatus::Pending,
                substeps: VecDeque::new(),
            }),
            current_stage: None,
            frame_lines: 0,
            prompt_lines: 0,
            prompt_active: false,
        }
    }
}

#[derive(Clone)]
struct RenderedRow {
    stage: Option<ReleaseStage>,
    stage_status: Option<StageStatus>,
    substep: Option<SubstepState>,
}

struct ReleaseProgress {
    interactive: bool,
    color: bool,
    terminal_renderer: bool,
    state: RefCell<ReleaseProgressState>,
}

impl ReleaseProgress {
    fn new() -> Self {
        let interactive = Term::stderr().is_term();
        let color = interactive && env::var_os("NO_COLOR").is_none();
        let progress = Self {
            interactive,
            color,
            terminal_renderer: interactive,
            state: RefCell::new(ReleaseProgressState::new()),
        };
        progress.refresh();
        progress
    }

    #[cfg(test)]
    fn for_test(interactive: bool, color: bool) -> Self {
        Self {
            interactive,
            color,
            terminal_renderer: false,
            state: RefCell::new(ReleaseProgressState::new()),
        }
    }

    fn step<T>(
        &self,
        stage: ReleaseStage,
        label: &str,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.begin_substep(stage, label, None);
        match action() {
            Ok(value) => {
                self.finish_substep(true);
                Ok(value)
            }
            Err(error) => {
                self.finish_substep(false);
                Err(error.context(format!("{label} failed")))
            }
        }
    }

    fn step_with_progress<T>(
        &self,
        stage: ReleaseStage,
        label: &str,
        progress: (usize, usize),
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.begin_substep(stage, label, Some(progress));
        match action() {
            Ok(value) => {
                self.finish_substep(true);
                Ok(value)
            }
            Err(error) => {
                self.finish_substep(false);
                Err(error.context(format!("{label} failed")))
            }
        }
    }

    fn command_counted(
        &self,
        stage: ReleaseStage,
        index: usize,
        total: usize,
        label: &str,
        command: Command,
    ) -> Result<()> {
        self.step_with_progress(stage, label, (index, total), || run(command))
    }

    fn test_command(&self, label: &str, command: Command, total: Option<usize>) -> Result<()> {
        self.begin_substep(ReleaseStage::Checks, label, total.map(|total| (0, total)));
        match run_test_command(command, self, total) {
            Ok(()) => {
                if let Some(total) = total {
                    self.set_substep_progress(total, total);
                }
                self.finish_substep(true);
                Ok(())
            }
            Err(error) => {
                self.finish_substep(false);
                Err(error.context(format!("{label} failed")))
            }
        }
    }

    fn begin_substep(&self, stage: ReleaseStage, label: &str, progress: Option<(usize, usize)>) {
        {
            let mut state = self.state.borrow_mut();
            if state.current_stage != Some(stage) {
                if let Some(previous) = state.current_stage {
                    let previous_state = &mut state.stages[previous.index()];
                    if previous_state.status == StageStatus::Active {
                        previous_state.status = StageStatus::Complete;
                    }
                }
                state.current_stage = Some(stage);
                state.stages[stage.index()].status = StageStatus::Active;
            }
            let substeps = &mut state.stages[stage.index()].substeps;
            if substeps.len() == SUBSTEP_LIMIT {
                substeps.pop_front();
            }
            substeps.push_back(SubstepState {
                label: label.to_owned(),
                status: SubstepStatus::Active,
                progress,
            });
        }
        self.refresh();
    }

    fn set_substep_progress(&self, completed: usize, total: usize) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(stage) = state.current_stage
                && let Some(substep) = state.stages[stage.index()].substeps.back_mut()
            {
                substep.progress = Some((completed, total));
            }
        }
        self.refresh();
    }

    fn finish_substep(&self, success: bool) {
        let plain_line = {
            let mut state = self.state.borrow_mut();
            let Some(stage) = state.current_stage else {
                return;
            };
            let stage_state = &mut state.stages[stage.index()];
            if let Some(substep) = stage_state.substeps.back_mut() {
                substep.status = if success {
                    SubstepStatus::Complete
                } else {
                    SubstepStatus::Failed
                };
            }
            if !success {
                stage_state.status = StageStatus::Failed;
            }
            if self.interactive {
                None
            } else {
                stage_state
                    .substeps
                    .back()
                    .map(|substep| self.plain_line(stage, stage_state.status, substep))
            }
        };
        self.refresh();
        if let Some(line) = plain_line {
            eprintln!("{line}");
        }
    }

    fn complete_current_stage(&self) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(stage) = state.current_stage
                && state.stages[stage.index()].status == StageStatus::Active
            {
                state.stages[stage.index()].status = StageStatus::Complete;
            }
        }
        self.refresh();
    }

    fn fail_current_stage(&self) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(stage) = state.current_stage {
                state.stages[stage.index()].status = StageStatus::Failed;
            }
        }
        self.refresh();
    }

    fn refresh(&self) {
        if !self.interactive {
            return;
        }
        let mut state = self.state.borrow_mut();
        if !state.prompt_active {
            render_frame_locked(&mut state, self.color);
        }
    }

    #[cfg(test)]
    fn render_row(&self, row: &RenderedRow) -> String {
        render_row_with_color(row, self.color)
    }

    fn plain_line(
        &self,
        stage: ReleaseStage,
        status: StageStatus,
        substep: &SubstepState,
    ) -> String {
        let marker = if status == StageStatus::Failed {
            "✖"
        } else {
            "✔"
        };
        let stage_text = format!("{marker} {}", stage.label());
        let stage_text = format!("{stage_text:<width$}", width = STAGE_COLUMN_WIDTH);
        let substep_marker = if substep.status == SubstepStatus::Failed {
            "✖"
        } else {
            "✔"
        };
        let label = match substep.progress {
            Some((completed, total)) => format!("{} {completed}/{total}", substep.label),
            None => substep.label.clone(),
        };
        format!("{stage_text}  {substep_marker} {label}")
    }

    fn begin_prompt(&self) {
        if !self.interactive {
            return;
        }
        let mut state = self.state.borrow_mut();
        let term = Term::stderr();
        let separator = "────────────────────────────────────────────────────────";
        let separator = if self.color {
            term.style()
                .dim()
                .apply_to(separator)
                .force_styling(true)
                .to_string()
        } else {
            separator.to_owned()
        };
        let _ = term.write_line(&separator);
        let _ = term.flush();
        state.prompt_active = true;
        state.prompt_lines = 1;
    }

    fn replace_prompt(&self, lines: &[String]) -> Result<()> {
        let mut state = self.state.borrow_mut();
        let term = Term::stderr();
        let prompt_lines = state.prompt_lines.saturating_sub(1);
        if prompt_lines > 0 {
            term.clear_last_lines(prompt_lines)
                .context("clear release prompt")?;
        }
        for line in lines {
            term.write_line(line).context("write release prompt")?;
        }
        term.flush().context("flush release prompt")?;
        state.prompt_lines = 1 + lines.len();
        Ok(())
    }

    fn finish_prompt(&self) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        let mut state = self.state.borrow_mut();
        let term = Term::stderr();
        let lines = state.frame_lines + state.prompt_lines;
        if lines > 0 {
            term.clear_last_lines(lines)
                .context("clear release prompt frame")?;
        }
        state.frame_lines = 0;
        state.prompt_lines = 0;
        state.prompt_active = false;
        render_frame_locked(&mut state, self.color);
        Ok(())
    }

    fn read_prompt_interactive(&self, message: &str) -> Result<String> {
        self.begin_prompt();
        let result = (|| {
            let term = Term::stderr();
            term.write_str(message).context("write release prompt")?;
            term.flush().context("flush release prompt")?;
            {
                let mut state = self.state.borrow_mut();
                state.prompt_lines = 2;
            }
            term.read_line().context("read release prompt")
        })();
        self.finish_prompt()?;
        result
    }

    fn select_prompt_interactive<T: ToString>(&self, message: &str, items: &[T]) -> Result<usize> {
        self.begin_prompt();
        let term = Term::stderr();
        term.hide_cursor().context("hide release prompt cursor")?;
        let result = (|| {
            let mut selection = 0;
            loop {
                let lines = std::iter::once(message.to_owned())
                    .chain(items.iter().enumerate().map(|(index, item)| {
                        let line = format!(
                            "{} {}",
                            if index == selection { ">" } else { " " },
                            item.to_string()
                        );
                        if self.color && index == selection {
                            Term::stderr()
                                .style()
                                .yellow()
                                .bold()
                                .apply_to(line)
                                .force_styling(true)
                                .to_string()
                        } else {
                            line
                        }
                    }))
                    .collect::<Vec<_>>();
                self.replace_prompt(&lines)?;
                match term.read_key().context("read release selection")? {
                    Key::ArrowDown | Key::Tab | Key::Char('j') => {
                        selection = (selection + 1) % items.len();
                    }
                    Key::ArrowUp | Key::BackTab | Key::Char('k') => {
                        selection = selection.checked_sub(1).unwrap_or(items.len() - 1);
                    }
                    Key::Enter | Key::Char(' ') => break,
                    Key::Escape | Key::Char('q') => bail!("release selection cancelled"),
                    Key::CtrlC => bail!("release selection interrupted"),
                    _ => {}
                }
            }
            Ok(selection)
        })();
        term.show_cursor().context("show release prompt cursor")?;
        self.finish_prompt()?;
        result
    }

    fn edit_prompt_interactive(&self, message: &str, initial: &str) -> Result<String> {
        self.begin_prompt();
        let term = Term::stderr();
        term.hide_cursor().context("hide release prompt cursor")?;
        let result = (|| {
            let mut chars = initial.chars().collect::<Vec<_>>();
            let mut position = chars.len();
            loop {
                let value = chars.iter().collect::<String>();
                let line = format!("{message}{value}");
                self.replace_prompt(&[line])?;
                let left = value.chars().count().saturating_sub(position);
                if left > 0 {
                    term.move_cursor_left(left)
                        .context("move release version cursor")?;
                }
                term.flush().context("flush release version prompt")?;
                let old_position = position;
                let old_length = chars.len();
                match term.read_key().context("read release version")? {
                    Key::Enter => {
                        term.write_str("\n")
                            .context("finish release version prompt")?;
                        break Ok(value);
                    }
                    Key::ArrowLeft if position > 0 => position -= 1,
                    Key::ArrowRight if position < chars.len() => position += 1,
                    Key::Home => position = 0,
                    Key::End => position = chars.len(),
                    Key::Backspace if position > 0 => {
                        position -= 1;
                        chars.remove(position);
                    }
                    Key::Del if position < chars.len() => {
                        chars.remove(position);
                    }
                    Key::Char(chr) if !chr.is_ascii_control() => {
                        chars.insert(position, chr);
                        position += 1;
                    }
                    Key::Escape => break Err(anyhow::anyhow!("release version edit cancelled")),
                    Key::CtrlC => break Err(anyhow::anyhow!("release version edit interrupted")),
                    _ => {}
                }
                term.move_cursor_right(old_length.saturating_sub(old_position))
                    .context("move release version cursor to line end")?;
            }
        })();
        term.show_cursor().context("show release prompt cursor")?;
        self.finish_prompt()?;
        result
    }
}

impl Drop for ReleaseProgress {
    fn drop(&mut self) {
        if self.terminal_renderer {
            let _ = Term::stderr().show_cursor();
        }
    }
}

fn render_frame_locked(state: &mut ReleaseProgressState, color: bool) {
    let rows = rendered_rows(state);
    let term = Term::stderr();
    if state.frame_lines > 0 && term.clear_last_lines(state.frame_lines).is_err() {
        return;
    }
    for row in &rows {
        let line = render_row_with_color(row, color);
        if term.write_line(&line).is_err() {
            return;
        }
    }
    let _ = term.flush();
    state.frame_lines = rows.len();
}

fn render_row_with_color(row: &RenderedRow, color: bool) -> String {
    let stage_text = match (row.stage, row.stage_status) {
        (Some(stage), Some(status)) => {
            let marker = match status {
                StageStatus::Pending | StageStatus::Active => "",
                StageStatus::Complete => "✔",
                StageStatus::Failed => "✖",
            };
            if marker.is_empty() {
                stage.label().to_owned()
            } else {
                format!("{marker} {}", stage.label())
            }
        }
        _ => String::new(),
    };
    let stage_text = format!("{stage_text:<width$}", width = STAGE_COLUMN_WIDTH);
    let substep_text = row.substep.as_ref().map(|substep| {
        let marker = match substep.status {
            SubstepStatus::Active => "→",
            SubstepStatus::Complete => "✔",
            SubstepStatus::Failed => "✖",
        };
        let label = match substep.progress {
            Some((completed, total)) => format!("{} {completed}/{total}", substep.label),
            None => substep.label.clone(),
        };
        format!("{marker} {label}")
    });

    if !color {
        return format!(
            "{}{}",
            stage_text,
            substep_text.map_or_else(String::new, |text| format!("  {text}"))
        );
    }

    let stage = match row.stage_status.unwrap_or(StageStatus::Pending) {
        StageStatus::Pending => Term::stderr().style().dim().apply_to(stage_text),
        StageStatus::Active => Term::stderr().style().cyan().bold().apply_to(stage_text),
        StageStatus::Complete => Term::stderr().style().green().apply_to(stage_text),
        StageStatus::Failed => Term::stderr().style().red().apply_to(stage_text),
    }
    .force_styling(true)
    .to_string();
    let substep = substep_text.map_or_else(String::new, |text| {
        let styled = match row.substep.as_ref().map(|value| value.status) {
            Some(SubstepStatus::Active) => Term::stderr().style().yellow().bold().apply_to(text),
            Some(SubstepStatus::Complete) => Term::stderr().style().green().apply_to(text),
            Some(SubstepStatus::Failed) => Term::stderr().style().red().apply_to(text),
            None => Term::stderr().style().apply_to(text),
        };
        format!("  {}", styled.force_styling(true))
    });
    format!("{stage}{substep}")
}

fn rendered_rows(state: &ReleaseProgressState) -> Vec<RenderedRow> {
    let mut rows = Vec::new();
    for stage in ReleaseStage::ALL {
        let stage_state = &state.stages[stage.index()];
        if stage_state.status == StageStatus::Pending && stage_state.substeps.is_empty() {
            continue;
        }
        let active_stage =
            state.current_stage == Some(stage) && stage_state.status == StageStatus::Active;
        let substeps = if active_stage {
            stage_state
                .substeps
                .iter()
                .rev()
                .take(SUBSTEP_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        } else {
            stage_state
                .substeps
                .back()
                .cloned()
                .into_iter()
                .collect::<Vec<_>>()
        };
        if substeps.is_empty() {
            rows.push(RenderedRow {
                stage: Some(stage),
                stage_status: Some(stage_state.status),
                substep: None,
            });
            continue;
        }
        for (index, substep) in substeps.into_iter().enumerate() {
            rows.push(RenderedRow {
                stage: (index == 0).then_some(stage),
                stage_status: (index == 0).then_some(stage_state.status),
                substep: Some(substep),
            });
        }
    }
    rows
}

fn publish_release_cli() -> Result<()> {
    let progress = ReleaseProgress::new();
    let workspace = workspace_root();
    let branch = progress.step(ReleaseStage::Infos, "Check release environment", || {
        preflight_repository(&workspace)?;
        let mut github_auth = Command::new("gh");
        github_auth.args(["auth", "status"]);
        run(github_auth)?;
        let branch = capture(git(&workspace, &["branch", "--show-current"]))?;
        let branch = branch.trim().to_owned();
        if branch.is_empty() {
            bail!("publish-release requires a checked-out branch");
        }
        Ok(branch)
    })?;

    let track = prompt_track(&progress)?;
    if track == ReleaseTrack::Release && branch != "main" {
        bail!("stable releases must be published from main");
    }

    let (published, current) =
        progress.step(ReleaseStage::Infos, "Load release context", || {
            let published = published_versions()?;
            let current = read_workspace_version(&workspace.join("Cargo.toml"))?;
            Ok((published, current))
        })?;
    let highest = published.iter().copied().max();
    let version_suggestions = VersionSuggestions::new(current, highest)?;
    let version = prompt_version(&progress, version_suggestions)?;
    if highest.is_some_and(|highest| version <= highest) {
        bail!("release version {version} must be newer than every published version");
    }
    let tag = format!("v{version}");
    if track == ReleaseTrack::Release {
        let expected = format!("RELEASE {tag}");
        let confirmation = prompt(
            &progress,
            ReleaseStage::Infos,
            "Confirm stable release",
            &format!("Type '{expected}' to publish a stable release: "),
        )?;
        if confirmation != expected {
            bail!("stable release confirmation did not match");
        }
    }
    let incomplete_draft = progress.step(ReleaseStage::Infos, "Check release target", || {
        let incomplete_draft = incomplete_draft_exists(&tag)?;
        if !incomplete_draft {
            ensure_tag_absent(&workspace, &tag)?;
        }
        Ok(incomplete_draft)
    })?;

    run_checks(&workspace, &progress)?;

    let manifest = workspace.join("Cargo.toml");
    let lockfile = workspace.join("Cargo.lock");
    let mut version_transaction = None;
    let outcome = (|| -> Result<PathBuf> {
        version_transaction = if version != current {
            Some(
                progress.step(ReleaseStage::Preparation, "Prepare release version", || {
                    let transaction = VersionTransaction::begin(&manifest, &lockfile, version)?;
                    run(cargo_metadata(&workspace))?;
                    Ok(transaction)
                })?,
            )
        } else {
            None
        };
        let verification = progress.step(
            ReleaseStage::Preparation,
            "Prepare verification package",
            || tempfile::tempdir().context("create release verification directory"),
        )?;
        let executable = build_release(version, &progress)?;
        let verification_artifact =
            progress.step(ReleaseStage::Build, "Package verification build", || {
                package_release(&executable, version, verification.path(), &workspace)
            })?;
        progress.step(ReleaseStage::Build, "Validate verification package", || {
            validate_release_outputs(&verification_artifact, version)
        })?;

        let message = format!("Release {tag}");
        progress.step(ReleaseStage::Publish, "Commit release version", || {
            run(git(&workspace, &["add", "Cargo.toml", "Cargo.lock"]))?;
            run(git(
                &workspace,
                &["commit", "--allow-empty", "-m", &message],
            ))
        })?;
        if let Some(transaction) = version_transaction.as_mut() {
            transaction.commit();
        }
        let output = workspace.join("dist");
        let artifact = progress.step(
            ReleaseStage::Publish,
            "Package release (reuse verified build)",
            || {
                clear_local_release_outputs(&output, version)?;
                package_release(&executable, version, &output, &workspace)
            },
        )?;
        progress.step(ReleaseStage::Publish, "Validate release package", || {
            validate_release_outputs(&artifact, version)
        })?;
        progress.step(ReleaseStage::Publish, "Publish release", || {
            run(git(&workspace, &["push", "origin", &branch]))?;
            let revision = capture(git(&workspace, &["rev-parse", "HEAD"]))?;
            if incomplete_draft {
                cleanup_incomplete_draft(&tag)?;
            }
            publish_github_release(track, version, revision.trim(), &artifact, &workspace)
        })?;
        Ok(artifact)
    })();

    match outcome {
        Ok(artifact) => {
            progress.complete_current_stage();
            println!("published {tag} from {}", artifact.display());
            Ok(())
        }
        Err(error) => {
            progress.fail_current_stage();
            if let Some(transaction) = version_transaction.as_mut()
                && let Err(rollback_error) = transaction.rollback()
            {
                return Err(error.context(format!(
                    "release also failed to restore the workspace version: {rollback_error:#}"
                )));
            }
            Err(error)
        }
    }
}

fn preflight_repository(workspace: &Path) -> Result<()> {
    let status = capture(git(
        workspace,
        &["status", "--porcelain", "--untracked-files=all"],
    ))?;
    if !status.trim().is_empty() {
        bail!("publish-release requires a clean worktree");
    }
    let remote = capture(git(workspace, &["remote", "get-url", "origin"]))?;
    if !remote.contains("NatanSlvdr/StageSwap") {
        bail!("origin does not point to NatanSlvdr/StageSwap");
    }
    run(git(workspace, &["fetch", "origin"]))?;
    let head = capture(git(workspace, &["rev-parse", "HEAD"]))?;
    let upstream = capture(git(workspace, &["rev-parse", "@{upstream}"]))?;
    if head.trim() != upstream.trim() {
        bail!("the current branch must exactly match its pushed upstream before publishing");
    }
    Ok(())
}

fn prompt_track(progress: &ReleaseProgress) -> Result<ReleaseTrack> {
    let selection = progress.step(ReleaseStage::Infos, "Select release track", || {
        if Term::stderr().is_term() {
            progress
                .select_prompt_interactive("Release track", &["Development (default)", "Release"])
        } else {
            eprintln!("Release track:");
            eprintln!("  1. Development (default)");
            eprintln!("  2. Release");
            match read_prompt("Select [1]: ")?.as_str() {
                "" | "1" => Ok(0),
                "2" => Ok(1),
                _ => bail!("release track must be 1 or 2"),
            }
        }
    })?;
    match selection {
        0 => Ok(ReleaseTrack::Development),
        1 => Ok(ReleaseTrack::Release),
        _ => bail!("release track selection was out of range"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VersionSuggestions {
    patch: ReleaseVersion,
    minor: ReleaseVersion,
    major: ReleaseVersion,
}

impl VersionSuggestions {
    fn new(current: ReleaseVersion, highest: Option<ReleaseVersion>) -> Result<Self> {
        let base = highest
            .filter(|highest| *highest > current)
            .unwrap_or(current);
        Ok(Self {
            patch: base.increment_patch()?,
            minor: base.increment_minor()?,
            major: base.increment_major()?,
        })
    }
}

fn prompt_version(
    progress: &ReleaseProgress,
    suggestions: VersionSuggestions,
) -> Result<ReleaseVersion> {
    let items = [
        format!("Patch ({})", suggestions.patch),
        format!("Minor ({})", suggestions.minor),
        format!("Major ({})", suggestions.major),
        format!("Manual (edit {})", suggestions.patch),
    ];
    let selection = progress.step(ReleaseStage::Infos, "Select version bump", || {
        if Term::stderr().is_term() {
            progress.select_prompt_interactive("Version bump", &items)
        } else {
            eprintln!("Version bump:");
            for (index, item) in items.iter().enumerate() {
                eprintln!("  {}. {item}", index + 1);
            }
            match read_prompt("Select [1]: ")?.as_str() {
                "" | "1" => Ok(0),
                "2" => Ok(1),
                "3" => Ok(2),
                "4" => Ok(3),
                _ => bail!("version bump must be 1, 2, 3, or 4"),
            }
        }
    })?;
    match selection {
        0 => Ok(suggestions.patch),
        1 => Ok(suggestions.minor),
        2 => Ok(suggestions.major),
        3 => prompt_manual_version(progress, suggestions.patch),
        _ => bail!("version bump selection was out of range"),
    }
}

fn prompt_manual_version(
    progress: &ReleaseProgress,
    suggested: ReleaseVersion,
) -> Result<ReleaseVersion> {
    progress.step(ReleaseStage::Infos, "Edit release version", || {
        let value = if Term::stderr().is_term() {
            progress.edit_prompt_interactive("Version: ", &suggested.to_string())?
        } else {
            read_prompt(&format!("Version [{suggested}]: "))?
        };
        parse_version_input(&value, suggested)
    })
}

fn prompt(
    progress: &ReleaseProgress,
    stage: ReleaseStage,
    label: &str,
    message: &str,
) -> Result<String> {
    progress.step(stage, label, || {
        if Term::stderr().is_term() {
            progress.read_prompt_interactive(message)
        } else {
            read_prompt(message)
        }
    })
}

fn read_prompt(message: &str) -> Result<String> {
    eprint!("{message}");
    io::stderr().flush().context("flush prompt")?;
    let mut value = String::new();
    io::stdin().read_line(&mut value).context("read prompt")?;
    eprintln!();
    Ok(value.trim().to_owned())
}

fn parse_version_input(value: &str, suggested: ReleaseVersion) -> Result<ReleaseVersion> {
    if value.trim().is_empty() {
        Ok(suggested)
    } else {
        ReleaseVersion::parse(value.trim())
    }
}

fn published_versions() -> Result<Vec<ReleaseVersion>> {
    let output = capture({
        let mut command = Command::new("gh");
        command.args([
            "release",
            "list",
            "--repo",
            "NatanSlvdr/StageSwap",
            "--limit",
            "100",
            "--json",
            "tagName,isDraft",
            "--jq",
            ".[] | select(.isDraft == false) | .tagName",
        ]);
        command
    })?;
    Ok(output
        .lines()
        .filter_map(|tag| {
            let version = ReleaseVersion::parse(tag).ok()?;
            (tag == format!("v{version}")).then_some(version)
        })
        .collect())
}

fn incomplete_draft_exists(tag: &str) -> Result<bool> {
    let mut inspect = Command::new("gh");
    inspect.args([
        "release",
        "view",
        tag,
        "--repo",
        "NatanSlvdr/StageSwap",
        "--json",
        "isDraft",
        "--jq",
        ".isDraft",
    ]);
    let output = inspect
        .output()
        .with_context(|| format!("could not inspect GitHub release {tag}"))?;
    if output.status.success() {
        let is_draft =
            String::from_utf8(output.stdout).context("GitHub release state was not valid UTF-8")?;
        if is_draft.trim() != "true" {
            bail!("release {tag} already exists");
        }
        Ok(true)
    } else {
        let diagnostics = command_diagnostics(&output.stdout, &output.stderr);
        let diagnostics_lower = diagnostics.to_ascii_lowercase();
        if !diagnostics_lower.contains("release not found")
            && !diagnostics_lower.contains("not found")
        {
            bail!("could not inspect GitHub release {tag}{diagnostics}");
        }
        Ok(false)
    }
}

fn cleanup_incomplete_draft(tag: &str) -> Result<()> {
    let mut delete = Command::new("gh");
    delete.args([
        "release",
        "delete",
        tag,
        "--repo",
        "NatanSlvdr/StageSwap",
        "--yes",
        "--cleanup-tag",
    ]);
    run(delete)
}

fn ensure_tag_absent(workspace: &Path, tag: &str) -> Result<()> {
    let reference = format!("refs/tags/{tag}");
    let output = capture(git(
        workspace,
        &["ls-remote", "--tags", "origin", &reference],
    ))?;
    if !output.trim().is_empty() {
        bail!("tag {tag} already exists");
    }
    Ok(())
}

fn run_checks(workspace: &Path, progress: &ReleaseProgress) -> Result<()> {
    const TOTAL: usize = 4;
    let mut format = Command::new("cargo");
    format
        .current_dir(workspace)
        .args(["fmt", "--all", "--", "--check"]);
    progress.command_counted(ReleaseStage::Checks, 1, TOTAL, "Check formatting", format)?;
    let mut clippy = Command::new("cargo");
    clippy.current_dir(workspace).args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    progress.command_counted(ReleaseStage::Checks, 2, TOTAL, "Run host Clippy", clippy)?;
    let mut windows_clippy = Command::new("cargo");
    windows_clippy.current_dir(workspace).args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--target",
        "x86_64-pc-windows-msvc",
        "--",
        "-D",
        "warnings",
    ]);
    progress.command_counted(
        ReleaseStage::Checks,
        3,
        TOTAL,
        "Run Windows-target Clippy",
        windows_clippy,
    )?;
    let mut tests = Command::new("cargo");
    tests
        .current_dir(workspace)
        .args(["test", "--workspace", "--all-targets"]);
    let total_tests = discover_test_count(workspace);
    progress.test_command("Run workspace tests", tests, total_tests)
}

fn validate_release_outputs(artifact: &Path, version: ReleaseVersion) -> Result<()> {
    let expected_name = format!("{RELEASE_PREFIX}{version}.exe");
    if artifact.file_name().and_then(|name| name.to_str()) != Some(&expected_name) {
        bail!("packaged executable name does not match {expected_name}");
    }
    let checksum = artifact.with_file_name(format!("{expected_name}.sha256"));
    if !checksum.is_file() {
        bail!("packaging did not produce {}", checksum.display());
    }
    Ok(())
}

fn clear_local_release_outputs(output: &Path, version: ReleaseVersion) -> Result<()> {
    let executable = output.join(format!("{RELEASE_PREFIX}{version}.exe"));
    let checksum = output.join(format!("{RELEASE_PREFIX}{version}.exe.sha256"));
    for path in [executable, checksum] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("remove stale release output {}", path.display()));
            }
        }
    }
    Ok(())
}

fn publish_github_release(
    track: ReleaseTrack,
    version: ReleaseVersion,
    revision: &str,
    artifact: &Path,
    workspace: &Path,
) -> Result<()> {
    let tag = format!("v{version}");
    let title = match track {
        ReleaseTrack::Development => format!("StageSwap {tag} Beta"),
        ReleaseTrack::Release => format!("StageSwap {tag}"),
    };
    let checksum = artifact.with_file_name(format!("{}{}.exe.sha256", RELEASE_PREFIX, version));
    let mut command = Command::new("gh");
    command.current_dir(workspace).args([
        "release",
        "create",
        &tag,
        artifact.to_str().context("artifact path is not UTF-8")?,
        checksum.to_str().context("checksum path is not UTF-8")?,
        "--repo",
        "NatanSlvdr/StageSwap",
        "--target",
        revision,
        "--title",
        &title,
        "--generate-notes",
    ]);
    match track {
        ReleaseTrack::Development => {
            command.args(["--prerelease", "--latest=false"]);
        }
        ReleaseTrack::Release => {
            command.arg("--latest");
        }
    }
    run(command)
}

fn git(workspace: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.current_dir(workspace).args(arguments);
    command
}

fn capture(mut command: Command) -> Result<String> {
    let description = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("could not run {description}"))?;
    if !output.status.success() {
        bail!(
            "command failed ({}): {description}{}",
            output.status,
            command_diagnostics(&output.stdout, &output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("command output was not valid UTF-8")
}

fn command_diagnostics(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let mut diagnostics = String::new();
    if !stdout.trim().is_empty() {
        diagnostics.push_str("\nstdout:\n");
        diagnostics.push_str(stdout.trim());
    }
    if !stderr.trim().is_empty() {
        diagnostics.push_str("\nstderr:\n");
        diagnostics.push_str(stderr.trim());
    }
    diagnostics
}

fn discover_test_count(workspace: &Path) -> Option<usize> {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .args(["test", "--workspace", "--all-targets", "--", "--list"]);
    capture(command)
        .ok()
        .and_then(|output| parse_test_list_count(&output))
}

fn parse_test_list_count(output: &str) -> Option<usize> {
    let mut total = 0;
    let mut found_summary = false;
    for line in output.lines() {
        let mut words = line.split_whitespace();
        let Some(count) = words.next().and_then(|value| value.parse::<usize>().ok()) else {
            continue;
        };
        let Some(kind) = words.next() else {
            continue;
        };
        if matches!(kind.trim_end_matches(','), "test" | "tests") && line.contains("benchmark") {
            total += count;
            found_summary = true;
        }
    }
    found_summary.then_some(total)
}

#[derive(Clone, Copy)]
enum ChildStream {
    Stdout,
    Stderr,
}

struct ChildLine {
    stream: ChildStream,
    bytes: Vec<u8>,
}

fn run_test_command(
    mut command: Command,
    progress: &ReleaseProgress,
    total: Option<usize>,
) -> Result<()> {
    let description = format!("{command:?}");
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not run {description}"))?;
    let stdout = child.stdout.take().context("capture test stdout")?;
    let stderr = child.stderr.take().context("capture test stderr")?;
    let (sender, receiver) = mpsc::channel();
    let stdout_thread = spawn_child_reader(stdout, ChildStream::Stdout, sender.clone());
    let stderr_thread = spawn_child_reader(stderr, ChildStream::Stderr, sender);
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut completed = 0;
    for line in receiver {
        match line.stream {
            ChildStream::Stdout => {
                if parse_test_completion(&line.bytes) {
                    completed += 1;
                    if let Some(total) = total {
                        progress.set_substep_progress(completed.min(total), total);
                    }
                }
                stdout_bytes.extend_from_slice(&line.bytes);
            }
            ChildStream::Stderr => stderr_bytes.extend_from_slice(&line.bytes),
        }
    }
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    let status = child
        .wait()
        .with_context(|| format!("wait for {description}"))?;
    if !status.success() {
        bail!(
            "command failed ({}): {description}{}",
            status,
            command_diagnostics(&stdout_bytes, &stderr_bytes)
        );
    }
    Ok(())
}

fn spawn_child_reader<R>(
    reader: R,
    stream: ChildStream,
    sender: mpsc::Sender<ChildLine>,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender
                        .send(ChildLine {
                            stream,
                            bytes: bytes.clone(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    })
}

fn parse_test_completion(bytes: &[u8]) -> bool {
    let line = String::from_utf8_lossy(bytes);
    let line = line.trim();
    let Some(line) = line.strip_prefix("test ") else {
        return false;
    };
    let Some((_, status)) = line.rsplit_once(" ... ") else {
        return false;
    };
    matches!(status.trim(), "ok" | "FAILED" | "ignored")
}

fn build_release_pair(
    workspace: &Path,
    target: &str,
    architecture: &str,
    progress: &ReleaseProgress,
) -> Result<(PathBuf, PathBuf)> {
    progress.command_counted(
        ReleaseStage::Build,
        1,
        2,
        "Build media-source DLL",
        cargo_build(workspace, MEDIA_SOURCE_PACKAGE, target, None),
    )?;
    let dll = release_artifact(workspace, target, MEDIA_SOURCE_DLL);
    if !dll.is_file() {
        bail!("media-source build did not produce {}", dll.display());
    }
    validate_pe_path(&dll, architecture)?;
    let embedded_dll = dll
        .canonicalize()
        .context("canonicalize media-source DLL")?;
    progress.command_counted(
        ReleaseStage::Build,
        2,
        2,
        "Build StageSwap executable",
        cargo_build(workspace, APP_PACKAGE, target, Some(&embedded_dll)),
    )?;
    let executable = release_artifact(workspace, target, APP_EXECUTABLE);
    validate_pe_path(&executable, architecture)?;
    validate_embedded_payload(&executable, &dll)?;
    Ok((dll, executable))
}

fn release_artifact(workspace: &Path, target: &str, name: &str) -> PathBuf {
    workspace
        .join("target")
        .join(target)
        .join("release")
        .join(name)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    fn parse(value: &str) -> Result<Self> {
        let mut parts = value.split('.');
        let version = Self {
            major: parts.next().context("missing major version")?.parse()?,
            minor: parts.next().context("missing minor version")?.parse()?,
            patch: parts.next().context("missing patch version")?.parse()?,
        };
        if parts.next().is_some() {
            bail!("release version must contain exactly three numbers");
        }
        Ok(version)
    }

    fn increment_patch(self) -> Result<Self> {
        Ok(Self {
            patch: self
                .patch
                .checked_add(1)
                .context("patch version overflow")?,
            ..self
        })
    }

    fn increment_minor(self) -> Result<Self> {
        Ok(Self {
            minor: self
                .minor
                .checked_add(1)
                .context("minor version overflow")?,
            patch: 0,
            ..self
        })
    }

    fn increment_major(self) -> Result<Self> {
        Ok(Self {
            major: self
                .major
                .checked_add(1)
                .context("major version overflow")?,
            minor: 0,
            patch: 0,
        })
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
fn select_release_version(
    application_version: ReleaseVersion,
    latest: Option<&(ReleaseVersion, String)>,
    digest: &[u8; 32],
) -> Result<ReleaseVersion> {
    let Some((latest_version, latest_digest)) = latest else {
        return Ok(application_version);
    };
    if *latest_version == application_version && *latest_digest == hex(digest) {
        return Ok(application_version);
    }
    Ok(max(latest_version.increment_patch()?, application_version))
}

#[cfg(test)]
fn latest_release(output: &Path) -> Result<Option<(ReleaseVersion, String)>> {
    if !output.exists() {
        return Ok(None);
    }
    let mut latest = None;
    for entry in fs::read_dir(output).context("read release output directory")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = name
            .strip_prefix(RELEASE_PREFIX)
            .and_then(|name| name.strip_suffix(RELEASE_SUFFIX))
        else {
            continue;
        };
        let Ok(version) = ReleaseVersion::parse(version) else {
            continue;
        };
        if latest
            .as_ref()
            .is_some_and(|(latest_version, _)| *latest_version >= version)
        {
            continue;
        }
        let contents = fs::read_to_string(entry.path())
            .with_context(|| format!("read release checksum {}", entry.path().display()))?;
        let digest = contents
            .lines()
            .find(|line| !line.starts_with('#') && !line.trim().is_empty())
            .and_then(|line| line.split_whitespace().next())
            .context("release sidecar has no checksum")?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("release sidecar has an invalid SHA-256 checksum");
        }
        latest = Some((version, digest.to_ascii_lowercase()));
    }
    Ok(latest)
}

fn cargo_build(
    workspace: &Path,
    package: &str,
    target: &str,
    embedded_dll: Option<&Path>,
) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(workspace);
    if env::var_os("STAGESWAP_USE_CARGO_XWIN").is_some() {
        command.args(["xwin", "build"]);
    } else {
        command.arg("build");
    }
    command.args(["-p", package, "--release", "--target", target]);
    if let Some(dll) = embedded_dll {
        command.env("STAGESWAP_MEDIA_SOURCE_DLL", dll);
    }
    command
}

fn cargo_metadata(workspace: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .args(["metadata", "--format-version", "1"])
        .stdout(Stdio::null());
    command
}

fn read_workspace_version(manifest: &Path) -> Result<ReleaseVersion> {
    let contents = fs::read_to_string(manifest)
        .with_context(|| format!("read workspace manifest {}", manifest.display()))?;
    let (version, _) = workspace_version_range(&contents)?;
    Ok(version)
}

fn workspace_version_range(contents: &str) -> Result<(ReleaseVersion, Range<usize>)> {
    let mut in_workspace_package = false;
    let mut offset = 0;
    for line in contents.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace_package = trimmed == "[workspace.package]";
        } else if in_workspace_package
            && let Some((key, value)) = trimmed.split_once('=')
            && key.trim() == "version"
        {
            let value = value
                .split('#')
                .next()
                .expect("split always returns one item")
                .trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .context("workspace package version must be a quoted string")?;
            let version = ReleaseVersion::parse(unquoted)?;
            let value_offset = line
                .find(unquoted)
                .context("could not locate workspace version value")?;
            let start = offset + value_offset;
            return Ok((version, start..start + unquoted.len()));
        }
        offset += line.len();
    }
    bail!("workspace manifest has no [workspace.package] version")
}

fn replace_workspace_version(contents: &str, version: ReleaseVersion) -> Result<String> {
    let (_, range) = workspace_version_range(contents)?;
    let mut updated = contents.to_owned();
    updated.replace_range(range, &version.to_string());
    Ok(updated)
}

struct VersionTransaction {
    manifest: PathBuf,
    manifest_contents: Vec<u8>,
    lockfile: PathBuf,
    lockfile_contents: Option<Vec<u8>>,
    active: bool,
}

impl VersionTransaction {
    fn begin(manifest: &Path, lockfile: &Path, version: ReleaseVersion) -> Result<Self> {
        let manifest_contents = fs::read(manifest)
            .with_context(|| format!("read workspace manifest {}", manifest.display()))?;
        let manifest_text = std::str::from_utf8(&manifest_contents)
            .context("workspace manifest is not valid UTF-8")?;
        let updated = replace_workspace_version(manifest_text, version)?;
        let lockfile_contents = match fs::read(lockfile) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("read lockfile {}", lockfile.display()));
            }
        };
        fs::write(manifest, updated)
            .with_context(|| format!("update workspace version in {}", manifest.display()))?;
        Ok(Self {
            manifest: manifest.to_owned(),
            manifest_contents,
            lockfile: lockfile.to_owned(),
            lockfile_contents,
            active: true,
        })
    }

    fn commit(&mut self) {
        self.active = false;
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        fs::write(&self.manifest, &self.manifest_contents)
            .with_context(|| format!("restore workspace manifest {}", self.manifest.display()))?;
        match &self.lockfile_contents {
            Some(contents) => fs::write(&self.lockfile, contents)
                .with_context(|| format!("restore lockfile {}", self.lockfile.display()))?,
            None if self.lockfile.exists() => {
                fs::remove_file(&self.lockfile).with_context(|| {
                    format!("remove generated lockfile {}", self.lockfile.display())
                })?
            }
            None => {}
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for VersionTransaction {
    fn drop(&mut self) {
        if self.active {
            let _ = self.rollback();
        }
    }
}

fn publish_release(
    output: &Path,
    artifact: &str,
    executable: &[u8],
    checksum: &[u8],
) -> Result<PathBuf> {
    fs::create_dir_all(output).context("create dist directory")?;
    let checksum_name = format!("{artifact}.sha256");
    let destination = output.join(artifact);
    let checksum_destination = output.join(&checksum_name);
    ensure_existing_output_matches(&destination, executable)?;
    ensure_existing_output_matches(&checksum_destination, checksum)?;

    let staging = tempfile::tempdir_in(output).context("create release staging directory")?;
    let staged_artifact = staging.path().join(artifact);
    let staged_checksum = staging.path().join(&checksum_name);
    fs::write(&staged_artifact, executable).context("stage release executable")?;
    fs::write(&staged_checksum, checksum).context("stage release checksum")?;

    let artifact_created = if destination.exists() {
        false
    } else {
        fs::rename(&staged_artifact, &destination)
            .with_context(|| format!("publish release executable {}", destination.display()))?;
        true
    };
    if !checksum_destination.exists()
        && let Err(error) = fs::rename(&staged_checksum, &checksum_destination)
    {
        if artifact_created {
            let _ = fs::remove_file(&destination);
        }
        return Err(error).with_context(|| {
            format!(
                "publish release checksum {}",
                checksum_destination.display()
            )
        });
    }
    Ok(destination)
}

fn ensure_existing_output_matches(path: &Path, expected: &[u8]) -> Result<()> {
    if path.exists() {
        let existing = fs::read(path)
            .with_context(|| format!("read existing release output {}", path.display()))?;
        if existing != expected {
            bail!(
                "release output {} already exists with different contents",
                path.display()
            );
        }
    }
    Ok(())
}

fn selected_windows_sdk() -> Result<String> {
    let value = env::var("STAGESWAP_WINDOWS_SDK_VERSION")
        .or_else(|_| env::var("WindowsSDKVersion"))
        .context(
            "Windows SDK is not selected; run from a VS Developer shell or set \
             STAGESWAP_WINDOWS_SDK_VERSION",
        )?;
    let normalized = value.trim().trim_end_matches(['\\', '/']);
    if normalized != REQUIRED_WINDOWS_SDK {
        bail!("Windows SDK {normalized} is selected; packaging requires {REQUIRED_WINDOWS_SDK}");
    }
    Ok(normalized.to_owned())
}

fn validate_embedded_payload(executable: &Path, payload: &Path) -> Result<()> {
    let executable_bytes =
        fs::read(executable).with_context(|| format!("could not read {}", executable.display()))?;
    let payload_bytes =
        fs::read(payload).with_context(|| format!("could not read {}", payload.display()))?;
    let marker_len = payload_bytes.len().min(64);
    if marker_len == 0 {
        bail!("embedded media-source payload is empty");
    }
    let marker = &payload_bytes[..marker_len];
    let embedded = executable_bytes
        .windows(marker_len)
        .enumerate()
        .any(|(offset, candidate)| {
            candidate == marker
                && executable_bytes.get(offset..offset + payload_bytes.len())
                    == Some(payload_bytes.as_slice())
        });
    if !embedded {
        bail!("executable does not contain the media-source payload");
    }
    Ok(())
}

fn run(mut command: Command) -> Result<()> {
    let description = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("could not run {description}"))?;
    if !output.status.success() {
        bail!(
            "command failed ({}): {description}{}",
            output.status,
            command_diagnostics(&output.stdout, &output.stderr)
        );
    }
    Ok(())
}

fn validate_pe(path: String, architecture: String) -> Result<()> {
    validate_pe_path(Path::new(&path), &architecture)?;
    println!("validated {architecture} PE: {path}");
    Ok(())
}

fn validate_pe_path(path: &Path, architecture: &str) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    if bytes.get(..2) != Some(b"MZ") {
        bail!("{} is not a PE file", path.display());
    }
    let pe_offset = u32::from_le_bytes(
        bytes
            .get(0x3c..0x40)
            .context("truncated DOS header")?
            .try_into()?,
    ) as usize;
    if bytes.get(pe_offset..pe_offset + 4) != Some(b"PE\0\0") {
        bail!("{} has no PE signature", path.display());
    }
    let machine = u16::from_le_bytes(
        bytes
            .get(pe_offset + 4..pe_offset + 6)
            .context("truncated COFF header")?
            .try_into()?,
    );
    let expected = match architecture {
        "x64" => 0x8664,
        _ => bail!("architecture must be x64"),
    };
    if machine != expected {
        bail!("PE machine 0x{machine:04x} does not match {architecture}");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    output
}

// Compact FIPS 180-4 SHA-256 used by packaging to avoid relying on a platform
// command or adding a runtime dependency to the artifact.
fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (word, bytes) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("four-byte word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (value, addition) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(addition);
        }
    }
    let mut output = [0; 32];
    for (bytes, value) in output.chunks_exact_mut(4).zip(hash) {
        bytes.copy_from_slice(&value.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_progress_renders_compact_steps_with_one_active_arrow() {
        let progress = ReleaseProgress::for_test(false, false);
        let mut state = progress.state.borrow_mut();
        state.stages[ReleaseStage::Infos.index()] = stage_state(
            StageStatus::Complete,
            [("Check release environment", SubstepStatus::Complete, None)],
        );
        state.stages[ReleaseStage::Checks.index()] = stage_state(
            StageStatus::Complete,
            [("Tests", SubstepStatus::Complete, Some((223, 223)))],
        );
        state.stages[ReleaseStage::Preparation.index()] = stage_state(
            StageStatus::Active,
            [("Prepare release version", SubstepStatus::Active, None)],
        );
        state.current_stage = Some(ReleaseStage::Preparation);
        let rows = rendered_rows(&state);
        drop(state);

        let lines = rows
            .iter()
            .map(|row| progress.render_row(row))
            .collect::<Vec<_>>();
        let output = lines.join("\n");

        assert!(lines[0].starts_with("✔ Infos"));
        assert!(lines[1].starts_with("✔ Checks"));
        assert!(lines[1].contains("✔ Tests 223/223"));
        assert!(lines[2].starts_with("Preparation"));
        assert!(lines[2].contains("→ Prepare release version"));
        assert_eq!(lines.len(), 3);
        assert_eq!(output.matches('→').count(), 1);
        assert!(!output.contains('◈'));
        assert!(!output.contains('◆'));
        assert!(!output.contains('\u{1b}'));
    }

    #[test]
    fn release_progress_keeps_only_the_recent_substeps_for_active_stage() {
        let progress = ReleaseProgress::for_test(false, false);
        let mut state = progress.state.borrow_mut();
        state.stages[ReleaseStage::Checks.index()].status = StageStatus::Active;
        state.current_stage = Some(ReleaseStage::Checks);
        for index in 1..=6 {
            state.stages[ReleaseStage::Checks.index()]
                .substeps
                .push_back(SubstepState {
                    label: format!("Check {index}"),
                    status: if index == 6 {
                        SubstepStatus::Active
                    } else {
                        SubstepStatus::Complete
                    },
                    progress: None,
                });
        }
        let rows = rendered_rows(&state);
        drop(state);

        let output = rows
            .iter()
            .map(|row| progress.render_row(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!output.contains("Check 1"));
        for index in 2..=6 {
            assert!(output.contains(&format!("Check {index}")));
        }
        assert_eq!(output.matches('→').count(), 1);
    }

    #[test]
    fn release_progress_color_styles_the_current_path_without_changing_markers() {
        let progress = ReleaseProgress::for_test(true, true);
        let mut state = progress.state.borrow_mut();
        state.stages[ReleaseStage::Build.index()] = stage_state(
            StageStatus::Active,
            [("Build executable", SubstepStatus::Active, Some((1, 2)))],
        );
        state.current_stage = Some(ReleaseStage::Build);
        let row = rendered_rows(&state).into_iter().next().unwrap();
        drop(state);

        let line = progress.render_row(&row);
        assert!(line.contains("\u{1b}["));
        assert!(line.contains("Build"));
        assert!(line.contains("→ Build executable"));
        assert!(line.contains("Build executable 1/2"));
    }

    #[test]
    fn release_progress_plain_rendering_has_no_terminal_controls() {
        let progress = ReleaseProgress::for_test(false, false);
        let mut state = ReleaseProgressState::new();
        state.stages[ReleaseStage::Checks.index()] = stage_state(
            StageStatus::Complete,
            [(
                "Run workspace tests",
                SubstepStatus::Complete,
                Some((223, 223)),
            )],
        );
        let row = rendered_rows(&state).into_iter().next().unwrap();

        assert!(!progress.render_row(&row).contains('\u{1b}'));
    }

    #[test]
    fn test_progress_parsers_count_listed_and_completed_tests() {
        let listed = "47 tests, 0 benchmarks\n101 tests, 0 benchmarks\n43 tests, 0 benchmarks\n8 tests, 0 benchmarks\n1 test, 0 benchmarks\n1 test, 0 benchmarks\n22 tests, 0 benchmarks\n";
        assert_eq!(parse_test_list_count(listed), Some(223));
        assert!(parse_test_completion(b"test runtime::works ... ok\n"));
        assert!(parse_test_completion(b"test runtime::fails ... FAILED\n"));
        assert!(parse_test_completion(
            b"test runtime::ignored ... ignored\n"
        ));
        assert!(!parse_test_completion(b"test result: ok. 1 passed\n"));
    }

    type TestSubstep = (&'static str, SubstepStatus, Option<(usize, usize)>);

    fn stage_state<const N: usize>(status: StageStatus, substeps: [TestSubstep; N]) -> StageState {
        StageState {
            status,
            substeps: substeps
                .into_iter()
                .map(|(label, status, progress)| SubstepState {
                    label: label.to_owned(),
                    status,
                    progress,
                })
                .collect(),
        }
    }

    #[test]
    fn command_diagnostics_preserve_both_child_streams() {
        assert_eq!(
            command_diagnostics(b"stdout detail\n", b"stderr detail\n"),
            "\nstdout:\nstdout detail\nstderr:\nstderr detail"
        );
        assert_eq!(command_diagnostics(b"", b"\n"), "");
    }

    #[test]
    fn version_prompt_accepts_the_suggested_value_when_empty() {
        let suggested = ReleaseVersion::parse("0.3.18").unwrap();

        assert_eq!(parse_version_input("", suggested).unwrap(), suggested);
        assert_eq!(parse_version_input("   ", suggested).unwrap(), suggested);
    }

    #[test]
    fn version_prompt_accepts_an_edited_middle_number() {
        let suggested = ReleaseVersion::parse("0.3.18").unwrap();

        assert_eq!(
            parse_version_input("0.7.18", suggested).unwrap(),
            ReleaseVersion::parse("0.7.18").unwrap()
        );
    }

    #[test]
    fn version_prompt_accepts_edited_major_and_patch_values() {
        let suggested = ReleaseVersion::parse("0.3.18").unwrap();

        assert_eq!(
            parse_version_input("1.3.18", suggested).unwrap(),
            ReleaseVersion::parse("1.3.18").unwrap()
        );
        assert_eq!(
            parse_version_input("0.3.19", suggested).unwrap(),
            ReleaseVersion::parse("0.3.19").unwrap()
        );
    }

    #[test]
    fn version_suggestions_offer_patch_minor_and_major_from_release_history() {
        let current = ReleaseVersion::parse("0.2.18").unwrap();
        let highest = Some(ReleaseVersion::parse("0.2.19").unwrap());

        assert_eq!(
            VersionSuggestions::new(current, highest).unwrap(),
            VersionSuggestions {
                patch: ReleaseVersion::parse("0.2.20").unwrap(),
                minor: ReleaseVersion::parse("0.3.0").unwrap(),
                major: ReleaseVersion::parse("1.0.0").unwrap(),
            }
        );
    }

    #[test]
    fn version_suggestions_start_from_a_source_version_ahead_of_history() {
        let current = ReleaseVersion::parse("1.4.2").unwrap();
        let highest = Some(ReleaseVersion::parse("1.3.9").unwrap());

        assert_eq!(
            VersionSuggestions::new(current, highest).unwrap(),
            VersionSuggestions {
                patch: ReleaseVersion::parse("1.4.3").unwrap(),
                minor: ReleaseVersion::parse("1.5.0").unwrap(),
                major: ReleaseVersion::parse("2.0.0").unwrap(),
            }
        );
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn embedded_payload_validation_skips_partial_marker_matches() {
        let directory = tempfile::tempdir().unwrap();
        let payload = (0..96_u8).collect::<Vec<_>>();
        let payload_path = directory.path().join("source.dll");
        let executable_path = directory.path().join("app.exe");
        fs::write(&payload_path, &payload).unwrap();
        let mut executable = payload[..64].to_vec();
        executable.extend_from_slice(b"not the payload");
        executable.extend_from_slice(&payload);
        fs::write(&executable_path, executable).unwrap();
        validate_embedded_payload(&executable_path, &payload_path).unwrap();
    }

    #[test]
    fn windows_sdk_version_normalization_is_exact() {
        assert_eq!(
            "10.0.22621.0\\".trim_end_matches(['\\', '/']),
            REQUIRED_WINDOWS_SDK
        );
        assert_ne!(
            "10.0.26100.0".trim_end_matches(['\\', '/']),
            REQUIRED_WINDOWS_SDK
        );
    }

    #[test]
    fn release_version_reuses_only_matching_current_version_and_checksum() {
        let digest = sha256(b"current build");
        let current = ReleaseVersion::parse("1.2.22").unwrap();
        let latest = (current, hex(&digest));

        assert_eq!(
            select_release_version(current, Some(&latest), &digest).unwrap(),
            current
        );
        assert_eq!(
            select_release_version(current, Some(&latest), &sha256(b"changed build")).unwrap(),
            ReleaseVersion::parse("1.2.23").unwrap()
        );
    }

    #[test]
    fn release_version_never_moves_behind_source_or_release_history() {
        let digest = sha256(b"current build");
        let history_ahead = (ReleaseVersion::parse("1.2.22").unwrap(), hex(&digest));
        assert_eq!(
            select_release_version(
                ReleaseVersion::parse("1.2.20").unwrap(),
                Some(&history_ahead),
                &digest,
            )
            .unwrap(),
            ReleaseVersion::parse("1.2.23").unwrap()
        );

        let source_ahead = ReleaseVersion::parse("2.0.0").unwrap();
        assert_eq!(
            select_release_version(source_ahead, Some(&history_ahead), &digest).unwrap(),
            source_ahead
        );
    }

    #[test]
    fn packaging_identity_is_stageswap() {
        assert_eq!(APP_PACKAGE, "stageswap");
        assert_eq!(MEDIA_SOURCE_PACKAGE, "stageswap-media-source");
        assert_eq!(APP_EXECUTABLE, "StageSwap.exe");
        assert_eq!(MEDIA_SOURCE_DLL, "stageswap_media_source.dll");
        assert_eq!(RELEASE_PREFIX, "StageSwap_win64_v");
        assert_eq!(RELEASE_SUFFIX, ".exe.sha256");
    }

    #[test]
    fn release_version_starts_at_application_version() {
        let current = ReleaseVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
        assert_eq!(
            select_release_version(current, None, &sha256(b"first build")).unwrap(),
            current
        );
    }

    #[test]
    fn webcam_switcher_history_does_not_affect_stageswap_versions() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory
                .path()
                .join("WebcamSwitcher_win64_v9.9.9.exe.sha256"),
            format!("{} *legacy.exe\n", hex(&sha256(b"legacy"))),
        )
        .unwrap();
        let latest = latest_release(directory.path()).unwrap();
        assert_eq!(
            select_release_version(
                ReleaseVersion::parse("0.2.0").unwrap(),
                latest.as_ref(),
                &sha256(b"first StageSwap build"),
            )
            .unwrap(),
            ReleaseVersion::parse("0.2.0").unwrap()
        );
    }

    #[test]
    fn legacy_release_sidecars_remain_valid_history() {
        let directory = tempfile::tempdir().unwrap();
        let digest = sha256(b"legacy metadata shape");
        fs::write(
            directory.path().join("StageSwap_win64_v3.4.5.exe.sha256"),
            format!("{} *StageSwap_win64_v3.4.5.exe\n", hex(&digest)),
        )
        .unwrap();
        assert_eq!(
            latest_release(directory.path()).unwrap(),
            Some((ReleaseVersion::parse("3.4.5").unwrap(), hex(&digest)))
        );
    }

    #[test]
    fn workspace_version_update_preserves_manifest_and_rolls_back_lockfile() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("Cargo.toml");
        let lockfile = directory.path().join("Cargo.lock");
        let original_manifest = "[workspace]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"0.2.0\" # release\nedition = \"2024\"\n";
        let original_lock = b"original lock";
        fs::write(&manifest, original_manifest).unwrap();
        fs::write(&lockfile, original_lock).unwrap();

        {
            let _transaction = VersionTransaction::begin(
                &manifest,
                &lockfile,
                ReleaseVersion::parse("0.2.11").unwrap(),
            )
            .unwrap();
            assert_eq!(
                read_workspace_version(&manifest).unwrap(),
                ReleaseVersion::parse("0.2.11").unwrap()
            );
            fs::write(&lockfile, "regenerated lock").unwrap();
        }

        assert_eq!(fs::read_to_string(&manifest).unwrap(), original_manifest);
        assert_eq!(fs::read(&lockfile).unwrap(), original_lock);
    }

    #[test]
    fn committed_workspace_version_and_regenerated_lockfile_are_retained() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("Cargo.toml");
        let lockfile = directory.path().join("Cargo.lock");
        fs::write(&manifest, "[workspace.package]\nversion = \"0.2.0\"\n").unwrap();
        fs::write(&lockfile, "original lock").unwrap();
        let mut transaction = VersionTransaction::begin(
            &manifest,
            &lockfile,
            ReleaseVersion::parse("0.2.11").unwrap(),
        )
        .unwrap();
        fs::write(&lockfile, "regenerated lock").unwrap();
        transaction.commit();
        drop(transaction);

        assert_eq!(
            read_workspace_version(&manifest).unwrap(),
            ReleaseVersion::parse("0.2.11").unwrap()
        );
        assert_eq!(fs::read_to_string(&lockfile).unwrap(), "regenerated lock");
    }

    #[test]
    fn release_publication_refuses_to_replace_different_existing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = "StageSwap_win64_v1.0.0.exe";
        fs::write(directory.path().join(artifact), b"existing").unwrap();
        assert!(publish_release(directory.path(), artifact, b"new", b"checksum").is_err());
        assert_eq!(
            fs::read(directory.path().join(artifact)).unwrap(),
            b"existing"
        );
        assert!(!directory.path().join(format!("{artifact}.sha256")).exists());
    }

    #[test]
    fn production_build_commands_and_artifact_paths_are_release_only() {
        let workspace = Path::new("workspace");
        let command = cargo_build(workspace, APP_PACKAGE, "test-target", None);
        let description = format!("{command:?}");
        assert!(description.contains("--release"));
        assert!(
            release_artifact(workspace, "test-target", APP_EXECUTABLE)
                .ends_with("target/test-target/release/StageSwap.exe")
        );
    }
}
