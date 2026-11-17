use super::*;

#[test]
fn coordinator_idle_defer_release_and_shutdown_use_existing_admission() {
    use crate::{
        file_workspace::WorkClass,
        scheduler::{
            PermissiveResourcePolicy, ResourceCapacities, ResourceHints, SchedulerConfig,
            WorkRequest, WorkScheduler,
        },
    };
    use std::{
        sync::{mpsc, Arc},
        time::Duration,
    };
    let f = Fixture::new();
    let scheduler = Arc::new(WorkScheduler::new(
        SchedulerConfig::default()
            .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
            .with_policy(Arc::new(PermissiveResourcePolicy)),
    ));
    scheduler.set_native_policy_notifications_available(true);
    let (tx, rx) = mpsc::channel();
    let coordinator = AutomationTriggerCoordinator::start_with_scheduler(
        f.db().clone(),
        scheduler.clone(),
        Arc::new(move || {
            let _ = tx.send(());
        }),
    )
    .unwrap();
    // No enabled automatic Intent: no scheduler request, receipt or mutation.
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(scheduler.snapshot().total_grants, 0);
    assert_eq!(scheduler.snapshot().queued, 0);
    assert!(f.db().list_automation_runs(None).unwrap().is_empty());
    let holder = scheduler
        .try_acquire(WorkRequest::new(
            "foreground-holder",
            WorkClass::Foreground,
            ResourceHints {
                cpu: 1,
                ..ResourceHints::empty()
            },
        ))
        .unwrap();
    let i = intent(&f, schedule("UTC", "09:00", &[1, 2, 3, 4, 5, 6, 7]));
    force_due(
        f.db(),
        &i.id,
        crate::db::queries::current_unix_seconds() - 86400,
    );
    f.db().wake_automation_triggers();
    rx.recv_timeout(Duration::from_secs(3))
        .expect("deferred projection event");
    assert!(f.db().list_automation_runs(None).unwrap().is_empty());
    assert_eq!(
        f.db()
            .get_automation_intent(&i.id)
            .unwrap()
            .trigger_state
            .unwrap()
            .last_error_code
            .as_deref(),
        Some("automation_resource_deferred")
    );
    assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
    assert_eq!(scheduler.snapshot().total_timed_wait_wakeups, 0);
    assert_eq!(scheduler.snapshot().total_grants, 1);
    let (foreground_tx, foreground_rx) = mpsc::channel();
    let foreground_scheduler = scheduler.clone();
    let foreground = std::thread::spawn(move || {
        let lease = foreground_scheduler
            .acquire(WorkRequest::new(
                "priority-foreground",
                WorkClass::Foreground,
                ResourceHints {
                    cpu: 1,
                    ..ResourceHints::empty()
                },
            ))
            .unwrap();
        foreground_tx.send(lease).unwrap();
    });
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while scheduler.snapshot().queued < 2 {
        assert!(std::time::Instant::now() < until, "both admissions queue");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(holder);
    let foreground_lease = foreground_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(f.db().list_automation_runs(None).unwrap().is_empty());
    drop(foreground_lease);
    foreground.join().unwrap();
    rx.recv_timeout(Duration::from_secs(3))
        .expect("receipt after foreground release");
    coordinator.shutdown();
    assert_eq!(f.db().list_automation_runs(None).unwrap().len(), 1);
    assert_eq!(scheduler.snapshot().running, 0);
    assert_eq!(scheduler.snapshot().queued, 0);
    assert_eq!(scheduler.snapshot().granted, ResourceHints::empty());
}

#[test]
fn coordinator_shutdown_cancels_deferred_cause_without_consuming_it() {
    use crate::{
        file_workspace::WorkClass,
        scheduler::{
            PermissiveResourcePolicy, ResourceCapacities, ResourceHints, SchedulerConfig,
            WorkRequest, WorkScheduler,
        },
    };
    use std::{
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };
    // Exercise the shutdown/waiter hand-off repeatedly. Keep shutdown off the
    // test thread so a lost condition-variable wake fails with diagnostics
    // instead of pinning the Windows CI worker until its job timeout.
    for attempt in 0..16 {
        let f = Fixture::new();
        let scheduler = Arc::new(WorkScheduler::new(
            SchedulerConfig::default()
                .with_capacities(ResourceCapacities::new(1, 1, 8, 1, 1, 1))
                .with_policy(Arc::new(PermissiveResourcePolicy)),
        ));
        scheduler.set_native_policy_notifications_available(true);
        let holder = scheduler
            .try_acquire(WorkRequest::new(
                "holder",
                WorkClass::Foreground,
                ResourceHints {
                    cpu: 1,
                    ..ResourceHints::empty()
                },
            ))
            .unwrap();
        let i = intent(&f, schedule("UTC", "09:00", &[1, 2, 3, 4, 5, 6, 7]));
        force_due(
            f.db(),
            &i.id,
            crate::db::queries::current_unix_seconds() - 86400,
        );
        let (tx, rx) = mpsc::channel();
        let coordinator = AutomationTriggerCoordinator::start_with_scheduler(
            f.db().clone(),
            scheduler.clone(),
            Arc::new(move || {
                let _ = tx.send(());
            }),
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|error| {
                panic!("attempt {attempt}: no admission notification: {error}")
            });

        let queued_deadline = Instant::now() + Duration::from_secs(5);
        while scheduler.snapshot().queued == 0 {
            assert!(
                Instant::now() < queued_deadline,
                "attempt {attempt}: coordinator did not queue behind the held lease"
            );
            std::thread::yield_now();
        }

        let cause = f
            .db()
            .select_automation_trigger(crate::db::queries::current_unix_seconds())
            .unwrap()
            .0
            .unwrap();
        let waiting_state = scheduler.snapshot();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let shutdown_thread = std::thread::spawn(move || {
            coordinator.shutdown();
            let _ = shutdown_tx.send(());
        });
        if let Err(error) = shutdown_rx.recv_timeout(Duration::from_secs(5)) {
            panic!(
                "attempt {attempt}: coordinator shutdown did not cancel its scheduler wait within 5s: {error}; before shutdown queued={} running={} granted={:?}",
                waiting_state.queued,
                waiting_state.running,
                waiting_state.granted
            );
        }
        shutdown_thread
            .join()
            .unwrap_or_else(|_| panic!("attempt {attempt}: coordinator shutdown thread panicked"));

        assert_eq!(scheduler.snapshot().queued, 0, "attempt {attempt}");
        assert!(
            f.db().list_automation_runs(None).unwrap().is_empty(),
            "attempt {attempt}: shutdown consumed the deferred cause"
        );
        assert_eq!(
            f.db()
                .select_automation_trigger(crate::db::queries::current_unix_seconds())
                .unwrap()
                .0
                .unwrap()
                .request_key,
            cause.request_key,
            "attempt {attempt}: deferred cause changed"
        );
        drop(holder);
        assert_eq!(scheduler.snapshot().running, 0, "attempt {attempt}");
        assert_eq!(
            scheduler.snapshot().granted,
            ResourceHints::empty(),
            "attempt {attempt}"
        );
    }
}
