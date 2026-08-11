use anyhow::{Context, Result, bail};
use dialoguer::{
    Input, Select,
    console::Term,
    theme::{ColorfulTheme, SimpleTheme},
};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
#[cfg(test)]
use std::cmp::max;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

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

fn build_release(
    release_version: ReleaseVersion,
    progress: &ReleaseProgress,
    phase: &str,
) -> Result<PathBuf> {
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
    let (_, executable) = build_release_pair(&workspace, target, architecture, progress, phase)?;
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

#[derive(Clone, Copy)]
enum StepIcon {
    Repository,
    GitHub,
    Discovery,
    Version,
    Checks,
    Build,
    Package,
    Git,
    Publish,
}

impl StepIcon {
    fn glyph(self) -> &'static str {
        match self {
            Self::Repository => "◈",
            Self::GitHub => "◆",
            Self::Discovery => "ℹ",
            Self::Version => "✎",
            Self::Checks => "⚙",
            Self::Build => "⚒",
            Self::Package => "▣",
            Self::Git => "⇆",
            Self::Publish => "↗",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Repository => "blue",
            Self::GitHub => "magenta",
            Self::Discovery => "cyan",
            Self::Version => "yellow",
            Self::Checks => "blue",
            Self::Build => "yellow",
            Self::Package => "magenta",
            Self::Git => "cyan",
            Self::Publish => "green",
        }
    }
}

struct ReleaseProgress {
    interactive: bool,
    color: bool,
}

impl ReleaseProgress {
    fn new() -> Self {
        let interactive = Term::stderr().is_term();
        Self {
            interactive,
            color: interactive && env::var_os("NO_COLOR").is_none(),
        }
    }

    #[cfg(test)]
    fn for_test(interactive: bool, color: bool) -> Self {
        Self { interactive, color }
    }

    fn step<T>(
        &self,
        icon: StepIcon,
        label: &str,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let spinner = self.start(icon, label);
        let result = action();
        match result {
            Ok(value) => {
                self.finish(spinner, icon, label, true);
                Ok(value)
            }
            Err(error) => {
                self.finish(spinner, icon, label, false);
                Err(error.context(format!("{label} failed")))
            }
        }
    }

    fn command(&self, icon: StepIcon, label: &str, command: Command) -> Result<()> {
        self.step(icon, label, || run(command))
    }

    fn command_counted(
        &self,
        icon: StepIcon,
        group: &str,
        index: usize,
        total: usize,
        label: &str,
        command: Command,
    ) -> Result<()> {
        let label = format!("{group} [{index}/{total}] {label}");
        self.command(icon, &label, command)
    }

    fn start(&self, icon: StepIcon, label: &str) -> Option<ProgressBar> {
        if !self.interactive {
            return None;
        }

        let template = if self.color {
            format!("{{spinner:.{}}}{{msg}}", icon.color())
        } else {
            "{spinner}{msg}".to_owned()
        };
        let style = ProgressStyle::with_template(&template)
            .expect("release progress template must remain valid")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", ""]);
        let spinner = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
        spinner.set_style(style);
        spinner.set_message(format!(" {} {label}", icon.glyph()));
        spinner.enable_steady_tick(Duration::from_millis(80));
        Some(spinner)
    }

    fn finish(&self, spinner: Option<ProgressBar>, icon: StepIcon, label: &str, success: bool) {
        let message = self.status_line(icon, label, success);
        if let Some(spinner) = spinner {
            spinner.finish_and_clear();
        }
        eprintln!("{message}");
    }

    fn status_line(&self, icon: StepIcon, label: &str, success: bool) -> String {
        let marker = if success { "✔" } else { "✖" };
        let marker = if self.color {
            let styled = if success {
                Term::stderr().style().green().apply_to(marker)
            } else {
                Term::stderr().style().red().apply_to(marker)
            };
            styled.force_styling(true).to_string()
        } else {
            marker.to_owned()
        };
        let icon = if self.color {
            let styled = match icon.color() {
                "blue" => Term::stderr().style().blue().apply_to(icon.glyph()),
                "cyan" => Term::stderr().style().cyan().apply_to(icon.glyph()),
                "green" => Term::stderr().style().green().apply_to(icon.glyph()),
                "magenta" => Term::stderr().style().magenta().apply_to(icon.glyph()),
                "yellow" => Term::stderr().style().yellow().apply_to(icon.glyph()),
                _ => Term::stderr().style().apply_to(icon.glyph()),
            };
            styled.force_styling(true).to_string()
        } else {
            icon.glyph().to_owned()
        };
        let error = if success {
            String::new()
        } else if self.color {
            let styled = Term::stderr().style().red().apply_to("— error");
            format!(" {}", styled.force_styling(true))
        } else {
            " — error".to_owned()
        };
        format!("{marker} {icon} {label}{error}")
    }
}

fn publish_release_cli() -> Result<()> {
    let progress = ReleaseProgress::new();
    let workspace = workspace_root();
    let branch = progress.step(StepIcon::Repository, "Check release environment", || {
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

    let track = prompt_track()?;
    if track == ReleaseTrack::Release && branch != "main" {
        bail!("stable releases must be published from main");
    }

    let (published, current) =
        progress.step(StepIcon::Discovery, "Load release context", || {
            let published = published_versions()?;
            let current = read_workspace_version(&workspace.join("Cargo.toml"))?;
            Ok((published, current))
        })?;
    let highest = published.iter().copied().max();
    let suggested = match highest {
        Some(highest) if current <= highest => highest.increment_patch()?,
        _ => current,
    };
    let version = prompt_version(suggested)?;
    if highest.is_some_and(|highest| version <= highest) {
        bail!("release version {version} must be newer than every published version");
    }
    let tag = format!("v{version}");
    if track == ReleaseTrack::Release {
        let expected = format!("RELEASE {tag}");
        let confirmation = prompt(&format!("Type '{expected}' to publish a stable release: "))?;
        if confirmation != expected {
            bail!("stable release confirmation did not match");
        }
    }
    let incomplete_draft = progress.step(StepIcon::GitHub, "Check release target", || {
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
                progress.step(StepIcon::Version, "Prepare release version", || {
                    let transaction = VersionTransaction::begin(&manifest, &lockfile, version)?;
                    run(cargo_metadata(&workspace))?;
                    Ok(transaction)
                })?,
            )
        } else {
            None
        };
        let verification =
            progress.step(StepIcon::Package, "Prepare verification package", || {
                tempfile::tempdir().context("create release verification directory")
            })?;
        let executable = build_release(version, &progress, "Build verification package")?;
        let verification_artifact =
            progress.step(StepIcon::Package, "Package verification build", || {
                package_release(&executable, version, verification.path(), &workspace)
            })?;
        progress.step(StepIcon::Package, "Validate verification package", || {
            validate_release_outputs(&verification_artifact, version)
        })?;

        let message = format!("Release {tag}");
        progress.step(StepIcon::Git, "Commit release version", || {
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
            StepIcon::Package,
            "Package release (reuse verified build)",
            || {
                clear_local_release_outputs(&output, version)?;
                package_release(&executable, version, &output, &workspace)
            },
        )?;
        progress.step(StepIcon::Package, "Validate release package", || {
            validate_release_outputs(&artifact, version)
        })?;
        progress.step(StepIcon::Publish, "Publish release", || {
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
            println!("published {tag} from {}", artifact.display());
            Ok(())
        }
        Err(error) => {
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

fn prompt_track() -> Result<ReleaseTrack> {
    if Term::stderr().is_term() {
        let selection = if env::var_os("NO_COLOR").is_none() {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Release track")
                .items(&["Development (default)", "Release"])
                .default(0)
                .interact_on(&Term::stderr())
        } else {
            Select::with_theme(&SimpleTheme)
                .with_prompt("Release track")
                .items(&["Development (default)", "Release"])
                .default(0)
                .interact_on(&Term::stderr())
        }
        .context("select release track")?;
        return match selection {
            0 => Ok(ReleaseTrack::Development),
            1 => Ok(ReleaseTrack::Release),
            _ => bail!("release track selection was out of range"),
        };
    }

    eprintln!("Release track:");
    eprintln!("  1. Development (default)");
    eprintln!("  2. Release");
    match prompt("Select [1]: ")?.as_str() {
        "" | "1" => Ok(ReleaseTrack::Development),
        "2" => Ok(ReleaseTrack::Release),
        _ => bail!("release track must be 1 or 2"),
    }
}

fn prompt_version(suggested: ReleaseVersion) -> Result<ReleaseVersion> {
    let value = if Term::stderr().is_term() {
        if env::var_os("NO_COLOR").is_none() {
            Input::<String>::with_theme(&ColorfulTheme::default())
                .with_prompt("Version")
                .with_initial_text(suggested.to_string())
                .interact_text()
                .context("read version prompt")?
        } else {
            Input::<String>::with_theme(&SimpleTheme)
                .with_prompt("Version")
                .with_initial_text(suggested.to_string())
                .interact_text()
                .context("read version prompt")?
        }
    } else {
        prompt(&format!("Version [{suggested}]: "))?
    };
    parse_version_input(&value, suggested)
}

fn parse_version_input(value: &str, suggested: ReleaseVersion) -> Result<ReleaseVersion> {
    if value.trim().is_empty() {
        Ok(suggested)
    } else {
        ReleaseVersion::parse(value.trim())
    }
}

fn prompt(message: &str) -> Result<String> {
    eprint!("{message}");
    io::stderr().flush().context("flush prompt")?;
    let mut value = String::new();
    io::stdin().read_line(&mut value).context("read prompt")?;
    eprintln!();
    Ok(value.trim().to_owned())
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
    progress.command_counted(
        StepIcon::Checks,
        "Run validation checks",
        1,
        TOTAL,
        "Check formatting",
        format,
    )?;
    let mut clippy = Command::new("cargo");
    clippy.current_dir(workspace).args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    progress.command_counted(
        StepIcon::Checks,
        "Run validation checks",
        2,
        TOTAL,
        "Run host Clippy",
        clippy,
    )?;
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
        StepIcon::Checks,
        "Run validation checks",
        3,
        TOTAL,
        "Run Windows-target Clippy",
        windows_clippy,
    )?;
    let mut tests = Command::new("cargo");
    tests
        .current_dir(workspace)
        .args(["test", "--workspace", "--all-targets"]);
    progress.command_counted(
        StepIcon::Checks,
        "Run validation checks",
        4,
        TOTAL,
        "Run workspace tests",
        tests,
    )
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

fn build_release_pair(
    workspace: &Path,
    target: &str,
    architecture: &str,
    progress: &ReleaseProgress,
    phase: &str,
) -> Result<(PathBuf, PathBuf)> {
    progress.command_counted(
        StepIcon::Build,
        phase,
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
        StepIcon::Build,
        phase,
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
    fn release_progress_plain_status_has_no_terminal_controls() {
        let progress = ReleaseProgress::for_test(false, false);

        assert_eq!(
            progress.status_line(StepIcon::Checks, "Run workspace tests", true),
            "✔ ⚙ Run workspace tests"
        );
        assert_eq!(
            progress.status_line(StepIcon::Checks, "Run workspace tests", false),
            "✖ ⚙ Run workspace tests — error"
        );
    }

    #[test]
    fn release_progress_honors_color_suppression_on_a_terminal() {
        let progress = ReleaseProgress::for_test(true, false);

        assert!(
            !progress
                .status_line(StepIcon::Checks, "Run workspace tests", true)
                .contains('\u{1b}')
        );
    }

    #[test]
    fn release_progress_color_status_styles_the_marker() {
        let progress = ReleaseProgress::for_test(true, true);
        let line = progress.status_line(StepIcon::Checks, "Run workspace tests", true);

        assert!(line.contains("\u{1b}["));
        assert!(line.contains("✔"));
        assert!(line.contains("Run workspace tests"));
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
