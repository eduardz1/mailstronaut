//! Native Wayland application and display-independent command entry points.

extern crate gpui_kit as gpui;

mod account_actions;
mod accounts;
mod assets;
mod attachment_portal;
mod composer;
mod config_cli;
mod config_export;
mod html_reader;
mod html_worker;
mod instance;
#[cfg(test)]
mod lifecycle_tests;
mod link_portal;
mod mail_notify;
mod mail_priority;
mod mail_search;
mod mail_semantic;
mod mail_sync;
mod mailbox;
mod onboarding;
#[cfg(test)]
mod portal_fixture;
mod preferences;
mod read_gate;
mod reminders;
mod search_filters;
mod thread_reader;
mod workspace;

use std::{cell::RefCell, env, path::PathBuf, process::ExitCode, rc::Rc, sync::Arc};

use gpui::{
    AppContext as _, Bounds, GraphicalEnvironment, KeyBinding, QuitMode, WindowBounds,
    WindowOptions, WindowingModes, WindowingRequest, px, size,
};

type SharedStore = Arc<tokio::sync::OnceCell<Arc<mailstronaut_core::worker::Worker>>>;

#[derive(Clone)]
struct Services {
    reminders: gpui::Entity<reminders::Session>,
    sync: gpui::Entity<mail_sync::Session>,
    store: SharedStore,
    previews: attachment_portal::Previews,
}

gpui::actions!(
    mailstronaut,
    [
        #[derive(Eq)]
        /// Stop the application, including any background work.
        QuitApplication
    ]
);

#[derive(Default)]
struct LifecycleRequests {
    closed: bool,
    activate: bool,
    target: Option<workspace::PendingOpen>,
}

type Requests = Rc<RefCell<LifecycleRequests>>;

fn wake_lifecycle(
    requests: &Requests,
    wake: &async_channel::Sender<()>,
    change: impl FnOnce(&mut LifecycleRequests),
) {
    change(&mut requests.borrow_mut());
    _ = wake.try_send(());
}

fn open_main_window(
    config: Option<PathBuf>,
    demo: bool,
    runtime: Arc<tokio::runtime::Runtime>,
    services: Services,
    cx: &mut gpui::App,
) -> Result<
    (
        gpui::AnyWindowHandle,
        gpui::WeakEntity<workspace::Workspace>,
    ),
    &'static str,
> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(840.)),
            cx,
        ))),
        app_id: Some(include_str!("../../../assets/app-id").into()),
        ..gpui::component::TitleBar::window_options()
    };
    let (window, view) = gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| workspace::Workspace::new(config, demo, runtime, services, window, cx))
    })
    .map_err(|_| "Unable to open a Wayland window.")?;
    Ok((window, view.downgrade()))
}

#[expect(
    clippy::too_many_lines,
    reason = "Application initialization wires its retained services and lifecycle callbacks together."
)]
fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.as_slice() == ["--internal-project-threads"] {
        return mailstronaut_core::threads::helper::run();
    }
    let demo = arguments.as_slice() == ["--demo"];
    let background = arguments.first().is_some_and(|arg| arg == "--background");
    if !demo && let Some(status) = config_cli::dispatch(&arguments) {
        return status;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("Unable to start mail runtime.");
        return ExitCode::FAILURE;
    };
    let runtime = Arc::new(runtime);
    let config = config_cli::selected_path(&arguments);
    // Claim the session name before creating a toolkit, window or store owner.
    // A second launch only asks the existing process to activate its window.
    let instance = match runtime.block_on(instance::launch(config.as_deref(), demo, background)) {
        Ok(Some(instance)) => instance,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let mut environment = GraphicalEnvironment::detect();
    if environment
        .wayland_display
        .as_ref()
        .is_none_or(|value| value.is_empty())
    {
        eprintln!("Mailstronaut requires a native Wayland session (WAYLAND_DISPLAY).");
        _ = runtime.block_on(instance.close());
        return ExitCode::FAILURE;
    }
    // Never fall back to X11, even when a session exports DISPLAY as well.
    environment.x11_display = None;
    let startup = if background {
        let result = runtime.block_on(async {
            if ashpd::is_sandboxed() {
                return Err("Background startup is unavailable in the sandbox.");
            }
            let path = config
                .clone()
                .ok_or("Unable to locate the native configuration.")?;
            let executable =
                env::current_exe().map_err(|_| "Unable to locate configuration helper.")?;
            let declaration = mailstronaut_config::source::load(
                path,
                &executable,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(|_| "Unable to load native configuration.")?
            .ok_or("Native configuration is missing.")?;
            if !declaration.background.enabled {
                return Err("Background operation is disabled in the native configuration.");
            }
            let root = directories::BaseDirs::new()
                .ok_or("Unable to locate local mail storage.")?
                .data_local_dir()
                .join("mailstronaut");
            let owner = mailstronaut_core::worker::Worker::open(root)
                .await
                .map_err(|_| "Unable to open local mail storage.")?;
            Ok((Arc::new(declaration), Arc::new(owner)))
        });
        match result {
            Ok(startup) => Some(startup),
            Err(error) => {
                eprintln!("{error}");
                _ = runtime.block_on(instance.close());
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let ui_runtime = runtime.clone();
    let activations = instance.activations();
    let disconnected = instance.disconnected();
    gpui::platform::linux(WindowingModes::WAYLAND | WindowingModes::HEADLESS)
        .with_windowing(if background {
            WindowingRequest::Headless
        } else {
            WindowingRequest::Windowed(environment.clone())
        })
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", QuitApplication, None)]);
            cx.on_action(|_: &QuitApplication, cx| cx.quit());
            let services = Services {
                reminders: cx.new(|_| reminders::Session::new(ui_runtime.clone())),
                sync: cx.new(|_| mail_sync::Session::new(ui_runtime.clone())),
                store: Arc::new(tokio::sync::OnceCell::new()),
                previews: crate::attachment_portal::Previews::default(),
            };
            if let Some((declaration, owner)) = &startup {
                assert!(services.store.set(owner.clone()).is_ok(), "new store owner");
                let handle = Some(owner.handle());
                services.reminders.update(cx, |session, cx| {
                    session.configure(declaration, handle.clone(), cx);
                });
                services
                    .sync
                    .update(cx, |session, cx| session.configure(declaration, handle, cx));
            }
            let initial = if background {
                None
            } else {
                open_main_window(
                    config.clone(),
                    demo,
                    ui_runtime.clone(),
                    services.clone(),
                    cx,
                )
                .ok()
            };
            if !background && initial.is_none() {
                eprintln!("Unable to open the native Wayland window.");
                cx.quit();
            }
            let requests = Rc::new(RefCell::new(LifecycleRequests::default()));
            let (wake, receiver) = async_channel::bounded(1);
            let close_requests = requests.clone();
            let close_wake = wake.clone();
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() {
                    wake_lifecycle(&close_requests, &close_wake, |requests| {
                        requests.closed = true;
                    });
                }
            })
            .detach();
            let sync_requests = requests.clone();
            let sync_wake = wake.clone();
            cx.subscribe(&services.sync, move |_, event: &mail_sync::Event, cx| {
                if cx.windows().is_empty()
                    && let mail_sync::Event::Open(click) = event
                {
                    wake_lifecycle(&sync_requests, &sync_wake, |requests| {
                        requests.target = Some(workspace::PendingOpen::Mail(click.clone()));
                    });
                }
            })
            .detach();
            let reminder_requests = requests.clone();
            let reminder_wake = wake.clone();
            cx.subscribe(
                &services.reminders,
                move |_, event: &reminders::SessionEvent, cx| {
                    if cx.windows().is_empty()
                        && let reminders::SessionEvent::Open(record) = event
                    {
                        wake_lifecycle(&reminder_requests, &reminder_wake, |requests| {
                            requests.target =
                                Some(workspace::PendingOpen::Reminder(record.clone()));
                        });
                    }
                },
            )
            .detach();
            listen_for_activation(activations, disconnected, requests.clone(), wake, cx);
            run_lifecycle(
                receiver,
                requests,
                initial,
                services,
                config,
                demo,
                ui_runtime,
                environment,
                background,
                cx,
            );
            if !background {
                cx.activate(true);
            }
        });
    // Keep the session name until the platform and its window-owned work stop.
    _ = runtime.block_on(instance.close());
    ExitCode::SUCCESS
}

/// Forward passive activation on the UI thread and stop if session ownership ends.
fn listen_for_activation(
    activations: async_channel::Receiver<()>,
    disconnected: impl std::future::Future<Output = ()> + 'static,
    requests: Requests,
    wake: async_channel::Sender<()>,
    cx: &gpui::App,
) {
    let shutdown = activations.clone();
    cx.on_app_quit(move |_| {
        shutdown.close();
        std::future::ready(())
    })
    .detach();
    cx.spawn(async move |cx| {
        tokio::pin!(disconnected);
        loop {
            tokio::select! {
                () = &mut disconnected => {
                    eprintln!("The session bus closed. Mailstronaut is stopping.");
                    cx.update(|cx| cx.quit());
                    break;
                }
                request = activations.recv() => {
                    if request.is_err() { break; }
                    wake_lifecycle(&requests, &wake, |requests| requests.activate = true);
                }
            }
        }
    })
    .detach();
}

#[expect(
    clippy::too_many_arguments,
    reason = "The application lifecycle retains the existing session, display and activation inputs."
)]
fn run_lifecycle(
    receiver: async_channel::Receiver<()>,
    requests: Requests,
    initial: Option<(
        gpui::AnyWindowHandle,
        gpui::WeakEntity<workspace::Workspace>,
    )>,
    services: Services,
    config: Option<PathBuf>,
    demo: bool,
    runtime: Arc<tokio::runtime::Runtime>,
    environment: GraphicalEnvironment,
    initially_headless: bool,
    cx: &gpui::App,
) {
    cx.spawn(async move |cx| {
        let mut current = initial;
        let mut headless = initially_headless;
        while receiver.recv().await.is_ok() {
            let request = std::mem::take(&mut *requests.borrow_mut());
            if request.activate || request.target.is_some() {
                let empty = cx.update(|cx| cx.windows().is_empty());
                if empty {
                    if headless {
                        let attach = cx.update(|cx| {
                            cx.request_windowing(WindowingRequest::Windowed(environment.clone()))
                        });
                        if attach.await.is_err() {
                            eprintln!("Unable to reconnect to the native Wayland session.");
                            let mut pending = requests.borrow_mut();
                            if pending.target.is_none() {
                                pending.target = request.target;
                            }
                            continue;
                        }
                        headless = false;
                    }
                    let Ok(opened) = cx.update(|cx| {
                        open_main_window(
                            config.clone(),
                            demo,
                            runtime.clone(),
                            services.clone(),
                            cx,
                        )
                    }) else {
                        eprintln!("Unable to reopen the native Wayland window.");
                        {
                            let mut pending = requests.borrow_mut();
                            if pending.target.is_none() {
                                pending.target = request.target;
                            }
                        }
                        let detach =
                            cx.update(|cx| cx.request_windowing(WindowingRequest::Headless));
                        if detach.await.is_err() {
                            cx.update(|cx| cx.quit());
                            break;
                        }
                        headless = true;
                        continue;
                    };
                    current = Some(opened);
                }
                if let Some((handle, view)) = &current {
                    let target = request.target;
                    _ = handle.update(cx, |_, window, cx| {
                        window.activate_window();
                        if let (Some(target), Some(view)) = (target, view.upgrade()) {
                            view.update(cx, |view, cx| view.open_notification(target, window, cx));
                        }
                    });
                }
            } else if request.closed && cx.update(|cx| cx.windows().is_empty()) {
                current = None;
                let enabled = cx.update(|cx| services.sync.read(cx).background_enabled());
                if !enabled || demo || ashpd::is_sandboxed() {
                    cx.update(|cx| cx.quit());
                    break;
                }
                if !headless {
                    let detach = cx.update(|cx| cx.request_windowing(WindowingRequest::Headless));
                    if detach.await.is_err() {
                        eprintln!("Unable to continue without the native window.");
                        cx.update(|cx| cx.quit());
                        break;
                    }
                    headless = true;
                }
            }
        }
    })
    .detach();
}

mod cancellable_send {
    // Bounded service updates must not prevent shutdown when a consumer stalls.

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    pub async fn send<T>(sender: &mpsc::Sender<T>, value: T, cancel: &CancellationToken) -> bool {
        tokio::select! {
            biased;
            () = cancel.cancelled() => false,
            result = sender.send(value) => result.is_ok(),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use tokio::sync::mpsc;
        use tokio_util::sync::CancellationToken;

        #[tokio::test]
        async fn stalled_updates_do_not_prevent_shutdown() {
            let (sender, mut receiver) = mpsc::channel(1);
            let cancel = CancellationToken::new();
            assert!(super::send(&sender, 1, &cancel).await);
            let pending = tokio::spawn({
                let sender = sender.clone();
                let cancel = cancel.clone();
                async move { super::send(&sender, 2, &cancel).await }
            });
            tokio::task::yield_now().await;
            assert!(
                !pending.is_finished(),
                "the full channel applies backpressure"
            );
            cancel.cancel();
            assert!(
                !tokio::time::timeout(Duration::from_secs(1), pending)
                    .await
                    .expect("shutdown must not hang")
                    .expect("task")
            );
            assert_eq!(receiver.recv().await, Some(1));
            assert!(receiver.try_recv().is_err());
            assert!(!super::send(&sender, 3, &cancel).await);
        }

        #[tokio::test]
        async fn closed_consumer_stops_the_service() {
            let (sender, receiver) = mpsc::channel(1);
            drop(receiver);
            assert!(!super::send(&sender, 1, &CancellationToken::new()).await);
        }
    }
}
