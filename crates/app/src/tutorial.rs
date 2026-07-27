use crate::ui_icon::UiIcon;
use eframe::egui::{Pos2, Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

const UI_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TutorialStep {
    Welcome,
    Automatic,
    Previews,
    Health,
    Controls,
    Prepare,
    Ready,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TutorialDetail {
    pub(crate) icon: UiIcon,
    pub(crate) title: &'static str,
    pub(crate) explanation: &'static str,
    pub(crate) accent: TutorialAccent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TutorialAccent {
    Blue,
    Green,
    Amber,
    Red,
}

macro_rules! tutorial_detail {
    ($icon:expr, $title:expr, $explanation:expr $(,)?) => {
        TutorialDetail {
            icon: $icon,
            title: $title,
            explanation: $explanation,
            accent: TutorialAccent::Blue,
        }
    };
    ($icon:expr, $title:expr, $explanation:expr, $accent:expr $(,)?) => {
        TutorialDetail {
            icon: $icon,
            title: $title,
            explanation: $explanation,
            accent: $accent,
        }
    };
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TutorialParagraph {
    pub(crate) heading: &'static str,
    pub(crate) explanation: &'static str,
}

impl TutorialParagraph {
    pub(crate) fn accent(self) -> TutorialAccent {
        if self.heading.starts_with("Green ") {
            TutorialAccent::Green
        } else if self.heading.starts_with("Red ") {
            TutorialAccent::Red
        } else {
            TutorialAccent::Blue
        }
    }
}

impl TutorialStep {
    pub(crate) const ALL: [Self; 7] = [
        Self::Welcome,
        Self::Automatic,
        Self::Previews,
        Self::Health,
        Self::Controls,
        Self::Prepare,
        Self::Ready,
    ];

    pub(crate) const fn number(self) -> usize {
        match self {
            Self::Welcome => 1,
            Self::Automatic => 2,
            Self::Previews => 3,
            Self::Health => 4,
            Self::Controls => 5,
            Self::Prepare => 6,
            Self::Ready => 7,
        }
    }

    pub(crate) const fn from_number(number: usize) -> Option<Self> {
        match number {
            1 => Some(Self::Welcome),
            2 => Some(Self::Automatic),
            3 => Some(Self::Previews),
            4 => Some(Self::Health),
            5 => Some(Self::Controls),
            6 => Some(Self::Prepare),
            7 => Some(Self::Ready),
            _ => None,
        }
    }

    pub(crate) const fn previous(self) -> Option<Self> {
        Self::from_number(self.number().saturating_sub(1))
    }

    pub(crate) const fn next(self) -> Option<Self> {
        Self::from_number(self.number() + 1)
    }

    pub(crate) const fn is_last(self) -> bool {
        matches!(self, Self::Ready)
    }

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome to StageSwap",
            Self::Automatic => "Automatic chooses for you",
            Self::Previews => "Know the four previews",
            Self::Health => "Check what is ready",
            Self::Controls => "Control what people see",
            Self::Prepare => "Prepare the screen",
            Self::Ready => "Ready to use StageSwap",
        }
    }

    pub(crate) const fn icon(self) -> UiIcon {
        match self {
            Self::Welcome => UiIcon::Broadcast,
            Self::Automatic => UiIcon::Robot,
            Self::Previews => UiIcon::Window,
            Self::Health => UiIcon::Check,
            Self::Controls => UiIcon::Route,
            Self::Prepare => UiIcon::Capture,
            Self::Ready => UiIcon::Play,
        }
    }

    pub(crate) const fn paragraphs(self) -> &'static [TutorialParagraph] {
        match self {
            Self::Welcome => &[
                TutorialParagraph {
                    heading: "Choose StageSwap in Zoom",
                    explanation: "StageSwap creates a virtual camera for Zoom. Open Zoom’s camera list and select StageSwap instead of your physical webcam.",
                },
                TutorialParagraph {
                    heading: "One camera, two possible views",
                    explanation: "That single camera can show either your physical webcam or one selected display. StageSwap changes between them without starting Zoom’s screen-sharing mode.",
                },
                TutorialParagraph {
                    heading: "Private and silent",
                    explanation: "Camera and screen frames stay on this computer. StageSwap does not record them, upload them, or send audio.",
                },
            ],
            Self::Automatic => &[
                TutorialParagraph {
                    heading: "First, save a reference",
                    explanation: "The reference is a picture of the display when you want people in Zoom to see your webcam. It is usually an idle slide, holding image, or desktop background.",
                },
                TutorialParagraph {
                    heading: "StageSwap watches the picture",
                    explanation: "It compares the live display with the reference four times per second. It looks only at visual similarity; it does not read slide titles, app names, or text.",
                },
                TutorialParagraph {
                    heading: "The match chooses the view",
                    explanation: "A matching display selects the webcam. A changed display selects the screen. When the reference returns, StageSwap switches back to the webcam.",
                },
            ],
            Self::Previews => &[
                TutorialParagraph {
                    heading: "Follow the full video path",
                    explanation: "The four previews update live before and during a Zoom call. They let you confirm both selected inputs, the saved reference, and the final result.",
                },
                TutorialParagraph {
                    heading: "Green outline: active input",
                    explanation: "The green outline marks the webcam or screen currently feeding the result.",
                },
                TutorialParagraph {
                    heading: "Red outline: Zoom output",
                    explanation: "The red Output outline marks the feed sent to Zoom. FPS shows the live frame rate.",
                },
            ],
            Self::Health => &[
                TutorialParagraph {
                    heading: "Check the three components",
                    explanation: "Webcam, Screen, and Output show whether each part is ready. Check this area first if a preview is missing or Zoom shows no picture.",
                },
                TutorialParagraph {
                    heading: "Check the current decision",
                    explanation: "Detection reports whether the live screen matches the reference. Screen mix shows Webcam only, Screen only, or Crossfading while StageSwap moves between them.",
                },
            ],
            Self::Controls => &[
                TutorialParagraph {
                    heading: "Start or stop the output",
                    explanation: "Start automation makes the selected mode live in Zoom. Stop automation keeps the StageSwap camera available but replaces its picture with the black StageSwap off screen.",
                },
                TutorialParagraph {
                    heading: "Choose how StageSwap decides",
                    explanation: "Automatic, Webcam, and Screen are output modes. A manual Webcam or Screen choice stays selected until you choose another mode.",
                },
            ],
            Self::Prepare => &[
                TutorialParagraph {
                    heading: "Capture the idle view",
                    explanation: "Show the normal idle view on your selected display, then choose Capture reference. StageSwap saves that exact view as the picture Automatic mode should recognize.",
                },
                TutorialParagraph {
                    heading: "Find the correct display",
                    explanation: "Rescan screens searches connected displays for the saved reference. It helps StageSwap find the right display; it does not restart screen capture.",
                },
                TutorialParagraph {
                    heading: "Adjust the setup in Settings",
                    explanation: "Use Settings to choose the webcam and display, adjust matching, control startup behavior, or open recovery tools.",
                },
            ],
            Self::Ready => &[
                TutorialParagraph {
                    heading: "Complete the four checks below",
                    explanation: "StageSwap is ready once its two inputs are selected, a reference is saved, and StageSwap is chosen as the camera in Zoom.",
                },
                TutorialParagraph {
                    heading: "Return whenever you need help",
                    explanation: "You can reopen this tutorial from General Settings at any time. The tutorial never changes your devices, reference, mode, or automation state.",
                },
            ],
        }
    }

    pub(crate) const fn details(self) -> &'static [TutorialDetail] {
        match self {
            Self::Welcome => &[
                tutorial_detail!(
                    UiIcon::Broadcast,
                    "One camera in Zoom",
                    "Choose StageSwap from Zoom’s camera list.",
                ),
                tutorial_detail!(
                    UiIcon::Route,
                    "Two possible sources",
                    "StageSwap supplies the webcam or selected display.",
                ),
            ],
            Self::Automatic => &[
                tutorial_detail!(
                    UiIcon::Image,
                    "Reference",
                    "The saved display image that means “show my webcam.”",
                ),
                tutorial_detail!(
                    UiIcon::Camera,
                    "Reference matches",
                    "People in Zoom see the webcam.",
                ),
                tutorial_detail!(
                    UiIcon::Monitor,
                    "Display changes",
                    "People in Zoom see the selected screen.",
                ),
            ],
            Self::Previews => &[
                tutorial_detail!(
                    UiIcon::Camera,
                    "Webcam",
                    "The picture from the selected physical camera.",
                ),
                tutorial_detail!(
                    UiIcon::Monitor,
                    "Screen",
                    "The live picture from the selected display.",
                ),
                tutorial_detail!(
                    UiIcon::Image,
                    "Reference",
                    "The saved picture used by Automatic mode.",
                ),
                tutorial_detail!(
                    UiIcon::Broadcast,
                    "Output",
                    "Exactly what people in Zoom receive.",
                ),
            ],
            Self::Health => &[
                tutorial_detail!(
                    UiIcon::Check,
                    "Green",
                    "Ready, matching, or currently selected.",
                    TutorialAccent::Green,
                ),
                tutorial_detail!(
                    UiIcon::Loader,
                    "Amber",
                    "Starting, waiting, not matching, or changing.",
                    TutorialAccent::Amber,
                ),
                tutorial_detail!(
                    UiIcon::Error,
                    "Red",
                    "Unavailable, failed, or missing a reference.",
                    TutorialAccent::Red,
                ),
            ],
            Self::Controls => &[
                tutorial_detail!(
                    UiIcon::Robot,
                    "Automatic",
                    "Uses the reference to choose webcam or screen.",
                ),
                tutorial_detail!(UiIcon::Camera, "Webcam", "Keeps the webcam visible."),
                tutorial_detail!(
                    UiIcon::Monitor,
                    "Screen",
                    "Keeps the selected display visible.",
                ),
            ],
            Self::Prepare => &[
                tutorial_detail!(
                    UiIcon::Capture,
                    "Capture reference",
                    "Save the display’s current view.",
                ),
                tutorial_detail!(
                    UiIcon::Refresh,
                    "Rescan screens",
                    "Find the display containing the reference.",
                ),
                tutorial_detail!(
                    UiIcon::Settings,
                    "Settings",
                    "Choose devices, matching, behavior, and recovery.",
                ),
            ],
            Self::Ready => &[
                tutorial_detail!(
                    UiIcon::Settings,
                    "Choose inputs",
                    "Select the webcam and display in Settings.",
                ),
                tutorial_detail!(
                    UiIcon::Capture,
                    "Save the idle view",
                    "Show it on the display and capture the reference.",
                ),
                tutorial_detail!(
                    UiIcon::Broadcast,
                    "Choose StageSwap",
                    "Select it as the camera in Zoom.",
                ),
                tutorial_detail!(
                    UiIcon::Play,
                    "Go live",
                    "Return to the dashboard and start automation.",
                ),
            ],
        }
    }

    pub(crate) const fn callout_height(self) -> f32 {
        match self {
            Self::Welcome => 520.0,
            Self::Automatic => 548.0,
            Self::Previews => 548.0,
            Self::Health => 510.0,
            Self::Controls => 500.0,
            Self::Prepare => 530.0,
            Self::Ready => 548.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TutorialReturnView {
    Dashboard,
    Settings,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TutorialSession {
    pub(crate) step: TutorialStep,
    pub(crate) return_view: TutorialReturnView,
    pub(crate) opened_at: Option<Instant>,
    pub(crate) step_changed_at: Option<Instant>,
}

impl TutorialSession {
    pub(crate) fn live(return_view: TutorialReturnView) -> Self {
        let now = Instant::now();
        Self {
            step: TutorialStep::Welcome,
            return_view,
            opened_at: Some(now),
            step_changed_at: Some(now),
        }
    }

    #[cfg(any(not(windows), test))]
    pub(crate) const fn preview(step: TutorialStep) -> Self {
        Self {
            step,
            return_view: TutorialReturnView::Dashboard,
            opened_at: None,
            step_changed_at: None,
        }
    }

    pub(crate) fn go_to(&mut self, step: TutorialStep) {
        if self.step != step {
            self.step = step;
            self.step_changed_at = Some(Instant::now());
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TutorialUiState {
    schema_version: u32,
    tutorial_completed: bool,
}

impl Default for TutorialUiState {
    fn default() -> Self {
        Self {
            schema_version: UI_STATE_SCHEMA_VERSION,
            tutorial_completed: false,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TutorialStartup {
    pub(crate) show_tutorial: bool,
    pub(crate) warnings: Vec<String>,
}

impl TutorialStartup {
    #[cfg(not(windows))]
    pub(crate) const fn suppressed() -> Self {
        Self {
            show_tutorial: false,
            warnings: Vec::new(),
        }
    }
}

pub(crate) fn has_existing_user_data(directory: &Path) -> bool {
    fs::read_dir(directory).ok().is_some_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| name != "ui-state.json")
        })
    })
}

#[derive(Clone, Debug)]
pub(crate) struct TutorialStateStore {
    path: PathBuf,
}

impl TutorialStateStore {
    pub(crate) fn new(directory: impl AsRef<Path>) -> Self {
        Self {
            path: directory.as_ref().join("ui-state.json"),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn initialize(&self, existing_install: bool) -> TutorialStartup {
        if self.path.exists() {
            match self.load() {
                Ok(state) => {
                    return TutorialStartup {
                        show_tutorial: !state.tutorial_completed,
                        warnings: Vec::new(),
                    };
                }
                Err(error) => {
                    let state = TutorialUiState {
                        tutorial_completed: existing_install,
                        ..TutorialUiState::default()
                    };
                    let mut warnings = vec![format!(
                        "Could not read tutorial progress; using a safe default: {error}"
                    )];
                    if let Err(save_error) = self.save(&state) {
                        warnings.push(format!("Could not repair tutorial progress: {save_error}"));
                    }
                    return TutorialStartup {
                        show_tutorial: !state.tutorial_completed,
                        warnings,
                    };
                }
            }
        }

        let state = TutorialUiState {
            tutorial_completed: existing_install,
            ..TutorialUiState::default()
        };
        let mut warnings = Vec::new();
        if let Err(error) = self.save(&state) {
            warnings.push(format!("Could not save tutorial progress: {error}"));
        }
        TutorialStartup {
            show_tutorial: !state.tutorial_completed,
            warnings,
        }
    }

    pub(crate) fn mark_completed(&self) -> io::Result<()> {
        self.save(&TutorialUiState {
            tutorial_completed: true,
            ..TutorialUiState::default()
        })
    }

    fn load(&self) -> io::Result<TutorialUiState> {
        let json = fs::read_to_string(&self.path)?;
        let state = serde_json::from_str::<TutorialUiState>(&json)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if state.schema_version != UI_STATE_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported tutorial UI state schema",
            ));
        }
        Ok(state)
    }

    fn save(&self, state: &TutorialUiState) -> io::Result<()> {
        let Some(directory) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tutorial UI state has no parent directory",
            ));
        };
        fs::create_dir_all(directory)?;
        let mut bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        bytes.push(b'\n');
        fs::write(&self.path, bytes)
    }
}

pub(crate) fn callout_rect(content: Rect, target: Option<Rect>, desired: Vec2) -> Rect {
    const EDGE_MARGIN: f32 = 18.0;
    const TARGET_GAP: f32 = 20.0;

    let maximum = (content.size() - Vec2::splat(EDGE_MARGIN * 2.0)).max(Vec2::splat(1.0));
    let size = desired.min(maximum);
    let bounds = content.shrink(EDGE_MARGIN);
    let Some(target) = target else {
        return Rect::from_center_size(content.center(), size);
    };

    let right_space = bounds.right() - target.right() - TARGET_GAP;
    let left_space = target.left() - bounds.left() - TARGET_GAP;
    let x = if right_space >= size.x {
        target.right() + TARGET_GAP
    } else if left_space >= size.x {
        target.left() - TARGET_GAP - size.x
    } else if target.center().x <= content.center().x {
        bounds.right() - size.x
    } else {
        bounds.left()
    };
    let y = (target.center().y - size.y / 2.0)
        .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top()));
    Rect::from_min_size(Pos2::new(x, y), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tutorial_steps_have_stable_bidirectional_boundaries() {
        assert_eq!(TutorialStep::Welcome.previous(), None);
        assert_eq!(TutorialStep::Ready.next(), None);
        for (index, step) in TutorialStep::ALL.into_iter().enumerate() {
            assert_eq!(step.number(), index + 1);
            assert_eq!(TutorialStep::from_number(index + 1), Some(step));
        }
        assert_eq!(TutorialStep::from_number(0), None);
        assert_eq!(TutorialStep::from_number(8), None);
    }

    #[test]
    fn every_step_has_separate_explanations_and_icon_led_details() {
        let mut mentions_zoom = false;
        for step in TutorialStep::ALL {
            assert!(
                step.paragraphs().len() >= 2,
                "{step:?} needs at least two explanatory paragraphs"
            );
            assert!(
                step.details().len() >= 2,
                "{step:?} needs at least two icon-led details"
            );
            assert!(!step.icon().glyph().is_empty());
            for paragraph in step.paragraphs() {
                for text in [paragraph.heading, paragraph.explanation] {
                    mentions_zoom |= text.contains("Zoom");
                    assert!(!text.contains("Teams"));
                    assert!(!text.contains("meeting app"));
                    assert!(!text.contains("video app"));
                }
            }
            for detail in step.details() {
                assert!(!detail.icon.glyph().is_empty());
                assert!(!detail.title.is_empty());
                assert!(!detail.explanation.is_empty());
                for text in [detail.title, detail.explanation] {
                    mentions_zoom |= text.contains("Zoom");
                    assert!(!text.contains("Teams"));
                    assert!(!text.contains("meeting app"));
                    assert!(!text.contains("video app"));
                }
            }
        }
        assert!(mentions_zoom);

        let health = TutorialStep::Health.details();
        assert_eq!(health[0].accent, TutorialAccent::Green);
        assert_eq!(health[1].accent, TutorialAccent::Amber);
        assert_eq!(health[2].accent, TutorialAccent::Red);
    }

    #[test]
    fn fresh_install_starts_and_completion_is_persisted() {
        let directory = tempfile::tempdir().unwrap();
        let store = TutorialStateStore::new(directory.path());

        assert!(store.initialize(false).show_tutorial);
        store.mark_completed().unwrap();
        assert!(!store.initialize(false).show_tutorial);
    }

    #[test]
    fn existing_install_is_suppressed_without_changing_app_config() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.json");
        fs::write(&config_path, b"existing config").unwrap();
        let store = TutorialStateStore::new(directory.path());

        assert!(!store.initialize(config_path.exists()).show_tutorial);
        assert!(store.path().exists());
        assert_eq!(fs::read(config_path).unwrap(), b"existing config");
    }

    #[test]
    fn any_retained_user_data_counts_as_an_existing_install() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!has_existing_user_data(directory.path()));
        fs::write(directory.path().join("ui-state.json"), "bad").unwrap();
        assert!(!has_existing_user_data(directory.path()));
        fs::create_dir(directory.path().join("logs")).unwrap();
        assert!(has_existing_user_data(directory.path()));
    }

    #[test]
    fn malformed_state_uses_existing_install_as_the_safe_default() {
        let existing = tempfile::tempdir().unwrap();
        let existing_store = TutorialStateStore::new(existing.path());
        fs::write(existing_store.path(), "bad").unwrap();
        let existing_load = existing_store.initialize(true);
        assert!(!existing_load.show_tutorial);
        assert!(!existing_load.warnings.is_empty());

        let fresh = tempfile::tempdir().unwrap();
        let fresh_store = TutorialStateStore::new(fresh.path());
        fs::write(fresh_store.path(), "bad").unwrap();
        let fresh_load = fresh_store.initialize(false);
        assert!(fresh_load.show_tutorial);
        assert!(!fresh_load.warnings.is_empty());
    }

    #[test]
    fn unwritable_state_does_not_block_the_tutorial() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("not-a-directory");
        fs::write(&blocked, "file").unwrap();
        let store = TutorialStateStore::new(&blocked);

        let startup = store.initialize(false);
        assert!(startup.show_tutorial);
        assert_eq!(startup.warnings.len(), 1);
        assert!(store.mark_completed().is_err());
    }

    #[test]
    fn callouts_stay_inside_the_window_and_away_from_targets_when_space_allows() {
        for content in [
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0)),
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1067.0, 600.0)),
        ] {
            for target in [
                Rect::from_min_size(Pos2::new(40.0, 120.0), Vec2::new(620.0, 380.0)),
                Rect::from_min_size(Pos2::new(810.0, 120.0), Vec2::new(220.0, 180.0)),
            ] {
                let callout = callout_rect(content, Some(target), Vec2::new(300.0, 280.0));
                assert!(content.contains_rect(callout));
                if content.right() - target.right() >= 338.0
                    || target.left() - content.left() >= 338.0
                {
                    assert!(!callout.intersects(target));
                }
            }
        }
    }
}
