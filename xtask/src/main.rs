use anyhow::{Context, Result, bail};
use dialoguer::console::{Key, Term};
use std::cell::RefCell;
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

const STAGE_COLUMN_WIDTH: usize = 12;
const RELEASE_TITLE: &str = "StageSwap release";

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

    fn icon(self) -> &'static str {
        match self {
            Self::Infos => "ⓘ",
            Self::Checks => "✓",
            Self::Preparation => "⚙",
            Self::Build => "⚒",
            Self::Publish => "↗",
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

    fn substep_labels(self) -> &'static [&'static str] {
        match self {
            Self::Infos => &[
                "Check release environment",
                "Select release track",
                "Load release context",
                "Select version bump",
                "Check release target",
            ],
            Self::Checks => &[
                "Check formatting",
                "Run host Clippy",
                "Run Windows-target Clippy",
                "Run workspace tests",
            ],
            Self::Preparation => &["Prepare release version", "Prepare verification package"],
            Self::Build => &[
                "Build media-source DLL",
                "Build StageSwap executable",
                "Package verification build",
                "Validate verification package",
            ],
            Self::Publish => &[
                "Commit release version",
                "Package release (reuse verified build)",
                "Validate release package",
                "Publish release",
            ],
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
    Pending,
    Active,
    Complete,
    Skipped,
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
    substeps: Vec<SubstepState>,
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
            stages: std::array::from_fn(|index| StageState {
                status: StageStatus::Pending,
                substeps: ReleaseStage::ALL[index]
                    .substep_labels()
                    .iter()
                    .map(|label| SubstepState {
                        label: (*label).to_owned(),
                        status: SubstepStatus::Pending,
                        progress: None,
                    })
                    .collect(),
            }),
            current_stage: None,
            frame_lines: 0,
            prompt_lines: 0,
            prompt_active: false,
        }
    }
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
        if interactive {
            progress.refresh();
        } else {
            eprintln!("{RELEASE_TITLE}");
        }
        progress
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
                        for substep in &mut previous_state.substeps {
                            match substep.status {
                                SubstepStatus::Active => substep.status = SubstepStatus::Complete,
                                SubstepStatus::Pending => substep.status = SubstepStatus::Skipped,
                                SubstepStatus::Complete
                                | SubstepStatus::Skipped
                                | SubstepStatus::Failed => {}
                            }
                        }
                    }
                }
                state.current_stage = Some(stage);
                state.stages[stage.index()].status = StageStatus::Active;
            }
            let substeps = &mut state.stages[stage.index()].substeps;
            if let Some(substep) = substeps.iter_mut().find(|substep| substep.label == label) {
                substep.status = SubstepStatus::Active;
                substep.progress = progress;
            } else {
                substeps.push(SubstepState {
                    label: label.to_owned(),
                    status: SubstepStatus::Active,
                    progress,
                });
            }
        }
        self.refresh();
    }

    fn set_substep_progress(&self, completed: usize, total: usize) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(stage) = state.current_stage
                && let Some(substep) = state.stages[stage.index()]
                    .substeps
                    .iter_mut()
                    .rev()
                    .find(|substep| substep.status == SubstepStatus::Active)
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
            if let Some(substep) = stage_state
                .substeps
                .iter_mut()
                .rev()
                .find(|substep| substep.status == SubstepStatus::Active)
            {
                substep.status = if success {
                    SubstepStatus::Complete
                } else {
                    SubstepStatus::Failed
                };
            }
            if !success {
                stage_state.status = StageStatus::Failed;
            }
            (!self.interactive).then(|| render_stage_line(stage, stage_state, false))
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
                for substep in &mut state.stages[stage.index()].substeps {
                    match substep.status {
                        SubstepStatus::Active => substep.status = SubstepStatus::Complete,
                        SubstepStatus::Pending => substep.status = SubstepStatus::Skipped,
                        SubstepStatus::Complete
                        | SubstepStatus::Skipped
                        | SubstepStatus::Failed => {}
                    }
                }
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

    fn select_prompt_interactive<T: ToString>(&self, message: &str, items: &[T]) -> Result<usize> {
        self.begin_prompt();
        let term = Term::stderr();
        term.hide_cursor().context("hide release prompt cursor")?;
        let result = (|| {
            let mut selection = 0;
            loop {
                let choices = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let line = format!(
                            "{}{}",
                            if index == selection { "→ " } else { "  " },
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
                    })
                    .collect::<Vec<_>>();
                let separator = if message.ends_with('?') { " " } else { ": " };
                self.replace_prompt(&[format!("{message}{separator}{}", choices.join("   "))])?;
                match term.read_key().context("read release selection")? {
                    Key::ArrowRight => {
                        selection = (selection + 1) % items.len();
                    }
                    Key::ArrowLeft => {
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
    let term = Term::stderr();
    if state.frame_lines > 0 && term.clear_last_lines(state.frame_lines).is_err() {
        return;
    }
    let title = if color {
        Term::stderr()
            .style()
            .cyan()
            .bold()
            .apply_to(RELEASE_TITLE)
            .force_styling(true)
            .to_string()
    } else {
        RELEASE_TITLE.to_owned()
    };
    if term.write_line(&title).is_err() {
        return;
    }
    for stage in ReleaseStage::ALL {
        let stage_state = &state.stages[stage.index()];
        let line = render_stage_line(stage, stage_state, color);
        if term.write_line(&line).is_err() {
            return;
        }
    }
    let _ = term.flush();
    state.frame_lines = 1 + ReleaseStage::ALL.len();
}

fn render_stage_line(stage: ReleaseStage, stage_state: &StageState, color: bool) -> String {
    let stage_text = format!(
        "{} {:<width$}:",
        stage.icon(),
        stage.label(),
        width = STAGE_COLUMN_WIDTH
    );
    let children = stage_state
        .substeps
        .iter()
        .map(|substep| {
            let marker = match substep.status {
                SubstepStatus::Pending | SubstepStatus::Active | SubstepStatus::Skipped => "",
                SubstepStatus::Complete => "",
                SubstepStatus::Failed => "✖ ",
            };
            let label = short_substep_label(&substep.label);
            let label = match substep.progress {
                Some((completed, total)) => format!("{label} {completed}/{total}"),
                None => label.to_owned(),
            };
            format!("{marker}{label}")
        })
        .collect::<Vec<_>>();

    if !color {
        return format!("{stage_text}  {}", children.join(" → "));
    }

    let stage_label = match stage_state.status {
        StageStatus::Active => style_active_stage(stage, stage_text),
        StageStatus::Pending | StageStatus::Complete | StageStatus::Failed => {
            style_stage_category(stage, stage_text)
        }
    };
    let children = stage_state
        .substeps
        .iter()
        .zip(children)
        .map(|(substep, text)| style_substep(stage, substep.status, text))
        .collect::<Vec<_>>();
    format!("{stage_label}  {}", children.join(" → "))
}

fn style_active_stage(stage: ReleaseStage, text: String) -> String {
    let styled = match stage {
        ReleaseStage::Infos => Term::stderr().style().blue().bold().apply_to(text),
        ReleaseStage::Checks => Term::stderr().style().magenta().bold().apply_to(text),
        ReleaseStage::Preparation => Term::stderr().style().yellow().bold().apply_to(text),
        ReleaseStage::Build => Term::stderr().style().cyan().bold().apply_to(text),
        ReleaseStage::Publish => Term::stderr().style().green().bold().apply_to(text),
    };
    styled.force_styling(true).to_string()
}

fn style_stage_category(stage: ReleaseStage, text: String) -> String {
    let styled = match stage {
        ReleaseStage::Infos => Term::stderr().style().blue().apply_to(text),
        ReleaseStage::Checks => Term::stderr().style().magenta().apply_to(text),
        ReleaseStage::Preparation => Term::stderr().style().yellow().apply_to(text),
        ReleaseStage::Build => Term::stderr().style().cyan().apply_to(text),
        ReleaseStage::Publish => Term::stderr().style().green().apply_to(text),
    };
    styled.force_styling(true).to_string()
}

fn style_substep(stage: ReleaseStage, status: SubstepStatus, text: String) -> String {
    let styled = match status {
        SubstepStatus::Active => return style_active_stage(stage, text),
        SubstepStatus::Complete | SubstepStatus::Skipped => {
            Term::stderr().style().green().dim().apply_to(text)
        }
        SubstepStatus::Failed => Term::stderr().style().red().bold().apply_to(text),
        SubstepStatus::Pending => Term::stderr().style().dim().apply_to(text),
    };
    styled.force_styling(true).to_string()
}

fn short_substep_label(label: &str) -> &str {
    match label {
        "Check release environment" => "Environment",
        "Load release context" => "Context",
        "Check release target" => "Target",
        "Select release track" => "Track",
        "Select version bump" => "Bump",
        "Prepare release version" => "Version",
        "Prepare verification package" => "Verification",
        "Build media-source DLL" => "Media DLL",
        "Build StageSwap executable" => "App",
        "Package verification build" | "Package release (reuse verified build)" => "Package",
        "Validate verification package" | "Validate release package" => "Verify",
        "Commit release version" => "Commit",
        "Publish release" => "Publish",
        "Check formatting" => "Format",
        "Run host Clippy" => "Clippy",
        "Run Windows-target Clippy" => "Win Clippy",
        "Run workspace tests" => "Tests",
        _ => label,
    }
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
    let version = prompt_version(&progress, version_suggestions, track, highest)?;
    let tag = format!("v{version}");
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
            let message = format!("🎉 Release {tag} published successfully!");
            if Term::stdout().is_term() && env::var_os("NO_COLOR").is_none() {
                println!(
                    "{}",
                    Term::stdout()
                        .style()
                        .green()
                        .bold()
                        .apply_to(message)
                        .force_styling(true)
                );
            } else {
                println!("{message}");
            }
            println!("  {}", artifact.display());
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
    track: ReleaseTrack,
    highest: Option<ReleaseVersion>,
) -> Result<ReleaseVersion> {
    let items = [
        format!("Patch ({})", suggestions.patch),
        format!("Minor ({})", suggestions.minor),
        format!("Major ({})", suggestions.major),
        format!("Manual (edit {})", suggestions.patch),
    ];
    progress.step(ReleaseStage::Infos, "Select version bump", || {
        let selection = if Term::stderr().is_term() {
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
        }?;
        let version = match selection {
            0 => suggestions.patch,
            1 => suggestions.minor,
            2 => suggestions.major,
            3 => prompt_manual_version(progress, suggestions.patch)?,
            _ => bail!("version bump selection was out of range"),
        };
        if highest.is_some_and(|highest| version <= highest) {
            bail!("release version {version} must be newer than every published version");
        }
        if track == ReleaseTrack::Release
            && !confirm_prompt(
                progress,
                &format!("Publish v{version} as a stable release?"),
            )?
        {
            bail!("stable release confirmation declined");
        }
        Ok(version)
    })
}

fn prompt_manual_version(
    progress: &ReleaseProgress,
    suggested: ReleaseVersion,
) -> Result<ReleaseVersion> {
    let value = if Term::stderr().is_term() {
        progress.edit_prompt_interactive("Version: ", &suggested.to_string())?
    } else {
        read_prompt(&format!("Version [{suggested}]: "))?
    };
    parse_version_input(&value, suggested)
}

fn confirm_prompt(progress: &ReleaseProgress, message: &str) -> Result<bool> {
    let selection = if Term::stderr().is_term() {
        progress.select_prompt_interactive(message, &["No", "Yes"])?
    } else {
        eprintln!("{message} [y/N]");
        match read_prompt("Confirm: ")?.to_ascii_lowercase().as_str() {
            "" | "n" | "no" => 0,
            "y" | "yes" => 1,
            _ => bail!("confirmation must be yes or no"),
        }
    };
    Ok(selection == 1)
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
    let mut format = Command::new("cargo");
    format
        .current_dir(workspace)
        .args(["fmt", "--all", "--", "--check"]);
    progress.step(ReleaseStage::Checks, "Check formatting", || run(format))?;
    let mut clippy = Command::new("cargo");
    clippy.current_dir(workspace).args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    progress.step(ReleaseStage::Checks, "Run host Clippy", || run(clippy))?;
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
    progress.step(ReleaseStage::Checks, "Run Windows-target Clippy", || {
        run(windows_clippy)
    })?;
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
        ReleaseTrack::Development => format!("{tag} Beta"),
        ReleaseTrack::Release => tag.clone(),
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
    progress.step(ReleaseStage::Build, "Build media-source DLL", || {
        run(cargo_build(workspace, MEDIA_SOURCE_PACKAGE, target, None))
    })?;
    let dll = release_artifact(workspace, target, MEDIA_SOURCE_DLL);
    if !dll.is_file() {
        bail!("media-source build did not produce {}", dll.display());
    }
    validate_pe_path(&dll, architecture)?;
    let embedded_dll = dll
        .canonicalize()
        .context("canonicalize media-source DLL")?;
    progress.step(ReleaseStage::Build, "Build StageSwap executable", || {
        run(cargo_build(
            workspace,
            APP_PACKAGE,
            target,
            Some(&embedded_dll),
        ))
    })?;
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
