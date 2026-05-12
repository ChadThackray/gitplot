use crate::chart::TimeSeriesChart;
use crate::git_walk::{self, WalkMessage};
use crate::types::RepoData;
use chrono::NaiveDate;
use iced::futures::SinkExt;
use iced::widget::{button, checkbox, column, container, row, scrollable, text, Space};
use iced::{Background, Border, Color, Element, Length, Shadow, Subscription, Task, Theme, Vector};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::mpsc::{self, UnboundedSender};

#[derive(Debug, Clone)]
pub enum Message {
    OpenClicked,
    FolderPicked(Option<PathBuf>),
    WorkerReady(UnboundedSender<PathBuf>),
    WalkProgress { processed: usize, total: usize },
    WalkDone(Box<RepoData>),
    WalkFailed(String),
    ExtensionToggled(String, bool),
    SelectAllExtensions,
    DeselectAllExtensions,
    ViewSelected(ViewKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Loc,
    Commits,
}

#[derive(Debug)]
enum Status {
    Idle,
    Loading { processed: usize, total: usize },
    Loaded,
    Error(String),
}

pub struct App {
    status: Status,
    current_path: Option<PathBuf>,
    repo_data: Option<RepoData>,
    enabled: BTreeMap<String, bool>,
    worker_tx: Option<UnboundedSender<PathBuf>>,
    chart: Option<TimeSeriesChart>,
    view: ViewKind,
}

impl Default for App {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            current_path: None,
            repo_data: None,
            enabled: BTreeMap::new(),
            worker_tx: None,
            chart: None,
            view: ViewKind::Loc,
        }
    }
}

impl App {
    pub fn title(&self) -> String {
        match &self.current_path {
            Some(p) => format!("gitplot — {}", p.display()),
            None => "gitplot".to_string(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenClicked => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Pick a git repository")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::FolderPicked,
            ),
            Message::FolderPicked(None) => Task::none(),
            Message::FolderPicked(Some(path)) => {
                self.current_path = Some(path.clone());
                self.status = Status::Loading {
                    processed: 0,
                    total: 0,
                };
                self.repo_data = None;
                self.enabled.clear();
                if let Some(tx) = &self.worker_tx {
                    let _ = tx.send(path);
                }
                Task::none()
            }
            Message::WorkerReady(tx) => {
                self.worker_tx = Some(tx);
                Task::none()
            }
            Message::WalkProgress { processed, total } => {
                if matches!(self.status, Status::Loading { .. }) {
                    self.status = Status::Loading { processed, total };
                }
                Task::none()
            }
            Message::WalkDone(data) => {
                let mut enabled = BTreeMap::new();
                for ext in &data.all_extensions {
                    enabled.insert(ext.clone(), true);
                }
                self.enabled = enabled;
                self.repo_data = Some(*data);
                self.status = Status::Loaded;
                self.rebuild_chart();
                Task::none()
            }
            Message::WalkFailed(err) => {
                self.status = Status::Error(err);
                self.chart = None;
                Task::none()
            }
            Message::ExtensionToggled(ext, on) => {
                self.enabled.insert(ext, on);
                self.rebuild_chart();
                Task::none()
            }
            Message::SelectAllExtensions => {
                for v in self.enabled.values_mut() {
                    *v = true;
                }
                self.rebuild_chart();
                Task::none()
            }
            Message::DeselectAllExtensions => {
                for v in self.enabled.values_mut() {
                    *v = false;
                }
                self.rebuild_chart();
                Task::none()
            }
            Message::ViewSelected(view) => {
                if self.view != view {
                    self.view = view;
                    self.rebuild_chart();
                }
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let path_label: Element<'_, Message> = match self.current_path.as_ref() {
            Some(p) => text(p.display().to_string()).size(13).into(),
            None => text("no repo loaded")
                .size(13)
                .style(|t: &Theme| text::Style {
                    color: Some(t.extended_palette().background.strong.color),
                })
                .into(),
        };

        let top = container(
            row![
                button(text("Open repo").size(14))
                    .padding([8, 16])
                    .on_press(Message::OpenClicked)
                    .style(accent_button),
                Space::with_width(Length::Fixed(14.0)),
                path_label,
            ]
            .align_y(iced::Alignment::Center)
            .spacing(8),
        )
        .padding([12, 20])
        .width(Length::Fill)
        .style(top_bar_style);

        let status_line: Element<'_, Message> = match &self.status {
            Status::Idle => Space::with_height(Length::Fixed(0.0)).into(),
            Status::Loading { processed, total } => {
                let msg = if *total == 0 {
                    "Walking history…".to_string()
                } else {
                    format!("Walking commits {processed}/{total}")
                };
                text(msg)
                    .size(12)
                    .style(|t: &Theme| text::Style {
                        color: Some(t.extended_palette().background.strong.color),
                    })
                    .into()
            }
            Status::Loaded => {
                let n = self
                    .repo_data
                    .as_ref()
                    .map(|d| d.snapshots.len())
                    .unwrap_or(0);
                text(format!("Loaded — {n} day(s)"))
                    .size(12)
                    .style(|t: &Theme| text::Style {
                        color: Some(t.extended_palette().success.base.color),
                    })
                    .into()
            }
            Status::Error(e) => text(format!("Error: {e}"))
                .size(12)
                .style(|t: &Theme| text::Style {
                    color: Some(t.extended_palette().danger.base.color),
                })
                .into(),
        };

        let body: Element<'_, Message> = match (&self.status, &self.repo_data, &self.chart) {
            (Status::Loaded, Some(_), Some(chart)) => {
                let nav = container(self.nav_rail())
                    .width(Length::Fixed(160.0))
                    .height(Length::Fill)
                    .padding(12)
                    .style(card_style);
                let chart_card = container(chart.view())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(12)
                    .style(card_style);
                let mut body_row = row![nav].spacing(12);
                if self.view == ViewKind::Loc {
                    body_row = body_row.push(
                        container(self.sidebar())
                            .width(Length::Fixed(240.0))
                            .height(Length::Fill)
                            .padding(16)
                            .style(card_style),
                    );
                }
                body_row.push(chart_card).into()
            }
            _ => container(
                text("Open a git repository to begin.")
                    .size(16)
                    .style(|t: &Theme| text::Style {
                        color: Some(t.extended_palette().background.strong.color),
                    }),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into(),
        };

        let content = column![
            top,
            container(status_line).padding([4, 20]),
            container(body)
                .padding([4, 20])
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(0);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(app_background)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run(worker_subscription)
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let latest_loc = self.latest_loc_per_extension();
        let header = text("File extensions")
            .size(13)
            .style(|t: &Theme| text::Style {
                color: Some(t.extended_palette().background.strong.color),
            });
        let bulk_actions = row![
            button(text("All").size(12))
                .padding([4, 10])
                .on_press(Message::SelectAllExtensions)
                .style(subtle_button),
            button(text("None").size(12))
                .padding([4, 10])
                .on_press(Message::DeselectAllExtensions)
                .style(subtle_button),
        ]
        .spacing(6);
        let mut col = column![
            header,
            Space::with_height(Length::Fixed(4.0)),
            bulk_actions,
            Space::with_height(Length::Fixed(4.0)),
        ]
        .spacing(6);
        for (ext, enabled) in &self.enabled {
            let count = latest_loc.get(ext).copied().unwrap_or(0);
            let label = format!(".{ext}   {}", short_count(count));
            let ext_owned = ext.clone();
            col = col.push(
                checkbox(label, *enabled)
                    .size(16)
                    .spacing(8)
                    .text_size(13)
                    .on_toggle(move |b| Message::ExtensionToggled(ext_owned.clone(), b)),
            );
        }
        scrollable(col).height(Length::Fill).into()
    }

    fn rebuild_chart(&mut self) {
        let (points, unit_label) = match self.view {
            ViewKind::Loc => (self.loc_chart_points(), "LOC"),
            ViewKind::Commits => (self.commits_chart_points(), "commits"),
        };
        self.chart = if points.is_empty() {
            None
        } else {
            Some(TimeSeriesChart::new(points, unit_label))
        };
    }

    fn loc_chart_points(&self) -> Vec<(NaiveDate, u64)> {
        let Some(data) = &self.repo_data else {
            return Vec::new();
        };
        data.snapshots
            .iter()
            .map(|s| {
                let total: u64 = s
                    .files
                    .iter()
                    .filter(|f| self.enabled.get(&f.extension).copied().unwrap_or(true))
                    .map(|f| f.code_lines)
                    .sum();
                (s.date, total)
            })
            .collect()
    }

    fn commits_chart_points(&self) -> Vec<(NaiveDate, u64)> {
        let Some(data) = &self.repo_data else {
            return Vec::new();
        };
        let mut running: u64 = 0;
        data.commits_per_day
            .iter()
            .map(|(d, n)| {
                running += u64::from(*n);
                (*d, running)
            })
            .collect()
    }

    fn nav_rail(&self) -> Element<'_, Message> {
        let header = text("Views")
            .size(13)
            .style(|t: &Theme| text::Style {
                color: Some(t.extended_palette().background.strong.color),
            });
        column![
            header,
            Space::with_height(Length::Fixed(6.0)),
            nav_button("Lines of code", ViewKind::Loc, self.view),
            nav_button("Commits", ViewKind::Commits, self.view),
        ]
        .spacing(6)
        .into()
    }

    fn latest_loc_per_extension(&self) -> BTreeMap<String, u64> {
        let mut out = BTreeMap::new();
        if let Some(data) = &self.repo_data {
            if let Some(latest) = data.snapshots.last() {
                for fs in &latest.files {
                    *out.entry(fs.extension.clone()).or_insert(0) += fs.code_lines;
                }
            }
        }
        out
    }
}

fn app_background(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme.extended_palette().background.base.color)),
        ..container::Style::default()
    }
}

fn top_bar_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(mix(
            palette.background.base.color,
            palette.background.weak.color,
            0.5,
        ))),
        border: Border {
            color: palette.background.weak.color,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

fn card_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(mix(
            palette.background.base.color,
            palette.background.weak.color,
            0.4,
        ))),
        border: Border {
            color: palette.background.weak.color,
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow {
            color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.25 },
            offset: Vector::new(0.0, 2.0),
            blur_radius: 8.0,
        },
        ..container::Style::default()
    }
}

fn accent_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let (bg, fg) = match status {
        button::Status::Active => (palette.primary.base.color, palette.primary.base.text),
        button::Status::Hovered => (palette.primary.strong.color, palette.primary.strong.text),
        button::Status::Pressed => (palette.primary.weak.color, palette.primary.weak.text),
        button::Status::Disabled => (palette.background.weak.color, palette.background.strong.color),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
    }
}

fn nav_button(label: &str, kind: ViewKind, current: ViewKind) -> Element<'_, Message> {
    let active = kind == current;
    let style = if active { accent_button } else { subtle_button };
    button(text(label.to_string()).size(13))
        .padding([8, 12])
        .width(Length::Fill)
        .on_press(Message::ViewSelected(kind))
        .style(style)
        .into()
}

fn subtle_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let (bg, fg) = match status {
        button::Status::Active => (palette.background.weak.color, palette.background.base.text),
        button::Status::Hovered => (
            mix(palette.background.weak.color, palette.background.strong.color, 0.4),
            palette.background.base.text,
        ),
        button::Status::Pressed => (palette.background.strong.color, palette.background.base.text),
        button::Status::Disabled => (palette.background.weak.color, palette.background.strong.color),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 8.0.into(),
        },
        shadow: Shadow::default(),
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r * (1.0 - t) + b.r * t,
        g: a.g * (1.0 - t) + b.g * t,
        b: a.b * (1.0 - t) + b.b * t,
        a: a.a * (1.0 - t) + b.a * t,
    }
}

fn short_count(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}k", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
}

fn worker_subscription() -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(64, |mut output| async move {
        let (req_tx, mut req_rx) = mpsc::unbounded_channel::<PathBuf>();
        if output.send(Message::WorkerReady(req_tx)).await.is_err() {
            return;
        }

        while let Some(path) = req_rx.recv().await {
            let (walk_tx, mut walk_rx) = mpsc::unbounded_channel::<WalkMessage>();
            std::thread::spawn(move || {
                git_walk::analyze(path, walk_tx);
            });

            while let Some(msg) = walk_rx.recv().await {
                let (m, done) = match msg {
                    WalkMessage::Progress { processed, total } => {
                        (Message::WalkProgress { processed, total }, false)
                    }
                    WalkMessage::Done(data) => (Message::WalkDone(data), true),
                    WalkMessage::Failed(e) => (Message::WalkFailed(e), true),
                };
                if output.send(m).await.is_err() {
                    return;
                }
                if done {
                    break;
                }
            }
        }
    })
}
