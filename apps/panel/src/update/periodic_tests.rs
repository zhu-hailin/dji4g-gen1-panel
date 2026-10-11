use super::*;

fn available() -> Candidate {
    Candidate {
        version: semver::Version::new(0, 1, 15),
        tag: "v0.1.15".into(),
        download_url: format!(
            "https://github.com/{REPOSITORY}/releases/download/v0.1.15/{PORTABLE_ASSET}"
        ),
        checksum_url: format!(
            "https://github.com/{REPOSITORY}/releases/download/v0.1.15/{PORTABLE_ASSET}.sha256"
        ),
        size: 100,
    }
}

#[test]
fn failed_periodic_check_keeps_previously_validated_update_available() {
    let (mut service, _) = test_driver(UpdateState::Available {
        version: "0.1.15".into(),
    });
    service.candidate = Some(available());
    service.check_in_flight = true;
    service
        .sender
        .as_ref()
        .unwrap()
        .send(Event::Checked(Err(UpdateError::Network)))
        .unwrap();
    service.poll();
    assert_eq!(
        service.state(),
        &UpdateState::Available {
            version: "0.1.15".into()
        }
    );
    assert_eq!(service.candidate, Some(available()));
}

#[test]
fn successful_no_update_result_discards_obsolete_candidate() {
    let (mut service, _) = test_driver(UpdateState::Available {
        version: "0.1.15".into(),
    });
    service.candidate = Some(available());
    service.check_in_flight = true;
    service
        .sender
        .as_ref()
        .unwrap()
        .send(Event::Checked(Ok(None)))
        .unwrap();
    service.poll();
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(service.candidate.is_none());
}

// Only replace the external HTTP job. Scheduling, state transitions and egui are real.
fn completed_job(_: SyncSender<Event>, _: egui::Context) -> std::io::Result<worker::CheckTask> {
    let (tx, _) = mpsc::sync_channel(8);
    let task = worker::CheckTask::spawn(tx, egui::Context::default(), async { Ok(None) })?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !task.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "test job failed to finish"
        );
        std::thread::yield_now();
    }
    Ok(task)
}

fn finish(service: &mut UpdateService, result: Result<Option<Candidate>, UpdateError>) {
    service
        .sender
        .as_ref()
        .unwrap()
        .send(Event::Checked(result))
        .unwrap();
    service.poll();
}

#[test]
fn startup_only_dispatches_once_and_unconfigured_service_does_not_check() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    assert!(!service.check_due_at(&ctx, now, |_, _| panic!("unconfigured")));
    service.start_at(&ctx, now, completed_job);
    assert_eq!(service.state(), &UpdateState::Checking);
    assert_eq!(service.last_check, Some(now));
    service.start_at(&ctx, now, |_, _| panic!("duplicate startup"));
    finish(&mut service, Ok(None));
    service.start_at(&ctx, now, |_, _| {
        panic!("startup repeated after completion")
    });
    assert_eq!(service.state(), &UpdateState::Idle);
}

#[test]
fn six_hour_boundary_dispatches_again_without_bursting_after_long_pause() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    finish(&mut service, Ok(None));
    for seconds in [0, 1, 3600, 21599] {
        assert!(!service.check_due_at(
            &ctx,
            now + std::time::Duration::from_secs(seconds),
            |_, _| panic!("too soon")
        ));
    }
    let due = now + std::time::Duration::from_secs(21600);
    assert!(service.check_due_at(&ctx, due, completed_job));
    finish(&mut service, Ok(None));
    let later = now + std::time::Duration::from_secs(21600 * 5);
    assert!(service.check_due_at(&ctx, later, completed_job));
    finish(&mut service, Ok(None));
    assert!(!service.check_due_at(&ctx, later, |_, _| panic!("catch-up burst")));
}

#[test]
fn in_flight_check_is_not_duplicated_even_when_six_hours_elapsed() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    assert!(!service.check_due_at(
        &ctx,
        now + std::time::Duration::from_secs(21600),
        |_, _| panic!("concurrent check")
    ));
}

#[test]
fn download_verification_ready_and_installation_pause_due_checks() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    for state in [
        UpdateState::Downloading {
            downloaded: 1,
            total: 100,
        },
        UpdateState::Verifying,
        UpdateState::Ready,
        UpdateState::Installing,
    ] {
        let (mut service, _) = test_driver(state.clone());
        service.last_check = Some(now);
        let due = now + std::time::Duration::from_secs(21600);
        assert!(!service.check_due_at(&ctx, due, |_, _| panic!("check during update")));
        assert_eq!(service.state(), &state);
        assert_eq!(service.last_check, Some(now));
        service.state = UpdateState::Idle;
        assert!(service.check_due_at(&ctx, due, completed_job));
    }
}

#[test]
fn failed_check_waits_full_cycle_and_spawn_failure_does_too() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    finish(&mut service, Err(UpdateError::Network));
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(!service.check_due_at(
        &ctx,
        now + std::time::Duration::from_secs(21599),
        |_, _| panic!("retry storm")
    ));
    let due = now + std::time::Duration::from_secs(21600);
    assert!(
        service.check_due_at(&ctx, due, |_, _| Err(std::io::Error::other(
            "spawn refused"
        )))
    );
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(!service.check_due_at(&ctx, due, |_, _| panic!("spawn retry storm")));
    assert!(service.check_due_at(
        &ctx,
        due + std::time::Duration::from_secs(21600),
        completed_job
    ));
}

#[test]
fn periodic_discovery_changes_icon_without_downloading_or_installing() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    finish(&mut service, Ok(None));
    assert!(service.check_due_at(
        &ctx,
        now + std::time::Duration::from_secs(21600),
        completed_job
    ));
    finish(&mut service, Ok(Some(available())));
    let output = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            assert!(
                crate::ui::update::indicator(
                    ui,
                    service.state(),
                    crate::localization::Language::ZhCn
                )
                .is_some()
            );
        });
    });
    assert!(output.platform_output.open_url.is_none());
    assert_eq!(
        service.state(),
        &UpdateState::Available {
            version: "0.1.15".into()
        }
    );
    assert!(service.download.is_none());
    assert!(!service.handoff_active());
    assert!(service.check_due_at(
        &ctx,
        now + std::time::Duration::from_secs(43200),
        completed_job
    ));
    assert_eq!(
        service.state(),
        &UpdateState::Available {
            version: "0.1.15".into()
        },
        "keep hint visible during recheck"
    );
    finish(&mut service, Err(UpdateError::Network));
    assert_eq!(
        service.state(),
        &UpdateState::Available {
            version: "0.1.15".into()
        }
    );
}

#[test]
fn cancelled_check_result_cannot_override_download_state_or_candidate() {
    let (mut service, _) = test_driver(UpdateState::Downloading {
        downloaded: 0,
        total: 100,
    });
    service.candidate = Some(available());
    // A user click has cancelled the background check before its queued result is drained.
    for result in [Ok(None), Ok(Some(available())), Err(UpdateError::Network)] {
        finish(&mut service, result);
        assert_eq!(
            service.state(),
            &UpdateState::Downloading {
                downloaded: 0,
                total: 100
            }
        );
        assert_eq!(service.candidate, Some(available()));
    }
}
#[test]
fn dropping_service_cancels_pending_check_without_waiting_on_ui() {
    struct Dropped(mpsc::Sender<()>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (started_tx, started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let ctx = egui::Context::default();
    let mut service = UpdateService::default();
    service.start_at(&ctx, std::time::Instant::now(), |tx, wake| {
        worker::CheckTask::spawn(tx, wake, async move {
            let _guard = Dropped(dropped_tx);
            started_tx.send(()).unwrap();
            std::future::pending().await
        })
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    service.tick(&ctx);
    assert_eq!(service.state(), &UpdateState::Checking);
    drop(service);
    dropped_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("check future must stop on exit");
}

#[test]
fn clicking_available_update_cancels_recheck_and_stale_result_is_ignored() {
    let ctx = egui::Context::default();
    let now = std::time::Instant::now();
    let mut service = UpdateService::default();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    struct Dropped(mpsc::Sender<()>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (started_tx, started_rx) = mpsc::channel();
    service.start_at(&ctx, now, |tx, wake| {
        worker::CheckTask::spawn(tx, wake, async move {
            let _guard = Dropped(cancelled_tx);
            started_tx.send(()).unwrap();
            std::future::pending().await
        })
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    service.candidate = Some(available());
    service.state = UpdateState::Available {
        version: "0.1.15".into(),
    };
    // No real download in this test: its terminal event is injected through the real channel.
    assert!(service.begin_download().is_some());
    assert!(!service.check_in_flight);
    cancelled_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    finish(&mut service, Ok(None));
    assert_eq!(
        service.state(),
        &UpdateState::Downloading {
            downloaded: 0,
            total: 100
        }
    );
    finish_download_failure(&mut service);
    assert_eq!(
        service.state(),
        &UpdateState::Failed {
            error: UpdateError::Download
        }
    );
    assert!(!service.check_due_at(
        &ctx,
        now + std::time::Duration::from_secs(21599),
        |_, _| panic!("early check")
    ));
}

fn finish_download_failure(service: &mut UpdateService) {
    service
        .sender
        .as_ref()
        .unwrap()
        .send(Event::Downloaded(Err(UpdateError::Download)))
        .unwrap();
    service.poll();
}

#[test]
fn completed_cancelled_worker_is_drained_before_another_check_can_start() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    service.check_in_flight = false;
    service.state = UpdateState::Idle;
    service
        .sender
        .as_ref()
        .unwrap()
        .send(Event::Checked(Ok(Some(available()))))
        .unwrap();
    let due = now + Duration::from_secs(21600);
    assert!(
        !service.check_due_at(&ctx, due, completed_job),
        "must drain cancelled result before reserving next check"
    );
    service.poll();
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(service.check_due_at(&ctx, due, completed_job));
}

#[test]
fn finished_worker_without_result_recovers_without_retry_storm() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    service.poll();
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(!service.check_in_flight);
    assert!(!service.check_due_at(&ctx, now, |_, _| panic!("immediate retry")));
}

#[test]
fn manual_check_bypasses_interval_but_never_duplicates_a_running_check() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let mut service = UpdateService::default();
    assert_eq!(service.check_status(), UpdateCheckStatus::NotChecked);
    assert!(!service.can_check());
    service.start_at(&ctx, now, completed_job);
    assert_eq!(service.check_status(), UpdateCheckStatus::Checking);
    finish(&mut service, Ok(None));
    assert_eq!(service.check_status(), UpdateCheckStatus::UpToDate);
    assert!(service.can_check());
    assert!(!service.check_due_at(&ctx, now, |_, _| panic!("not due")));
    assert!(service.begin_check_at(&ctx, now, completed_job));
    assert!(!service.begin_check_at(&ctx, now, |_, _| panic!("duplicate")));
    finish(&mut service, Ok(Some(available())));
    assert_eq!(service.check_status(), UpdateCheckStatus::Available);
    assert!(service.download.is_none());
    assert!(!service.handoff_active());
}

#[test]
fn failed_manual_check_is_distinct_from_current_and_can_retry_immediately() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    let mut service = UpdateService::default();
    service.start_at(&ctx, now, completed_job);
    finish(&mut service, Ok(None));
    assert!(
        service.begin_check_at(&ctx, now, |_, _| Err(std::io::Error::other(
            "spawn refused"
        )))
    );
    assert_eq!(service.check_status(), UpdateCheckStatus::Failed);
    assert_eq!(service.state(), &UpdateState::Idle);
    assert!(!service.check_due_at(&ctx, now, |_, _| panic!("retry storm")));
    assert!(service.begin_check_at(&ctx, now, completed_job));
    finish(&mut service, Ok(None));
    assert_eq!(service.check_status(), UpdateCheckStatus::UpToDate);
}

#[test]
fn manual_check_cannot_replace_active_download_verification_or_installation() {
    let ctx = egui::Context::default();
    let now = Instant::now();
    for state in [
        UpdateState::Downloading {
            downloaded: 1,
            total: 100,
        },
        UpdateState::Verifying,
        UpdateState::Ready,
        UpdateState::Installing,
    ] {
        let (mut service, _) = test_driver(state.clone());
        assert!(!service.can_check());
        assert!(!service.begin_check_at(&ctx, now, |_, _| panic!("active task")));
        assert_eq!(service.state(), &state);
    }
}

#[test]
fn unsupported_directory_does_not_start_another_download_on_retry() {
    let (mut service, _) = test_driver(UpdateState::Failed {
        error: UpdateError::Unsupported,
    });
    service.candidate = Some(available());
    assert!(service.begin_download().is_none());
    assert!(service.download.is_none());
    assert_eq!(
        service.state(),
        &UpdateState::Failed {
            error: UpdateError::Unsupported
        }
    );
}

#[test]
fn checking_again_cannot_clear_an_unsupported_installation_directory() {
    let ctx = egui::Context::default();
    for result in [Ok(Some(available())), Ok(None), Err(UpdateError::Network)] {
        let unsupported = UpdateState::Failed {
            error: UpdateError::Unsupported,
        };
        let (mut service, _) = test_driver(unsupported.clone());
        service.candidate = Some(available());
        for _ in 0..2 {
            assert!(service.begin_check_at(&ctx, Instant::now(), completed_job));
            finish(&mut service, result.clone());
            assert_eq!(service.state(), &unsupported);
            assert!(service.begin_download().is_none());
        }
    }
}
