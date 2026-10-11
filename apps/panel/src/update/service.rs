//! UI-owned states consume bounded worker events; no worker joins on egui.
use super::worker::{self, VerifiedDownload};
use super::*;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// The last check outcome is separate from download/install state, so a failed check
/// can never be presented as proof that the installed version is current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheckStatus {
    NotChecked,
    Checking,
    UpToDate,
    Available,
    Failed,
}
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available {
        version: String,
    },
    Downloading {
        downloaded: u64,
        total: u64,
    },
    Verifying,
    Ready,
    Installing,
    Failed {
        error: UpdateError,
    },
}
pub(super) enum Event {
    Checked(Result<Option<Candidate>, UpdateError>),
    Progress { downloaded: u64, total: u64 },
    Verifying,
    Downloaded(Result<VerifiedDownload, UpdateError>),
    Installed(Result<worker::Handoff, UpdateError>),
}
#[derive(Default)]
pub struct UpdateService {
    state: UpdateState,
    candidate: Option<Candidate>,
    download: Option<VerifiedDownload>,
    receiver: Option<Receiver<Event>>,
    sender: Option<SyncSender<Event>>,
    last_check: Option<Instant>,
    check_in_flight: bool,
    last_check_result: Option<Result<(), UpdateError>>,
    check_task: Option<worker::CheckTask>,
    handoff: bool,
    updater: Option<std::process::Child>,
}
impl UpdateService {
    pub fn state(&self) -> &UpdateState {
        &self.state
    }
    pub fn check_status(&self) -> UpdateCheckStatus {
        if self.check_in_flight {
            UpdateCheckStatus::Checking
        } else {
            match self.last_check_result {
                None => UpdateCheckStatus::NotChecked,
                Some(Err(_)) => UpdateCheckStatus::Failed,
                Some(Ok(())) if self.candidate.is_some() => UpdateCheckStatus::Available,
                Some(Ok(())) => UpdateCheckStatus::UpToDate,
            }
        }
    }
    pub fn can_check(&self) -> bool {
        self.sender.is_some()
            && !self.check_in_flight
            && self.check_task.is_none()
            && !matches!(
                self.state,
                UpdateState::Downloading { .. }
                    | UpdateState::Verifying
                    | UpdateState::Ready
                    | UpdateState::Installing
            )
    }
    /// An explicit check bypasses the periodic interval, while retaining the same
    /// single-flight guard and background worker as startup checks.
    pub fn request_check(&mut self, ctx: &egui::Context) -> bool {
        self.begin_check_at(ctx, Instant::now(), worker::spawn_check)
    }
    pub fn start(&mut self, ctx: &egui::Context) {
        self.start_at(ctx, Instant::now(), worker::spawn_check);
    }
    fn start_at(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        spawn: impl FnOnce(SyncSender<Event>, egui::Context) -> std::io::Result<worker::CheckTask>,
    ) {
        if self.sender.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(8);
        self.sender = Some(tx);
        self.receiver = Some(rx);
        self.check_due_at(ctx, now, spawn);
    }
    /// Called by the existing UI loop. No timer thread and no network work on egui.
    pub fn tick(&mut self, ctx: &egui::Context) {
        self.poll();
        self.check_due_at(ctx, Instant::now(), worker::spawn_check);
    }
    fn check_due_at(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        spawn: impl FnOnce(SyncSender<Event>, egui::Context) -> std::io::Result<worker::CheckTask>,
    ) -> bool {
        if self
            .last_check
            .is_some_and(|last| now.saturating_duration_since(last) < CHECK_INTERVAL)
        {
            return false;
        }
        self.begin_check_at(ctx, now, spawn)
    }
    fn begin_check_at(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
        spawn: impl FnOnce(SyncSender<Event>, egui::Context) -> std::io::Result<worker::CheckTask>,
    ) -> bool {
        if !self.can_check() {
            return false;
        }
        let Some(tx) = self.sender.clone() else {
            return false;
        };
        // Record attempts, including spawn and HTTP failures, so they never retry every frame.
        self.last_check = Some(now);
        self.check_in_flight = true;
        if self.candidate.is_none()
            && !matches!(
                self.state,
                UpdateState::Failed {
                    error: UpdateError::Unsupported
                }
            )
        {
            self.state = UpdateState::Checking;
        }
        match spawn(tx, ctx.clone()) {
            Ok(task) => self.check_task = Some(task),
            Err(_) => self.finish_check(Err(UpdateError::Network)),
        }
        true
    }
    fn finish_check(&mut self, result: Result<Option<Candidate>, UpdateError>) {
        // A cancelled recheck may already have queued its result before the user clicked.
        if !self.check_in_flight {
            return;
        }
        self.check_in_flight = false;
        self.last_check_result = Some(result.as_ref().map(|_| ()).map_err(|error| *error));
        match result {
            Ok(candidate) => self.candidate = candidate,
            Err(error) => tracing::info!(code=%error,"Optional update check unavailable"),
        }
        if matches!(
            self.state,
            UpdateState::Failed {
                error: UpdateError::Unsupported
            }
        ) {
            return;
        }
        self.state = match &self.candidate {
            Some(candidate) => UpdateState::Available {
                version: candidate.version.to_string(),
            },
            None => UpdateState::Idle,
        };
    }
    fn revoke_handoff(&mut self) {
        self.handoff = false;
        self.updater = None;
        self.state = UpdateState::Failed {
            error: UpdateError::Updater,
        };
    }
    pub fn handoff_active(&self) -> bool {
        self.updater.is_some() && matches!(self.state, UpdateState::Installing)
    }
    pub fn poll(&mut self) {
        // Observe completion before draining: a finished worker has queued all of its events.
        let check_finished = self
            .check_task
            .as_ref()
            .is_some_and(worker::CheckTask::is_finished);
        if check_finished {
            self.check_task = None;
        }
        if self
            .updater
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)))
        {
            self.revoke_handoff();
        }
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().take(8).collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Checked(result) => self.finish_check(result),
                Event::Progress { downloaded, total } => {
                    self.state = UpdateState::Downloading { downloaded, total }
                }
                Event::Verifying => self.state = UpdateState::Verifying,
                Event::Downloaded(Ok(download)) => {
                    self.download = Some(download);
                    self.state = UpdateState::Ready;
                }
                Event::Downloaded(Err(error)) | Event::Installed(Err(error)) => {
                    tracing::warn!(code=%error,"Optional update failed");
                    self.handoff = false;
                    self.updater = None;
                    self.state = UpdateState::Failed { error };
                }
                Event::Installed(Ok(handoff)) => {
                    self.handoff = true;
                    self.updater = Some(handoff.child);
                }
            }
        }
        if check_finished && self.check_in_flight {
            // A panic/runtime setup failure is an unavailable check, with the same retry period.
            self.finish_check(Err(UpdateError::Network));
        }
    }
    fn begin_download(&mut self) -> Option<(Candidate, SyncSender<Event>)> {
        if matches!(
            self.state,
            UpdateState::Failed {
                error: UpdateError::Unsupported
            }
        ) {
            return None;
        }
        if !matches!(
            self.state,
            UpdateState::Available { .. } | UpdateState::Failed { .. }
        ) {
            return None;
        }
        let (Some(candidate), Some(tx)) = (self.candidate.clone(), self.sender.clone()) else {
            return None;
        };
        self.check_in_flight = false;
        if let Some(task) = &mut self.check_task {
            task.cancel();
        }
        self.state = UpdateState::Downloading {
            downloaded: 0,
            total: candidate.size,
        };
        Some((candidate, tx))
    }
    pub fn request_download(&mut self, ctx: &egui::Context) {
        let Some((candidate, tx)) = self.begin_download() else {
            return;
        };
        let wake = ctx.clone();
        if std::thread::Builder::new()
            .name("dji4g-update-download".into())
            .spawn(move || {
                worker::send(
                    &tx,
                    &wake,
                    Event::Downloaded(worker::download(&candidate, &tx, &wake)),
                )
            })
            .is_err()
        {
            self.state = UpdateState::Failed {
                error: UpdateError::Download,
            };
        }
    }
    pub fn request_install(&mut self, ctx: &egui::Context) {
        if !matches!(self.state, UpdateState::Ready) {
            return;
        }
        let (Some(download), Some(tx)) = (self.download.take(), self.sender.clone()) else {
            return;
        };
        self.state = UpdateState::Installing;
        let wake = ctx.clone();
        if std::thread::Builder::new()
            .name("dji4g-update-handoff".into())
            .spawn(move || worker::send(&tx, &wake, Event::Installed(worker::install(download))))
            .is_err()
        {
            self.state = UpdateState::Failed {
                error: UpdateError::Updater,
            };
        }
    }
    pub fn take_handoff(&mut self) -> bool {
        if self
            .updater
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)))
        {
            self.revoke_handoff();
        }
        std::mem::take(&mut self.handoff)
    }
}

#[cfg(test)]
pub(crate) fn test_driver(state: UpdateState) -> (UpdateService, impl Fn(super::TestEvent)) {
    let (tx, rx) = mpsc::sync_channel(8);
    let driver = tx.clone();
    (
        UpdateService {
            state,
            receiver: Some(rx),
            sender: Some(tx),
            ..Default::default()
        },
        move |event| {
            let event = match event {
                super::TestEvent::Ready => Event::Installed(Ok(worker::Handoff {
                    child: std::process::Command::new(std::env::current_exe().unwrap())
                        .arg("--list")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .unwrap(),
                })),
                super::TestEvent::Failed => Event::Installed(Err(UpdateError::Updater)),
            };
            driver.send(event).unwrap();
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_exited_updater_revokes_ready_before_panel_exit() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let mut service = UpdateService {
            state: UpdateState::Installing,
            receiver: Some(rx),
            ..Default::default()
        };
        tx.send(Event::Installed(Ok(worker::Handoff { child })))
            .unwrap();
        service.poll();
        assert!(
            !service.take_handoff(),
            "dead updater must never close Panel"
        );
        assert!(!service.handoff_active());
        assert!(matches!(
            service.state(),
            UpdateState::Failed {
                error: UpdateError::Updater
            }
        ));
    }
    #[test]
    fn updater_failure_revokes_pending_exit_handoff() {
        let (mut service, send) = test_driver(UpdateState::Installing);
        send(super::super::TestEvent::Ready);
        send(super::super::TestEvent::Failed);
        service.poll();
        assert!(matches!(
            service.state(),
            UpdateState::Failed {
                error: UpdateError::Updater
            }
        ));
        assert!(
            !service.take_handoff(),
            "must not exit after updater failure"
        );
    }
}

#[cfg(test)]
#[path = "periodic_tests.rs"]
mod periodic_tests;
