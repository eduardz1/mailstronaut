//! Real native lifecycle feasibility, not a background-mail implementation.

use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use gpui::{
    AppContext as _, AsyncApp, GraphicalEnvironment, QuitMode, WindowingModes, WindowingRequest,
};

fn root() -> PathBuf {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_LIFECYCLE_ROOT").expect("private root"));
    assert!(root.is_absolute() && root.join("synthetic-lifecycle").is_file());
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
    root
}

fn renderer_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .expect("private process task list")
        .filter_map(Result::ok)
        .filter(|task| {
            std::fs::read_to_string(task.path().join("comm"))
                .is_ok_and(|name| name.starts_with("mailstronaut-ht"))
        })
        .count()
}

fn tick(cx: &AsyncApp) -> gpui::Task<()> {
    cx.background_executor().timer(Duration::from_millis(50))
}

#[test]
#[ignore = "private background reminder fixture only"]
fn seed_background_reminder() {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_BACKGROUND_SEED_ROOT").expect("fixture root"));
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    let root = root.canonicalize().expect("fixture root");
    assert!(root.starts_with(target));
    assert_eq!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
        format!("unix:path={}/runtime/bus", root.display())
    );
    let message = std::env::var("MAILSTRONAUT_BACKGROUND_SEED_MESSAGE")
        .expect("synthetic message id")
        .parse()
        .expect("integer message id");
    let mut store =
        mailstronaut_core::store::Store::open(&root.join("background-data/mailstronaut"))
            .expect("synthetic store");
    let now = chrono::Utc::now().timestamp();
    store
        .schedule_reminder(&mailstronaut_core::store::reminders::Schedule {
            account: "background-synthetic".into(),
            message,
            due_at: now + 90,
            time_zone: "UTC".into(),
            now,
            expected: None,
        })
        .expect("schedule synthetic reminder");
}

fn detached(cx: &AsyncApp) {
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 0);
        assert!(cx.graphical_environment().is_none());
        assert_eq!(cx.compositor_name(), "headless");
    });
}

fn open_workspace(
    cx: &AsyncApp,
    runtime: Arc<tokio::runtime::Runtime>,
    services: crate::Services,
) -> gpui::WeakEntity<crate::workspace::Workspace> {
    cx.update(|cx| {
        assert!(cx.graphical_environment().is_some());
        let (_, view) = gpui::open_window(
            gpui::WindowOptions {
                app_id: Some(include_str!("../../../assets/app-id").into()),
                ..gpui::component::TitleBar::window_options()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    crate::workspace::Workspace::new(None, true, runtime, services, window, cx)
                })
            },
        )
        .expect("actual synthetic workspace");
        view.downgrade()
    })
}

#[expect(
    clippy::future_not_send,
    reason = "Native display transitions run on GPUI's UI executor with its non-Send context."
)]
#[expect(
    clippy::too_many_lines,
    reason = "One native session verifies service retention across both window lifetimes."
)]
async fn scenario(cx: &mut AsyncApp, root: PathBuf, environment: GraphicalEnvironment) {
    detached(cx);
    assert_eq!(renderer_threads(), 0);
    let retained = cx.new(|_| 0usize);
    // A failed native attachment must leave the headless loop usable for retry.
    let mut missing = environment.clone();
    missing.wayland_display = Some(root.join("runtime/missing-wayland").into_os_string());
    let attempt = cx.update(|cx| cx.request_windowing(WindowingRequest::Windowed(missing)));
    assert!(attempt.await.is_err());
    detached(cx);
    tick(cx).await;
    retained.update(cx, |count, _| *count += 1);

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("mail runtime"),
    );
    let profile = root.join("reminder-profile");
    let mut store = mailstronaut_core::store::Store::open(&profile).expect("synthetic store");
    store.ensure_account("a").expect("account");
    let message = store
        .ingest_message("a", b"Subject: Synthetic\r\n\r\nFixture.")
        .expect("message");
    drop(store);
    let owner = Arc::new(
        runtime
            .spawn(mailstronaut_core::worker::Worker::open(profile))
            .await
            .expect("worker task")
            .expect("worker"),
    );
    let handle = owner.handle();
    let config: mailstronaut_config::Config = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1, "notifications": {"enabled": false},
        "accounts": [{"id":"a","provider":"icloud","address":"a@example.test","displayName":"A","userName":"a","auth":{"kind":"password","secretRef":"a"}}]
    })).expect("synthetic configuration");
    let config = Arc::new(config);
    let reminders = cx.new(|_| crate::reminders::Session::new(runtime.clone()));
    reminders.update(cx, |session, cx| {
        session.configure(&config, Some(handle.clone()), cx);
    });
    let (closed, receiver) = async_channel::bounded(1);
    let shared_store = Arc::new(tokio::sync::OnceCell::new());
    assert!(shared_store.set(owner.clone()).is_ok());
    let sync = cx.new(|_| crate::mail_sync::Session::new(runtime.clone()));
    let services = crate::Services {
        reminders: reminders.clone(),
        sync,
        store: shared_store.clone(),
        previews: crate::attachment_portal::Previews::default(),
    };
    cx.update(|cx| {
        cx.on_window_closed(move |cx, _| {
            if cx.windows().is_empty() {
                _ = closed.try_send(());
            }
        })
        .detach();
    });
    for cycle in 1..=2 {
        let attach =
            cx.update(|cx| cx.request_windowing(WindowingRequest::Windowed(environment.clone())));
        attach.await.expect("native Wayland attachment");
        let workspace = open_workspace(cx, runtime.clone(), services.clone());
        assert!(
            workspace
                .upgrade()
                .expect("workspace")
                .read_with(cx, |workspace, _| workspace.uses_owner(&owner))
        );
        for _ in 0..60 {
            if renderer_threads() == 1 {
                break;
            }
            tick(cx).await;
        }
        assert_eq!(
            renderer_threads(),
            1,
            "visible HTML worker positive control"
        );
        std::fs::write(root.join(format!("attached-{cycle}")), b"native window\n")
            .expect("attachment receipt");
        receiver.recv().await.expect("native close event");
        tick(cx).await;
        assert!(
            workspace.upgrade().is_none(),
            "the old workspace must be released"
        );
        let detach = cx.update(|cx| cx.request_windowing(WindowingRequest::Headless));
        detach.await.expect("detach after last window closes");
        detached(cx);
        for _ in 0..60 {
            if renderer_threads() == 0 {
                break;
            }
            tick(cx).await;
        }
        assert_eq!(
            renderer_threads(),
            0,
            "the Blitz worker must stop without a hidden window"
        );
        retained.update(cx, |count, _| *count += 1);
        if cycle == 1 {
            services.sync.update(cx, |session, cx| {
                session.configure(&config, Some(handle.clone()), cx);
            });
            let mut sync_observed = false;
            for _ in 0..100 {
                if services
                    .sync
                    .read_with(cx, |session, _| session.observed_failure())
                {
                    sync_observed = true;
                    break;
                }
                tick(cx).await;
            }
            assert!(
                sync_observed,
                "headless sync must report its bounded credential failure"
            );
            let now = chrono::Utc::now().timestamp();
            handle
                .schedule_reminder(mailstronaut_core::store::reminders::Schedule {
                    account: "a".into(),
                    message,
                    due_at: now + 3600,
                    time_zone: "UTC".into(),
                    now,
                    expected: None,
                })
                .await
                .expect("schedule while window is closed");
            reminders.update(cx, |session, _| session.wake());
            let mut observed = false;
            for _ in 0..60 {
                if reminders.read_with(cx, |session, _| session.has_message(message)) {
                    observed = true;
                    break;
                }
                tick(cx).await;
            }
            assert!(
                observed,
                "application-owned scheduler updates without a window"
            );
        }
        std::fs::write(
            root.join(format!("headless-{cycle}")),
            b"no window or HTML worker\n",
        )
        .expect("headless receipt");
        if cycle == 1 {
            for heartbeat in 1..=200 {
                std::fs::write(root.join("heartbeat.tmp"), heartbeat.to_string())
                    .expect("headless progress receipt");
                std::fs::rename(root.join("heartbeat.tmp"), root.join("heartbeat"))
                    .expect("atomic heartbeat publication");
                if root.join("reopen").is_file() {
                    break;
                }
                tick(cx).await;
            }
            assert!(
                root.join("reopen").is_file(),
                "driver must request reopening"
            );
        }
    }
    assert_eq!(retained.read_with(cx, |count, _| *count), 3);
    drop(reminders);
    drop(services);
    drop(shared_store);
    let owner = Arc::try_unwrap(owner).unwrap_or_else(|_| panic!("store still owned by a window"));
    runtime
        .spawn(owner.shutdown())
        .await
        .expect("shutdown task")
        .expect("shutdown synthetic worker");
    std::fs::write(
        root.join("passed"),
        b"one app, failed attach, two native windows, two detachments\n",
    )
    .expect("final receipt");
}

#[test]
#[ignore = "requires the native private session from experiments/p08-reminders/lifecycle.sh"]
fn native_headless_attach_close_detach_reopen() {
    let root = root();
    let mut environment = GraphicalEnvironment::detect();
    environment.x11_display = None;
    assert!(environment.wayland_display.is_some());
    let completed = Rc::new(Cell::new(false));
    let result = completed.clone();
    gpui::platform::linux(WindowingModes::WAYLAND | WindowingModes::HEADLESS)
        .with_windowing(WindowingRequest::Headless)
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui::init(cx);
            cx.spawn(async move |cx| {
                scenario(cx, root, environment).await;
                result.set(true);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(completed.get());
    println!(
        "Native headless lifecycle: failed attach/retry, two real windows, independent state, live headless loop and released workspaces/HTML workers passed."
    );
}

#[test]
#[ignore = "requires the private Dovecot and Secret Service harness"]
fn headless_sync_with_private_dovecot() {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_HEADLESS_SYNC_ROOT").expect("fixture root"));
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace")
        .join("target")
        .canonicalize()
        .expect("target");
    assert!(
        root.canonicalize()
            .expect("fixture root")
            .starts_with(target)
    );
    let config: mailstronaut_config::Config =
        serde_json::from_slice(&std::fs::read(root.join("config.json")).expect("fixture config"))
            .expect("valid config");
    let config = Arc::new(config);
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime"),
    );
    let owner = runtime
        .block_on(mailstronaut_core::worker::Worker::open(
            root.join("profile"),
        ))
        .expect("worker");
    let handle = owner.handle();
    let passed = Rc::new(Cell::new(false));
    let completed = passed.clone();
    gpui::platform::linux(WindowingModes::HEADLESS)
        .with_windowing(WindowingRequest::Headless)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui::init(cx);
            let session = cx.new(|_| crate::mail_sync::Session::new(runtime.clone()));
            session.update(cx, |session, cx| {
                session.configure(&config, Some(handle), cx);
            });
            cx.spawn(async move |cx| {
                std::fs::write(root.join("ready"), b"headless session\n").expect("ready receipt");
                for _ in 0..600 {
                    if root.join("stop").is_file() {
                        break;
                    }
                    tick(cx).await;
                }
                assert!(root.join("stop").is_file(), "driver must stop the session");
                drop(session);
                runtime
                    .spawn(owner.shutdown())
                    .await
                    .expect("shutdown task")
                    .expect("shutdown");
                completed.set(true);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.get());
}
