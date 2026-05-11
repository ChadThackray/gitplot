use crate::chart::LocChart;
use crate::git_walk::{self, WalkMessage};
use crate::types::RepoData;
use chrono::NaiveDate;
use iced::futures::SinkExt;
use iced::widget::{button, checkbox, column, container, row, scrollable, text, Space};
use iced::{Element, Length, Subscription, Task};
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
    chart: Option<LocChart>,
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
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let top = row![
            button(text("Open repo")).on_press(Message::OpenClicked),
            Space::with_width(Length::Fixed(12.0)),
            text(
                self.current_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "no repo loaded".to_string())
            ),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8);

        let status_line: Element<'_, Message> = match &self.status {
            Status::Idle => text("").into(),
            Status::Loading { processed, total } => {
                if *total == 0 {
                    text("Walking history…").into()
                } else {
                    text(format!("Walking commits {processed}/{total}")).into()
                }
            }
            Status::Loaded => {
                let n = self
                    .repo_data
                    .as_ref()
                    .map(|d| d.snapshots.len())
                    .unwrap_or(0);
                text(format!("Loaded — {n} day(s)")).into()
            }
            Status::Error(e) => text(format!("Error: {e}")).color(iced::Color::from_rgb(0.85, 0.2, 0.2)).into(),
        };

        let body: Element<'_, Message> = match (&self.status, &self.repo_data, &self.chart) {
            (Status::Loaded, Some(_), Some(chart)) => {
                let sidebar = self.sidebar();
                row![
                    container(sidebar)
                        .width(Length::Fixed(240.0))
                        .height(Length::Fill)
                        .padding(8),
                    container(chart.view())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .padding(8),
                ]
                .into()
            }
            _ => container(text("Open a git repository to begin."))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into(),
        };

        column![
            container(top).padding(8),
            container(status_line).padding([0, 8]),
            body,
        ]
        .spacing(4)
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run(worker_subscription)
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let latest_loc = self.latest_loc_per_extension();
        let mut col = column![text("File extensions").size(16)].spacing(4);
        for (ext, enabled) in &self.enabled {
            let count = latest_loc.get(ext).copied().unwrap_or(0);
            let label = format!(".{ext}  ({})", short_count(count));
            let ext_owned = ext.clone();
            col = col.push(
                checkbox(label, *enabled)
                    .on_toggle(move |b| Message::ExtensionToggled(ext_owned.clone(), b)),
            );
        }
        scrollable(col).height(Length::Fill).into()
    }

    fn rebuild_chart(&mut self) {
        let points = self.chart_points();
        self.chart = if points.is_empty() {
            None
        } else {
            Some(LocChart::new(points))
        };
    }

    fn chart_points(&self) -> Vec<(NaiveDate, u64)> {
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
