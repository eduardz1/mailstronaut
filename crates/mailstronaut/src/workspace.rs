//! GPUI Kit mailbox workspace. Mail and storage remain toolkit-independent.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::assets::IconName as AssetIcon;
use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Selectable as _, Theme, ThemeMode,
    WindowExt as _,
    alert::Alert,
    attachment::{
        Attachment, AttachmentActions, AttachmentContent, AttachmentDescription, AttachmentGroup,
        AttachmentMedia, AttachmentTitle,
    },
    badge::Badge,
    button::{Button, ButtonVariants as _},
    description_list::DescriptionList,
    empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    label::Label,
    list::{List, ListEvent, ListState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::{Notification, NotificationDelivery},
    popover::Popover,
    progress::Progress,
    resizable::{h_resizable, resizable_panel},
    setting::{SettingGroup, SettingItem, SettingPage, Settings},
    sidebar::{Sidebar, SidebarGroup, SidebarItem as _, SidebarMenu, SidebarMenuItem},
    spinner::Spinner,
    status_bar::StatusBar,
    tab::{Tab, TabBar},
    toolbar::{Toolbar, ToolbarGroup},
};
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_config::{Account, Appearance, Config, source};
use mailstronaut_core::{
    reminders::When,
    reply::Kind,
    store::backfill::FolderCoverage,
    store::browse::{FilteredPage, Scope, SearchFilter},
    store::mailbox::{Cursor, Folder, FolderRole},
    worker::{Handle, Worker},
};
use mailstronaut_mail::imap::FlagChange;
use mailstronaut_search::excerpt::BodyExcerpt;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::mailbox::{Priority, Row, Rows};

mod attachment_open;
mod deadlines;
mod history;
mod move_picker;

/// How long a queued move can be undone before the server is told about it.
const UNDO_GRACE: Duration = Duration::from_secs(8);
/// Returning to the window checks for mail, at most this often.
const ACTIVATION_WAKE_INTERVAL: Duration = Duration::from_secs(60);

gpui::actions!(
    mailstronaut,
    [
        #[derive(Eq)]
        /// Focus the native mailbox search field.
        FocusSearch,
        #[derive(Eq)]
        /// Refresh the local mailbox cache without changing Seen state.
        RefreshCache,
        #[derive(Eq)]
        /// Toggle the standard GPUI Kit light/dark appearance.
        ToggleAppearance,
        #[derive(Eq)]
        /// Focus the native read-only message text without opening a message.
        FocusReader,
        #[derive(Eq)]
        /// Focus the current message's native web-link picker.
        FocusLinks,
        #[derive(Eq)]
        /// Focus the message list without opening its selection.
        FocusMessages,
        #[derive(Eq)]
        /// Display configured accounts without starting network work.
        ShowAccounts,
        #[derive(Eq)]
        /// Return to the inbox without opening any message.
        ShowInbox,
        #[derive(Eq)]
        /// Explicitly reload the canonical native Nix source.
        ReloadConfiguration,
        #[derive(Eq)]
        /// Export a validated native snapshot from Settings.
        ExportConfiguration,
        #[derive(Eq)]
        /// Start a new local draft.
        ComposeMessage,
        #[derive(Eq)]
        /// Show saved drafts and the outbox.
        ShowDrafts,
        #[derive(Eq)]
        /// Prepare a reply to the open original.
        Reply,
        #[derive(Eq)]
        /// Prepare a reply to every visible recipient of the open original.
        ReplyAll,
        #[derive(Eq)]
        /// Prepare a plain-text forward of the open original.
        Forward,
        #[derive(Eq)]
        /// Toggle the open original between read and unread.
        ToggleRead,
        #[derive(Eq)]
        /// Toggle the star on the open original.
        ToggleStar,
        #[derive(Eq)]
        /// Move the open conversation to the account's archive.
        Archive,
        #[derive(Eq)]
        /// Move the open conversation to the account's trash.
        Trash,
        #[derive(Eq)]
        /// Show scheduled and overdue reminders.
        ShowReminders
    ]
);

#[derive(Eq, PartialEq)]
enum View {
    Inbox,
    Accounts,
    Compose,
    Reminders,
}

/// A notification action explicitly chosen while the application had no window.
#[derive(Clone)]
pub enum PendingOpen {
    /// A new-mail notification was activated.
    Mail(crate::mail_notify::Opened),
    /// A reminder notification was activated.
    Reminder(mailstronaut_core::store::reminders::Reminder),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReaderMode {
    Text,
    Html,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SearchMode {
    Words,
    Related,
}

pub struct Workspace {
    focus: FocusHandle,
    link_focus: FocusHandle,
    runtime: Arc<Runtime>,
    owner: Option<Arc<Worker>>,
    shared_store: crate::SharedStore,
    accounts: Vec<String>,
    folders: Vec<Folder>,
    /// Unread messages by folder ID, from the same read as `folders`.
    unread: HashMap<i64, usize>,
    scope: Scope,
    browse: crate::mailbox::Browse,
    declaration: source::Runtime,
    config_path: Option<PathBuf>,
    preferences: Entity<crate::preferences::Preferences>,
    onboarding: Entity<crate::onboarding::Onboarding>,
    _preference_events: Subscription,
    loading_config: bool,
    startup_error: Option<String>,
    pending_open: Option<PendingOpen>,
    view: View,
    account_access: crate::account_actions::Access,
    composer: Entity<crate::composer::Composer>,
    _composer_events: Subscription,
    reminders: Entity<crate::reminders::Reminders>,
    _reminder_events: Subscription,
    account_views: Vec<Entity<crate::accounts::AccountPane>>,
    account_events: Vec<Subscription>,
    account_lifetime: CancellationToken,
    lifetime: CancellationToken,
    query_cancel: CancellationToken,
    read_gate: crate::read_gate::Gate,
    list: Entity<ListState<Rows>>,
    _list_events: Subscription,
    thread_reader: Entity<crate::thread_reader::ThreadReader>,
    _thread_events: Subscription,
    selected: Option<Row>,
    move_picker: Option<move_picker::Picker>,
    body: String,
    text_reader: Entity<TextareaState>,
    reader_mode: ReaderMode,
    html_reader: Entity<crate::html_reader::HtmlReader>,
    _html_events: Subscription,
    opened_message: Option<Arc<mailstronaut_core::mime::Projection>>,
    /// What the opened message left on the server, if it is too large to keep.
    opened_omitted: Option<mailstronaut_core::outline::Omitted>,
    /// Why the opened message was or was not flagged, if it has been analyzed.
    opened_analysis: Option<mailstronaut_priority::Analysis>,
    corrected_deadline: Option<mailstronaut_core::store::analysis::DeadlineCorrection>,
    preferred_reader: ReaderMode,
    reader_notice: String,
    headers_open: bool,
    link_cancel: Option<CancellationToken>,
    preview_cancel: Option<CancellationToken>,
    previews: crate::attachment_portal::Previews,
    search: Entity<InputState>,
    filters: Entity<crate::search_filters::Filters>,
    filter: SearchFilter,
    _filter_events: Subscription,
    status: String,
    query: String,
    query_generation: u64,
    reading_generation: u64,
    demo: bool,
    _search_events: Subscription,
    _activation: Subscription,
    _appearance: Subscription,
    _focus_lost: Subscription,
    theme_override: Option<ThemeMode>,
    sync_session: Entity<crate::mail_sync::Session>,
    _sync_events: Subscription,
    /// Fuzzy search once the store is open; literal search answers until then.
    fuzzy: Option<crate::mail_search::MailSearch>,
    meaning: Option<crate::mail_semantic::Meaning>,
    search_mode: SearchMode,
    search_pause: Task<()>,
    /// Only while `priority.enabled` is set in the configuration.
    priority: Option<crate::mail_priority::PriorityEngine>,
    /// Refreshes the list when the first thing in the priority group expires.
    priority_expiry: Task<()>,
    /// How far back older mail has been fetched, by account and folder wire name. Absent
    /// means not yet asked for, and `None` means the beginning of the folder was reached.
    older_reached: HashMap<(String, String), Option<mailstronaut_core::sync::Position>>,
    /// The accepted configuration revision that owns the current history request.
    older_fetch: Option<u64>,
    announcer: Option<Arc<crate::mail_notify::Announcer>>,
    syncing: BTreeSet<String>,
    sync_progress: BTreeMap<String, crate::mail_sync::Fetch>,
    cancelled_fetches: BTreeSet<String>,
    sync_errors: BTreeMap<String, crate::mail_sync::Failure>,
    move_warnings: BTreeMap<String, String>,
    history: Option<Vec<FolderCoverage>>,
    history_task: Task<()>,
    last_sync: String,
    last_wake: Instant,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(announcer) = &self.announcer {
            announcer.set_window_active(false);
        }
        self.lifetime.cancel();
        self.account_lifetime.cancel();
        self.query_cancel.cancel();
    }
}

const SEARCH_PAUSE: Duration = Duration::from_millis(200);
fn related_intent(query: &str) -> bool {
    !query.trim().is_empty()
        && !query.contains('"')
        && !query.split_whitespace().any(|word| {
            let word = word.to_ascii_lowercase();
            word.starts_with("from:") || word.starts_with("subject:")
        })
}
/// Identifies the progress notification, so that the result replaces it.
struct AttachmentDownload;
/// Folders, unread counts, a new page, priority rows and aliases for retired row identities.
type ListPage = (
    Vec<Folder>,
    Vec<(i64, u32)>,
    Vec<(String, String)>,
    Vec<mailstronaut_core::store::conversations::Summary>,
    Priority,
    HashMap<i64, i64>,
);
/// More than this much needing attention at once is not a useful group until Show all.
const MAX_PRIORITY_ROWS: usize = 20;
/// The most conversations the store reads in one page.
const STORE_PAGE: u32 = 100;

/// Concurrent window initialization shares one profile owner, including an
/// initialization that outlives the window which requested it.
async fn open_shared_store(
    shared: &crate::SharedStore,
    root: PathBuf,
) -> Result<Arc<Worker>, String> {
    shared
        .get_or_try_init(|| async move {
            Worker::open(root)
                .await
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .await
        .cloned()
}

/// The priority group as list rows, one per conversation, soonest to expire first.
async fn priority_rows(
    store: &Handle,
    accounts: Vec<String>,
    show_all: bool,
) -> Result<Priority, mailstronaut_core::worker::Error> {
    let now = chrono::Utc::now().timestamp();
    // One row past the limit is asked for, which is how a full group shows it has more.
    let limit = if show_all {
        u32::MAX
    } else {
        u32::try_from(MAX_PRIORITY_ROWS + 1).unwrap_or(u32::MAX)
    };
    let mut items = store.priority_items(accounts.clone(), now, limit).await?;
    let more = crate::mailbox::cut_group(&mut items, MAX_PRIORITY_ROWS, show_all);
    // The store reads conversations at most one page at a time, so Show all reads in pages.
    let page_rows = usize::try_from(STORE_PAGE).unwrap_or(usize::MAX);
    let mut rows = Vec::with_capacity(items.len());
    for page in items.chunks(page_rows) {
        let ids = page.iter().map(|item| item.id).collect();
        let summaries = store
            .ranked_conversation_page(accounts.clone(), Scope::Inbox, ids, None, STORE_PAGE)
            .await?;
        rows.extend(summaries.into_iter().map(|row| {
            let mut row = Row::from(row);
            if let Some(item) = items.iter().find(|item| item.id == row.id) {
                row.due.clone_from(&item.due);
                row.expires_at = Some(item.expires_at);
            }
            row
        }));
    }
    Ok(Priority { rows, more })
}

/// A page of conversations: through the search index when there is a query and it can
/// answer, and otherwise through the store's own literal search.
#[expect(
    clippy::too_many_arguments,
    reason = "One scoped page shares optional lexical/semantic readers, filters and its cursor."
)]
async fn conversation_rows(
    store: &Handle,
    fuzzy: Option<&crate::mail_search::MailSearch>,
    meaning: Option<&crate::mail_semantic::Meaning>,
    accounts: Vec<String>,
    scope: Scope,
    query: String,
    after: Option<Cursor>,
    filter: SearchFilter,
) -> Result<(Vec<mailstronaut_core::store::conversations::Summary>, bool), String> {
    // Explicit related-mail mode. Quotes and field constraints retain word intent.
    if related_intent(&query)
        && let Some(meaning) = meaning
    {
        let found = meaning
            .hits(
                query.clone(),
                store.clone(),
                accounts.clone(),
                scope.clone(),
                filter.clone(),
            )
            .await
            .map_err(str::to_owned)?;
        return store
            .revisioned_conversation_page(
                FilteredPage {
                    accounts,
                    scope,
                    query,
                    hits: Some(found.ids),
                    filter,
                    after,
                    limit: 100,
                },
                found.sources,
            )
            .await
            .map(|rows| (rows, true))
            .map_err(|error| error.to_string());
    }
    let hits = match fuzzy {
        Some(search) if !query.trim().is_empty() => {
            // The number of hits is bounded, so other accounts must not use them up.
            let searched = match &scope {
                Scope::Inbox => accounts.clone(),
                Scope::Account(account) | Scope::Folder { account, .. } => vec![account.clone()],
            };
            search
                .filtered_hits(
                    query.clone(),
                    searched,
                    mailstronaut_search::Filter {
                        received_from: filter.received_from,
                        received_before: filter.received_before,
                        sender: filter.sender.clone(),
                        originals: Some(Arc::new(
                            store
                                .search_scope(accounts.clone(), scope.clone())
                                .await
                                .map_err(|error| error.to_string())?,
                        )),
                    },
                )
                .await
        }
        _ => None,
    };
    let ranked = hits.is_some();
    let excerpt = BodyExcerpt::for_words(&query);
    let mut rows = if !filter.is_empty() {
        store
            .filtered_conversation_page(FilteredPage {
                accounts,
                scope,
                query,
                hits,
                filter,
                after,
                limit: 100,
            })
            .await
    } else if let Some(hits) = hits {
        store
            .ranked_conversation_page(accounts, scope, hits, after, 100)
            .await
    } else {
        store
            .conversation_page(accounts, scope, query, after, 100)
            .await
    }
    .map_err(|error| error.to_string())?;
    if let Some(excerpt) = excerpt {
        crate::mail_search::attach_body_excerpts(store, &mut rows, excerpt.words()).await;
    }
    Ok((rows, ranked))
}

/// Resolve only groups that vanished from the loaded prefix. Any displayed row can
/// become the selection or scroll anchor while the background refresh is running.
async fn merged_row_aliases(
    store: &Handle,
    accounts: &[String],
    mut previous: HashSet<i64>,
    rows: &[mailstronaut_core::store::conversations::Summary],
    group: &Priority,
    cancel: &CancellationToken,
) -> Result<HashMap<i64, i64>, String> {
    for id in rows
        .iter()
        .filter_map(|row| row.conversation)
        .chain(group.rows.iter().filter_map(|row| row.conversation))
    {
        previous.remove(&id);
    }
    let mut aliases = HashMap::new();
    let missing: Vec<_> = previous.into_iter().collect();
    for batch in missing.chunks(100) {
        let merged = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("Local query cancelled.".into()),
            result = store.merged_conversations(accounts.to_vec(), batch.to_vec()) => result.map_err(|error| error.to_string())?,
        };
        aliases.extend(merged);
    }
    Ok(aliases)
}

/// Refresh the already-loaded metadata prefix in bounded store batches.
#[expect(
    clippy::too_many_arguments,
    reason = "Bounded prefix refresh shares scope, query, filters, and cancellation with the page reader."
)]
async fn conversation_prefix(
    handle: &Handle,
    fuzzy: Option<&crate::mail_search::MailSearch>,
    meaning: Option<&crate::mail_semantic::Meaning>,
    accounts: Vec<String>,
    scope: Scope,
    query: String,
    boundary: Option<Cursor>,
    cancel: &CancellationToken,
    filter: SearchFilter,
) -> Result<Vec<mailstronaut_core::store::conversations::Summary>, String> {
    let mut rows = Vec::new();
    let mut after = None;
    loop {
        if cancel.is_cancelled() {
            return Err("Local query cancelled.".into());
        }
        let (batch, ranked) = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("Local query cancelled.".into()),
            result = conversation_rows(handle, fuzzy, meaning, accounts.clone(), scope.clone(), query.clone(), after, filter.clone()) => result?,
        };
        if batch.iter().any(|row| {
            rows.iter().any(
                |loaded: &mailstronaut_core::store::conversations::Summary| {
                    loaded.latest.id == row.latest.id
                },
            )
        }) {
            return Err("Local search results changed. Refresh to retry.".into());
        }
        after = batch.last().map(|row| Cursor {
            received_at: row.latest.received_at,
            message_id: row.latest.id,
        });
        let more = batch.len() == 100;
        rows.extend(batch);
        let reached = boundary.is_none_or(|old| {
            if ranked {
                // Word-match tiers and related-mail scores are not chronological.
                // Seek the actual original; a vanished boundary is bounded by the
                // existing lexical cap (semantic suggestions have a lower cap).
                rows.iter().any(|row| row.latest.id == old.message_id)
                    || rows.len() >= crate::mail_search::MAX_HITS
            } else {
                after.is_some_and(|next| {
                    (next.received_at, next.message_id) <= (old.received_at, old.message_id)
                })
            }
        });
        if !more || reached {
            break;
        }
    }
    Ok(rows)
}

impl Workspace {
    #[expect(
        clippy::too_many_lines,
        reason = "Construct native entities and their subscriptions together before initializing the workspace that owns them."
    )]
    pub fn new(
        path: Option<PathBuf>,
        demo: bool,
        runtime: Arc<Runtime>,
        services: crate::Services,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Theme::sync_system_appearance(Some(window), cx);
        Self::bind_keys(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search cached mail"));
        let filters = cx.new(|cx| crate::search_filters::Filters::new(window, cx));
        let filter_events = cx.subscribe_in(&filters, window, |this, _, event, window, cx| {
            let crate::search_filters::Event::Applied(filter) = event;
            this.filter = filter.clone();
            this.browse = crate::mailbox::Browse::default();
            this.clear_page(window, cx);
            this.refresh(window, cx);
        });
        let text_reader = cx.new(|cx| TextareaState::new(window, cx));
        let html_reader =
            cx.new(|cx| crate::html_reader::HtmlReader::new(runtime.handle(), window, cx));
        let html_events = cx.subscribe_in(&html_reader, window, |this, _, event, window, cx| {
            this.html_event(event, window, cx);
        });
        let events = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            match event {
                InputEvent::PressEnter { .. } => {
                    this.search_pause = Task::ready(());
                    this.refresh(window, cx);
                }
                // A query is cheap, but several keystrokes are still one search.
                InputEvent::Change => {
                    this.search_pause = cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(SEARCH_PAUSE).await;
                        _ = this.update_in(cx, Self::refresh);
                    });
                }
                _ => {}
            }
        });
        let workspace = cx.entity().downgrade();
        let show_all_workspace = workspace.clone();
        let load_workspace = workspace.clone();
        let rows = Rows {
            on_dismiss: Some(Rc::new(move |message, window, cx| {
                _ = workspace.update(cx, |this, cx| this.dismiss_priority(message, window, cx));
            })),
            on_show_all: Some(Rc::new(move |window, cx| {
                _ = show_all_workspace.update(cx, |this, cx| this.show_all_priority(window, cx));
            })),
            on_load_more: Some(Rc::new(move |window, cx| {
                _ = load_workspace.update(cx, |this, cx| this.load_more_rows(window, cx));
            })),
            ..Rows::default()
        };
        let list = cx.new(|cx| ListState::new(rows, window, cx).searchable(false));
        let list_events = cx.subscribe_in(&list, window, |this, list, event, window, cx| {
            if let ListEvent::Confirm(index) = event
                && let Some(row) = list.read(cx).delegate().row(*index).cloned()
            {
                this.open(row, window, cx);
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            this.read_gate
                .visibility(window.is_window_active() && this.view == View::Inbox);
            // A message opened while the window was away, such as from a notification, is
            // marked read now that it is in front, if its body had already loaded.
            this.mark_loaded(window, cx);
            this.sync_session.update(cx, |session, _| {
                session.set_window_active(window.is_window_active());
            });
            if window.is_window_active() && this.last_wake.elapsed() >= ACTIVATION_WAKE_INTERVAL {
                this.last_wake = Instant::now();
                this.sync_session.read(cx).wake_all();
            }
        });
        let thread_reader =
            cx.new(|cx| crate::thread_reader::ThreadReader::new(runtime.clone(), window, cx));
        let thread_events = cx.subscribe_in(
            &thread_reader,
            window,
            |this, _, event: &crate::thread_reader::Open, window, cx| {
                if this.view == View::Inbox
                    && this
                        .selected
                        .as_ref()
                        .is_some_and(|row| row.same_group(&event.0))
                {
                    this.open(event.0.clone(), window, cx);
                }
            },
        );
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            if this.theme_override.is_none() {
                Theme::sync_system_appearance(Some(window), cx);
                cx.notify();
            }
        });
        // A focused control can leave the tree (a disabled pager button, a retired
        // thread list). Global bindings need the workspace back in the focus path.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            window.focus(&this.focus, cx);
        });
        let mut read_gate = crate::read_gate::Gate::default();
        read_gate.visibility(window.is_window_active());
        let lifetime = CancellationToken::new();
        let account_access = services.sync.read(cx).access();
        let composer = cx.new(|cx| {
            crate::composer::Composer::new(
                runtime.clone(),
                account_access.clone(),
                lifetime.child_token(),
                window,
                cx,
            )
        });
        let composer_events = cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if let crate::composer::Event::Closed(message) = event {
                window.push_notification(
                    Notification::success(message.clone()).delivery(NotificationDelivery::InApp),
                    cx,
                );
                this.navigate(this.scope.clone(), window, cx);
            }
            cx.notify();
        });
        let reminders = cx.new(|cx| {
            crate::reminders::Reminders::new(
                runtime.clone(),
                lifetime.child_token(),
                services.reminders,
                window,
                cx,
            )
        });
        let reminder_events = cx.subscribe_in(&reminders, window, |this, _, event, window, cx| {
            if let crate::reminders::Event::Open(record) = event {
                this.open_notification(PendingOpen::Reminder(record.clone()), window, cx);
            }
            cx.notify();
        });
        let sync_events = cx.subscribe_in(
            &services.sync,
            window,
            |this, _, event: &crate::mail_sync::Event, window, cx| match event {
                crate::mail_sync::Event::Sync(update) => {
                    this.sync_update(update.clone(), window, cx);
                }
                crate::mail_sync::Event::Open(click) => {
                    this.open_notification(PendingOpen::Mail(click.clone()), window, cx);
                }
            },
        );
        let preferences = cx.new(|cx| {
            crate::preferences::Preferences::new(
                runtime.clone(),
                lifetime.child_token(),
                window,
                cx,
            )
        });
        let preference_events =
            cx.subscribe_in(&preferences, window, |this, _, event, window, cx| {
                if let crate::preferences::Event::Saved(revision) = event
                    && *revision == this.declaration.revision()
                {
                    this.load_configuration(window, cx);
                }
                cx.notify();
            });
        let onboarding =
            cx.new(|cx| crate::onboarding::Onboarding::new(preferences.clone(), window, cx));
        let mut this = Self {
            focus,
            link_focus: cx.focus_handle(),
            runtime,
            owner: services.store.get().cloned(),
            shared_store: services.store,
            accounts: Vec::new(),
            folders: Vec::new(),
            unread: HashMap::new(),
            scope: Scope::Inbox,
            browse: crate::mailbox::Browse::default(),
            declaration: source::Runtime::default(),
            config_path: path,
            preferences,
            onboarding,
            _preference_events: preference_events,
            loading_config: false,
            startup_error: None,
            pending_open: None,
            view: View::Inbox,
            account_access,
            composer,
            _composer_events: composer_events,
            reminders,
            _reminder_events: reminder_events,
            account_views: Vec::new(),
            account_events: Vec::new(),
            account_lifetime: CancellationToken::new(),
            lifetime,
            query_cancel: CancellationToken::new(),
            read_gate,
            list,
            _list_events: list_events,
            thread_reader,
            _thread_events: thread_events,
            selected: None,
            move_picker: None,
            body: String::new(),
            text_reader,
            reader_mode: if demo {
                ReaderMode::Html
            } else {
                ReaderMode::Text
            },
            html_reader,
            _html_events: html_events,
            opened_message: None,
            opened_omitted: None,
            opened_analysis: None,
            corrected_deadline: None,
            preferred_reader: ReaderMode::Html,
            reader_notice: String::new(),
            headers_open: false,
            link_cancel: None,
            preview_cancel: None,
            previews: services.previews,
            search,
            filters,
            filter: SearchFilter::default(),
            _filter_events: filter_events,
            status: "Opening local mail…".into(),
            query: String::new(),
            query_generation: 0,
            reading_generation: 0,
            demo,
            _search_events: events,
            _activation: activation,
            _appearance: appearance,
            _focus_lost: focus_lost,
            theme_override: None,
            sync_session: services.sync,
            _sync_events: sync_events,
            fuzzy: None,
            meaning: None,
            search_mode: SearchMode::Words,
            search_pause: Task::ready(()),
            priority: None,
            priority_expiry: Task::ready(()),
            older_reached: HashMap::new(),
            older_fetch: None,
            announcer: None,
            syncing: BTreeSet::new(),
            sync_progress: BTreeMap::new(),
            cancelled_fetches: BTreeSet::new(),
            sync_errors: BTreeMap::new(),
            move_warnings: BTreeMap::new(),
            history: None,
            history_task: Task::ready(()),
            last_sync: String::new(),
            last_wake: Instant::now(),
        };
        if demo {
            this.init_demo(window, cx);
        } else {
            this.load_configuration(window, cx);
        }
        this
    }

    fn init_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status = "Synthetic preview · no accounts or network".into();
        let rows: Vec<_> = [
            (
                "The Mailstronaut team",
                "Welcome aboard",
                "A native space for your mail.",
            ),
            (
                "Doctoral school",
                "Réinscription en doctorat",
                "Synthetic example, no personal data.",
            ),
            (
                "Design notes",
                "A calmer inbox",
                "Built with standard GPUI Kit components.",
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (sender, subject, preview))| Row {
            account: "Demo".into(),
            id: i64::try_from(id).unwrap_or_default(),
            conversation: None,
            sender: sender.into(),
            subject: subject.into(),
            preview: preview.into(),
            unread: 1,
            messages: 1,
            received_at: 1_700_000_000,
            seen: false,
            starred: false,
            due: None,
            expires_at: None,
        })
        .collect();
        if let Some(row) = rows.first().cloned() {
            self.open(row, window, cx);
        }
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace(list, rows, window, cx);
        });
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One existing atomic configuration adoption path also updates its small preference snapshot"
    )]
    fn load_configuration(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.demo || self.loading_config || self.preferences.read(cx).busy() {
            return;
        }
        self.loading_config = true;
        self.preferences
            .update(cx, crate::preferences::Preferences::loading);
        self.status = "Loading native Nix configuration…".into();
        let path = self.config_path.clone();
        let owner = self.owner.clone();
        let shared_store = self.shared_store.clone();
        let cancel = self.lifetime.child_token();
        let job = self.runtime.spawn(async move {
            let path =
                path.ok_or_else(|| "Unable to locate native Nix configuration.".to_owned())?;
            let executable =
                std::env::current_exe().map_err(|_| "Unable to locate configuration helper.")?;
            let config = source::load(path.clone(), &executable, &cancel).await;
            let ownership = tokio::task::spawn_blocking(move || source::ownership(&path))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(source::Ownership::Unavailable);
            let owner = if owner.is_some() || config.is_err() {
                owner
            } else {
                let root = directories::BaseDirs::new()
                    .ok_or("Unable to locate local mail storage.")?
                    .data_local_dir()
                    .join("mailstronaut");
                Some(open_shared_store(&shared_store, root).await?)
            };
            Ok::<_, String>((owner, config, ownership))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.loading_config = false;
                let error = match result {
                    Ok(Ok((owner, config, ownership))) => {
                        this.startup_error = None;
                        this.owner = owner;
                        this.start_search(window, cx);
                        if this.declaration.apply(config) {
                            this.retire_open(window, cx);
                            this.list.update(cx, |list, cx| {
                                crate::mailbox::replace(list, Vec::new(), window, cx);
                            });
                            if let Some(config) = this.declaration.active().cloned() {
                                this.accounts = config
                                    .accounts
                                    .iter()
                                    .map(|account| account.id.clone())
                                    .collect();
                                if match &this.scope {
                                    Scope::Inbox => false,
                                    Scope::Account(account) | Scope::Folder { account, .. } => {
                                        !this.accounts.contains(account)
                                    }
                                } {
                                    this.scope = Scope::Inbox;
                                }
                                this.browse = crate::mailbox::Browse::default();
                                let zone = crate::mailbox::local_zone(config.time_zone.as_deref());
                                this.filters.update(cx, |filters, cx| {
                                    filters.set_zone(zone.clone(), window, cx);
                                });
                                let handle = this.handle();
                                this.reminders.update(cx, |reminders, cx| {
                                    reminders.configure(&config, handle, zone.clone(), cx);
                                });
                                this.list.update(cx, |list, _| {
                                    list.delegate_mut().time_zone = zone;
                                });
                                this.apply_appearance(&config, window, cx);
                                this.rebuild_accounts(&config, window, cx);
                                let handle = this.handle();
                                this.composer.update(cx, |composer, cx| {
                                    composer.configure(&config, handle, window, cx);
                                });
                                this.restart_sync(&config, window, cx);
                            }
                        }
                        if let Some(error) = this.declaration.error() {
                            this.status = if this.declaration.active().is_some() {
                                format!("{error}. The last valid configuration remains active.")
                            } else {
                                format!("{error}. No accounts have been started.")
                            };
                        } else {
                            this.refresh(window, cx);
                        }
                        let snapshot = crate::preferences::Snapshot {
                            path: this.config_path.clone(),
                            config: this.declaration.active().cloned(),
                            revision: this.declaration.revision(),
                            ownership,
                            error: this.declaration.error().map(|error| error.to_string()),
                        };
                        this.preferences.update(cx, |preferences, cx| {
                            preferences.configure(snapshot, window, cx);
                        });
                        if this.declaration.active().is_none() {
                            this.view = View::Accounts;
                            this.read_gate.visibility(false);
                        } else if let Some(target) = this.pending_open.take() {
                            this.open_notification(target, window, cx);
                        }
                        None
                    }
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some("Unable to open local mail.".into()),
                };
                if let Some(error) = error {
                    this.status.clone_from(&error);
                    if this.owner.is_none() {
                        this.startup_error = Some(error.clone());
                    }
                    this.preferences
                        .update(cx, |preferences, cx| preferences.failed(error, cx));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn rebuild_accounts(
        &mut self,
        config: &Arc<Config>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.account_lifetime.cancel();
        self.account_lifetime = self.lifetime.child_token();
        self.older_fetch = None;
        self.older_reached.clear();
        window.remove_notification::<AttachmentDownload>(cx);
        self.move_warnings.clear();
        self.account_events.clear();
        self.account_views.clear();
        let Some(handle) = self.handle() else {
            return;
        };
        let revision = self.declaration.revision();
        for account in &config.accounts {
            let account_id = account.id.clone();
            let inbox = account
                .folders
                .inbox
                .clone()
                .unwrap_or_else(|| "INBOX".into());
            let view = cx.new(|cx| {
                crate::accounts::AccountPane::new(
                    config.clone(),
                    account.clone(),
                    handle.clone(),
                    self.runtime.clone(),
                    self.account_access.clone(),
                    self.preferences.clone(),
                    self.account_lifetime.child_token(),
                    window,
                    cx,
                )
            });
            self.account_events.push(cx.subscribe_in(
                &view,
                window,
                move |this, _, event: &crate::accounts::Event, window, cx| {
                    if revision == this.declaration.revision() {
                        match event {
                            crate::accounts::Event::CacheChanged => this.refresh_rows(window, cx),
                            crate::accounts::Event::CacheEvicted => {
                                if this
                                    .selected
                                    .as_ref()
                                    .is_some_and(|item| item.account == account_id)
                                {
                                    this.retire_open(window, cx);
                                }
                                this.wake_cache_indexes();
                                this.refresh_rows(window, cx);
                            }
                            crate::accounts::Event::CredentialsChanged => {
                                this.sync_session
                                    .read(cx)
                                    .include_folder(&account_id, inbox.clone());
                            }
                        }
                    }
                },
            ));
            self.account_views.push(view);
        }
    }

    fn apply_appearance(&mut self, config: &Config, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_override = match config.appearance {
            Appearance::Dark => Some(ThemeMode::Dark),
            Appearance::Light => Some(ThemeMode::Light),
            _ => None,
        };
        if let Some(mode) = self.theme_override {
            Theme::change(mode, Some(window), cx);
        } else {
            Theme::sync_system_appearance(Some(window), cx);
        }
    }

    fn toggle_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().mode.is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.theme_override = Some(mode);
        Theme::change(mode, Some(window), cx);
        cx.notify();
    }

    fn focus_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_some() && self.view == View::Inbox {
            self.cancel_reader_requests();
            self.reader_mode = ReaderMode::Text;
            self.preferred_reader = ReaderMode::Text;
            self.html_reader
                .update(cx, crate::html_reader::HtmlReader::hide);
            self.mark_loaded(window, cx);
            self.text_reader
                .update(cx, |reader, cx| reader.focus(window, cx));
            cx.notify();
        }
    }

    fn handle(&self) -> Option<Handle> {
        self.owner.as_ref().map(|owner| owner.handle())
    }

    #[cfg(test)]
    pub(crate) fn uses_owner(&self, owner: &Arc<Worker>) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, owner))
    }

    fn retire_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.move_picker = None;
        self.text_reader
            .update(cx, |reader, cx| reader.set_value("", window, cx));
        self.thread_reader
            .update(cx, crate::thread_reader::ThreadReader::clear);
        self.cancel_reader_requests();
        self.reading_generation = self.reading_generation.wrapping_add(1);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.opened_message = None;
        self.opened_omitted = None;
        self.opened_analysis = None;
        self.corrected_deadline = None;
        self.headers_open = false;
        self.reader_notice.clear();
        self.read_gate
            .suppress(self.declaration.revision(), self.reading_generation);
        self.selected = None;
        self.body.clear();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        if query != self.query {
            self.query = query;
            self.browse = crate::mailbox::Browse::default();
            self.clear_page(window, cx);
        }
        self.refresh_rows(window, cx);
    }

    /// Nothing else happens when a priority's time runs out, so the list must look again.
    fn watch_priority_expiry(&mut self, group: &[Row], window: &Window, cx: &Context<Self>) {
        let now = chrono::Utc::now().timestamp();
        self.priority_expiry = group
            .iter()
            .filter_map(|row| row.expires_at)
            .min()
            .map_or_else(
                || Task::ready(()),
                |expires_at| {
                    let wait = u64::try_from(expires_at - now).unwrap_or_default() + 1;
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_secs(wait))
                            .await;
                        _ = this.update_in(cx, |this, window, cx| this.refresh_rows(window, cx));
                    })
                },
            );
    }

    /// Asks for every conversation that needs attention, rather than the first few.
    fn show_all_priority(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.list.update(cx, |list, _| {
            list.delegate_mut().show_all_priority = true;
        });
        self.refresh_rows(window, cx);
    }

    fn dismiss_priority(&self, message: i64, window: &Window, cx: &Context<Self>) {
        let Some(store) = self.handle() else {
            return;
        };
        let job = self
            .runtime
            .spawn(async move { store.dismiss_priority(message).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if matches!(result, Ok(Ok(()))) {
                    this.refresh_rows(window, cx);
                } else {
                    this.status = "Unable to dismiss this message.".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_priority(&mut self, config: &Config, window: &Window, cx: &Context<Self>) {
        self.priority = None;
        let Some(store) = self.handle().filter(|_| config.priority.enabled) else {
            return;
        };
        let (engine, mut changed) = crate::mail_priority::PriorityEngine::start(
            &self.runtime,
            store,
            config,
            self.account_lifetime.child_token(),
        );
        self.priority = Some(engine);
        let revision = self.declaration.revision();
        cx.spawn_in(window, async move |this, cx| {
            while changed.recv().await.is_some() {
                let applied = this.update_in(cx, |this, window, cx| {
                    if revision == this.declaration.revision() {
                        this.refresh_rows(window, cx);
                    }
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn start_meaning(&mut self, config: &Config, window: &Window, cx: &Context<Self>) {
        let accounts: Vec<_> = config
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect();
        if config.search.semantic
            && self.meaning.as_ref().is_some_and(|engine| {
                engine.configured(&config.search, &accounts, config.sync.max_body_bytes)
            })
        {
            return;
        }
        if let Some(old) = self.meaning.take() {
            old.stop();
        }
        if !config.search.semantic || accounts.is_empty() {
            self.search_mode = SearchMode::Words;
            return;
        }
        let (Some(store), Some(directories)) = (self.handle(), directories::BaseDirs::new()) else {
            return;
        };
        let (engine, mut changed) = crate::mail_semantic::Meaning::start(
            self.runtime.handle(),
            directories.data_local_dir().join("mailstronaut/meaning"),
            config.search.clone(),
            accounts,
            store,
            self.lifetime.child_token(),
            config.sync.max_body_bytes,
        );
        let cancelled = engine.cancel();
        self.meaning = Some(engine);
        cx.spawn_in(window, async move |this, cx| {
            let mut previous_revision = None;
            loop {
                tokio::select! {
                    biased;
                    () = cancelled.cancelled() => return,
                    result = changed.changed() => { if result.is_err() { return; } }
                }
                if cancelled.is_cancelled() {
                    return;
                }
                let state = changed.borrow().clone();
                let revision = match state {
                    crate::mail_semantic::State::Ready { revision, .. } => Some(revision),
                    _ => None,
                };
                let changed_index = revision != previous_revision;
                previous_revision = revision;
                if this
                    .update_in(cx, |this, window, cx| {
                        let retired = this.search_mode == SearchMode::Related
                            && matches!(state, crate::mail_semantic::State::Unavailable(_));
                        if retired {
                            this.search_mode = SearchMode::Words;
                        }
                        if retired
                            || (changed_index
                                && this.search_mode == SearchMode::Related
                                && related_intent(&this.query))
                        {
                            this.refresh_rows(window, cx);
                        } else {
                            this.status = this.page_status(this.browse.loaded);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    fn start_search(&mut self, window: &Window, cx: &Context<Self>) {
        if self.fuzzy.is_some() {
            return;
        }
        let (Some(store), Some(directories)) = (self.handle(), directories::BaseDirs::new()) else {
            return;
        };
        // A sibling of the mail store, which is the only source of what is in it.
        let directory = directories
            .data_local_dir()
            .join("mailstronaut")
            .join("search");
        let index = crate::mail_search::MailSearch::start(
            &self.runtime,
            directory,
            store,
            self.lifetime.child_token(),
        );
        let mut progress = index.progress();
        self.fuzzy = Some(index);
        cx.spawn_in(window, async move |this, cx| {
            while progress.changed().await.is_ok() {
                // Coalesce small batches into occasional status updates during a long rebuild.
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let ready = *progress.borrow_and_update() == Some(0);
                if this
                    .update_in(cx, |this, window, cx| {
                        let partial = !ready && this.search_mode == SearchMode::Words;
                        this.list.update(cx, |list, cx| {
                            if list.delegate().indexing_words != partial {
                                list.delegate_mut().indexing_words = partial;
                                cx.notify();
                            }
                        });
                        if ready
                            && this.view == View::Inbox
                            && this.search_mode == SearchMode::Words
                            && (!this.query.trim().is_empty() || !this.filter.is_empty())
                        {
                            this.refresh_rows(window, cx);
                        } else {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn page_status(&self, count: usize) -> String {
        let base = self.lexical_page_status(count);
        match &self.meaning {
            Some(meaning) => format!("{base} · {}", meaning.label()),
            None => base,
        }
    }

    fn lexical_page_status(&self, count: usize) -> String {
        if self.search_mode == SearchMode::Related && related_intent(&self.query) {
            format!("{count} related conversations · suggested by meaning")
        } else if self.query.trim().is_empty() && self.filter.is_empty() {
            format!("{count} cached conversations · local cache")
        } else {
            let matches = if count == 1 { "match" } else { "matches" };
            format!("{count} {matches} · searched local cache")
        }
    }

    fn show_page(&mut self, page: ListPage, window: &mut Window, cx: &mut Context<Self>) {
        let (folders, unread, warnings, rows, group, aliases) = page;
        self.move_warnings = warnings
            .into_iter()
            .filter(|(id, context)| {
                self.declaration
                    .active()
                    .and_then(|config| config.accounts.iter().find(|account| &account.id == id))
                    .and_then(|account| mailstronaut_core::store::flags::context(account).ok())
                    .as_ref()
                    == Some(context)
            })
            .collect();
        self.watch_priority_expiry(&group.rows, window, cx);
        if self.folders != folders {
            self.folders = folders;
            if self.move_picker.is_some() {
                self.reset_move_picker(window, cx);
            }
        }
        self.unread = unread
            .into_iter()
            .map(|(folder, count)| (folder, count as usize))
            .collect();
        let count = rows.len();
        self.browse.more = count > 0 && count.is_multiple_of(100);
        self.browse.after = rows.last().map(|row| Cursor {
            received_at: row.latest.received_at,
            message_id: row.latest.id,
        });
        self.browse.loaded = count;
        let rows = rows.into_iter().map(Row::from).collect();
        let excerpt = (self.search_mode == SearchMode::Words)
            .then(|| BodyExcerpt::for_words(&self.query))
            .flatten();
        let workspace = cx.entity().downgrade();
        let revision = self.declaration.revision();
        let scope = self.scope.clone();
        self.list.update(cx, |list, cx| {
            list.delegate_mut().on_row_action = Some(Rc::new(move |target, action, window, cx| {
                _ = workspace.update(cx, |this, cx| {
                    if this.view == View::Inbox
                        && this.declaration.revision() == revision
                        && this.scope == scope
                    {
                        this.row_action(target, action, window, cx);
                    }
                });
            }));
            crate::mailbox::replace_with_priority(list, rows, group, &aliases, window, cx);
            list.delegate_mut().body_excerpt = excerpt;
            list.delegate_mut().more = self.browse.more;
        });
        self.status = if self.accounts.is_empty() {
            "Add an account in Settings or your native Nix configuration.".into()
        } else {
            self.page_status(count)
        };
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One query/configuration generation fence covers this native refresh transaction."
    )]
    fn refresh_rows(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        self.query_generation = self.query_generation.wrapping_add(1);
        self.browse.pending = false;
        let generation = self.query_generation;
        let revision = self.declaration.revision();
        let accounts = self.accounts.clone();
        self.query_cancel.cancel();
        self.query_cancel = self.lifetime.child_token();
        let cancel = self.query_cancel.clone();
        let query = self.query.clone();
        let filter = self.filter.clone();
        let scope = self.scope.clone();
        let boundary = self.browse.after;
        let fuzzy = self.fuzzy.clone();
        let meaning = self
            .meaning
            .clone()
            .filter(|_| self.search_mode == SearchMode::Related);
        // The group belongs at the top of the unified view, never in searches.
        let show_priority = self.priority.is_some()
            && self.scope == Scope::Inbox
            && query.trim().is_empty()
            && filter.is_empty();
        let show_all = self.list.read(cx).delegate().show_all_priority;
        let displayed = self.list.read(cx).delegate();
        let previous = displayed
            .items
            .iter()
            .chain(&displayed.priority.rows)
            .filter_map(|row| row.conversation)
            .collect();
        self.list.update(cx, |list, cx| {
            let rows = list.delegate_mut();
            rows.loading = true;
            rows.searching = !query.is_empty() || !filter.is_empty();
            rows.indexing_words = self.search_mode == SearchMode::Words
                && self.fuzzy.as_ref().is_some_and(|index| index.queued() > 0);
            rows.has_accounts = !accounts.is_empty();
            cx.notify();
        });
        self.status = "Searching local cache…".into();
        let job = self.runtime.spawn(async move {
            let executable = std::env::current_exe()
                .map_err(|_| "Unable to locate the conversation helper.".to_owned())?;
            for account in &accounts {
                if let Some(snapshot) = handle
                    .conversation_snapshot(account.clone())
                    .await
                    .map_err(|error| error.to_string())?
                {
                    let groups = mailstronaut_core::threads::process::project(
                        &executable,
                        &snapshot.messages,
                        &cancel,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                    handle
                        .commit_conversations(snapshot, groups)
                        .await
                        .map_err(|error| error.to_string())?;
                }
            }
            if cancel.is_cancelled() {
                return Err("Local query cancelled.".into());
            }
            let folders = handle
                .folders(accounts.clone())
                .await
                .map_err(|error| error.to_string())?;
            let unread = handle
                .unread_counts(accounts.clone())
                .await
                .map_err(|error| error.to_string())?;
            let warnings = handle
                .move_warnings(accounts.clone())
                .await
                .map_err(|error| error.to_string())?;
            let group = if show_priority {
                priority_rows(&handle, accounts.clone(), show_all)
                    .await
                    .map_err(|error| error.to_string())?
            } else {
                Priority::default()
            };
            let rows = conversation_prefix(
                &handle,
                fuzzy.as_ref(),
                meaning.as_ref(),
                accounts.clone(),
                scope,
                query,
                boundary,
                &cancel,
                filter,
            )
            .await?;
            let aliases =
                merged_row_aliases(&handle, &accounts, previous, &rows, &group, &cancel).await?;
            Ok((folders, unread, warnings, rows, group, aliases))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if generation != this.query_generation || revision != this.declaration.revision() {
                    return;
                }
                this.list.update(cx, |list, cx| {
                    list.delegate_mut().loading = false;
                    cx.notify();
                });
                match result {
                    Ok(Ok(page)) => this.show_page(page, window, cx),
                    Ok(Err(error)) => this.status = error,
                    Err(_) => this.status = "Local query interrupted.".into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        self.headers_open = false;
        if let Some(handle) = self.handle() {
            let zone = self.list.read(cx).delegate().time_zone.clone();
            self.thread_reader.update(cx, |thread, cx| {
                thread.show(row.clone(), handle, zone, window, cx);
            });
        }
        self.cancel_reader_requests();
        self.reading_generation = self.reading_generation.wrapping_add(1);
        let generation = self.reading_generation;
        let revision = self.declaration.revision();
        self.read_gate.clicked(revision, generation);
        self.selected = Some(row.clone());
        self.reset_move_picker(window, cx);
        self.opened_message = None;
        self.reader_notice.clear();
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.body = "Opening cached message…".into();
        self.text_reader
            .update(cx, |reader, cx| reader.set_value("", window, cx));
        if self.demo {
            // A synthetic plain-text alternative; real messages use the MIME projection.
            let mut text = "A little more space for your mail.\n\nThis synthetic email has a native, read-only text alternative.\nTables, inline CSS and Unicode: Bonjour · Ciao · 世界.\n".to_owned();
            for line in 1..=120 {
                use std::fmt::Write as _;
                _ = writeln!(
                    text,
                    "\nSynthetic reader line {line:03} · Bonjour · Ciao · 世界."
                );
            }
            text.push_str("\nEnd of synthetic message · fin · 世界.");
            self.text_reader
                .update(cx, |reader, cx| reader.set_value(text, window, cx));
            self.body.clear();
            self.reader_mode = self.preferred_reader;
            if self.reader_mode == ReaderMode::Html {
                self.present_html(cx);
            }
        } else if let Some(handle) = self.handle() {
            let job = self.runtime.spawn(async move {
                let message = handle.message(row.account.clone(), row.id).await?;
                let omitted = handle.omitted(row.account.clone(), row.id).await?;
                // A result from another version may not parse, and then it says nothing.
                let analysis = handle
                    .analysis_data(row.id)
                    .await?
                    .and_then(|data| serde_json::from_str(&data).ok());
                let corrected = handle
                    .deadline_correction(row.account.clone(), row.id)
                    .await?;
                Ok::<_, mailstronaut_core::worker::Error>((message, omitted, analysis, corrected))
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = job.await;
                _ = this.update_in(cx, |this, window, cx| {
                    if generation != this.reading_generation
                        || revision != this.declaration.revision()
                    {
                        return;
                    }
                    match result {
                        Ok(Ok((message, omitted, analysis, corrected))) => {
                            this.opened_omitted = omitted;
                            this.opened_analysis = analysis;
                            this.corrected_deadline = corrected;
                            let readable = message.readable_text();
                            let text = readable.map_or_else(
                                || "No readable body is cached.".into(),
                                |text| crate::mailbox::display_text(&text),
                            );
                            this.text_reader
                                .update(cx, |reader, cx| reader.set_value(text, window, cx));
                            this.body.clear();
                            this.opened_message = Some(Arc::new(message));
                            this.reader_mode =
                                if this.preferred_reader == ReaderMode::Html && this.has_html() {
                                    ReaderMode::Html
                                } else {
                                    ReaderMode::Text
                                };
                            if this.reader_mode == ReaderMode::Text {
                                this.mark_loaded(window, cx);
                            } else {
                                this.present_html(cx);
                            }
                        }
                        Ok(Err(error)) => this.body = error.to_string(),
                        Err(_) => this.body = "Message load interrupted.".into(),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn has_html(&self) -> bool {
        self.demo
            || self.opened_message.as_ref().is_some_and(|message| {
                message.html.is_some() && !message.status.encrypted_unsupported
            })
    }

    fn present_html(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_none() || !self.has_html() || self.view != View::Inbox {
            return;
        }
        self.reader_mode = ReaderMode::Html;
        self.preferred_reader = ReaderMode::Html;
        let content = if self.demo {
            crate::html_worker::Content::Demo
        } else {
            let Some(message) = self.opened_message.clone() else {
                return;
            };
            crate::html_worker::Content::Mail(message)
        };
        self.html_reader.update(cx, |reader, cx| {
            reader.show(Arc::new(content), self.reading_generation, cx);
        });
        cx.notify();
    }

    fn html_event(
        &mut self,
        event: &crate::html_reader::Event,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let crate::html_reader::Event::Link(epoch, link) = event {
            self.open_link(*epoch, link.clone(), window, cx);
            return;
        }
        let (crate::html_reader::Event::Ready(epoch) | crate::html_reader::Event::Failed(epoch)) =
            event
        else {
            return;
        };
        if *epoch != self.reading_generation
            || self.reader_mode == ReaderMode::Text
            || self.view != View::Inbox
        {
            return;
        }
        if matches!(event, crate::html_reader::Event::Failed(_)) {
            self.reader_mode = ReaderMode::Text;
            self.reader_notice = "HTML could not be displayed. Showing cached text.".into();
            self.html_reader
                .update(cx, crate::html_reader::HtmlReader::hide);
        }
        self.mark_loaded(window, cx);
        cx.notify();
    }

    fn cancel_reader_requests(&mut self) {
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.cancel();
            self.reader_notice.clear();
        }
        if let Some(cancel) = self.link_cancel.take() {
            cancel.cancel();
            self.reader_notice.clear();
        }
    }

    fn open_link(
        &mut self,
        epoch: u64,
        link: mailstronaut_render::Link,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.reading_generation
            || self.view != View::Inbox
            || self.reader_mode != ReaderMode::Html
            || self.link_cancel.is_some()
            || self.selected.is_none()
        {
            return;
        }
        let revision = self.declaration.revision();
        let cancel = self.lifetime.child_token();
        self.link_cancel = Some(cancel.clone());
        self.reader_notice = "Opening link in your browser…".into();
        let work_cancel = cancel.clone();
        let job = self
            .runtime
            .spawn(async move { crate::link_portal::open(link, &work_cancel).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if cancel.is_cancelled()
                    || epoch != this.reading_generation
                    || revision != this.declaration.revision()
                {
                    return;
                }
                this.link_cancel = None;
                this.reader_notice = match result {
                    Ok(Ok(())) => "Link sent to your browser.",
                    Ok(Err(error)) => error,
                    Err(_) => "Browser request was interrupted.",
                }
                .into();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn mark_loaded(&mut self, window: &Window, cx: &Context<Self>) {
        let eligible = self
            .opened_message
            .as_ref()
            .is_some_and(|message| message.readable_text().is_some());
        if self.read_gate.take(
            self.declaration.revision(),
            self.reading_generation,
            eligible,
        ) {
            self.mark_opened(window, cx);
        }
    }

    fn mark_opened(&mut self, window: &Window, cx: &Context<Self>) {
        self.apply_flag(FlagChange::Seen, true, window, cx);
    }

    /// An explicit toggle stays until the next explicit open: the read gate is spent,
    /// so refreshing or reselecting this message cannot mark it read again.
    fn toggle_flag(&mut self, flag: FlagChange, window: &Window, cx: &Context<Self>) {
        let Some(row) = &self.selected else {
            return;
        };
        let desired = match flag {
            FlagChange::Seen => !row.seen,
            FlagChange::Starred => !row.starred,
        };
        self.apply_flag(flag, desired, window, cx);
    }

    fn apply_flag(&mut self, flag: FlagChange, desired: bool, window: &Window, cx: &Context<Self>) {
        let Some(row) = &self.selected else {
            return;
        };
        self.apply_row_flag(row.account.clone(), row.id, flag, desired, window, cx);
    }

    fn apply_row_flag(
        &mut self,
        account_id: String,
        id: i64,
        flag: FlagChange,
        desired: bool,
        window: &Window,
        cx: &Context<Self>,
    ) {
        let Some(handle) = self.handle() else {
            return;
        };
        let Some(account) = self.declaration.active().and_then(|config| {
            config
                .accounts
                .iter()
                .find(|account| account.id == account_id)
        }) else {
            return;
        };
        let Ok(context) = mailstronaut_core::store::flags::context(account) else {
            return;
        };
        if let Some(row) = self
            .selected
            .as_mut()
            .filter(|row| row.account == account_id && row.id == id)
        {
            match flag {
                FlagChange::Seen => row.seen = desired,
                FlagChange::Starred => row.starred = desired,
            }
        }
        let revision = self.declaration.revision();
        let job = self.runtime.spawn(async move {
            handle
                .set_flag(account_id, id, flag, desired, context)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() {
                    return;
                }
                match result {
                    Ok(Ok(())) => {
                        this.refresh_rows(window, cx);
                        this.thread_reader
                            .update(cx, |thread, cx| thread.refresh(window, cx));
                        // Changes reach the server on the account's next cycle.
                        this.sync_session.read(cx).wake_all();
                    }
                    Ok(Err(error)) => {
                        this.status = error.to_string();
                        if let Some(row) = this.selected.as_mut().filter(|row| row.id == id) {
                            match flag {
                                FlagChange::Seen => row.seen = !desired,
                                FlagChange::Starred => row.starred = !desired,
                            }
                        }
                    }
                    Err(_) => this.status = "Unable to save the message's state.".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn account_folders(&self, account: &str) -> impl Iterator<Item = &Folder> {
        self.folders
            .iter()
            .filter(move |folder| folder.account == account)
    }

    fn row_action(
        &mut self,
        target: &Row,
        action: crate::mailbox::RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::mailbox::RowAction;
        let Some(row) = self.list.read(cx).delegate().current(target).cloned() else {
            return;
        };
        match action {
            RowAction::Open => self.open(row, window, cx),
            RowAction::Flag(flag, desired) => {
                self.apply_row_flag(row.account, row.id, flag, desired, window, cx);
            }
            RowAction::Move(role) => self.move_row_to_role(row, role, window, cx),
        }
    }

    fn move_to_role(&mut self, role: FolderRole, window: &Window, cx: &mut Context<Self>) {
        if let Some(row) = self.selected.clone() {
            self.move_row_to_role(row, role, window, cx);
        }
    }

    fn move_row_to_role(
        &mut self,
        row: Row,
        role: FolderRole,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let destination = self
            .account_folders(&row.account)
            .find(|folder| folder.role == Some(role))
            .map(|folder| folder.name.clone());
        if let Some(destination) = destination {
            self.move_row(row, destination, role.label().to_owned(), window, cx);
        } else {
            let message = format!(
                "This account has no {} folder. Set one in your Nix configuration.",
                role.label()
            );
            self.reader_notice.clone_from(&message);
            self.status = message;
            cx.notify();
        }
    }

    /// Hide the open conversation now and report it with Undo. The server is told on
    /// the account's next cycle, and only copies in the current folder move.
    fn move_open(
        &mut self,
        destination: String,
        label: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.move_picker.take().is_none() {
            return;
        }
        cx.notify();
        if let Some(row) = self.selected.clone() {
            self.move_row(row, destination, label, window, cx);
        }
    }

    fn move_row(
        &self,
        row: Row,
        destination: String,
        label: String,
        window: &Window,
        cx: &Context<Self>,
    ) {
        let Some(handle) = self.handle() else {
            return;
        };
        let Some(context) = self.declaration.active().and_then(|config| {
            let account = config.accounts.iter().find(|item| item.id == row.account)?;
            mailstronaut_core::store::flags::context(account).ok()
        }) else {
            return;
        };
        let source = match &self.scope {
            Scope::Folder { folder, .. } => Some(*folder),
            _ => None,
        };
        let revision = self.declaration.revision();
        let generation = self.reading_generation;
        let (account, id) = (row.account.clone(), row.id);
        // The hold is stored with the move: IDLE can start a sync pass at any moment, and
        // opening a message is usually what makes it happen.
        let not_before = chrono::Utc::now()
            .timestamp()
            .saturating_add(i64::try_from(UNDO_GRACE.as_secs()).unwrap_or(i64::MAX));
        let job = self.runtime.spawn(async move {
            handle
                .queue_move(account, id, source, destination, context, not_before)
                .await
        });
        let undo = (row.account, row.id);
        let conversation = row.conversation;
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() {
                    return;
                }
                let current = generation == this.reading_generation;
                match result {
                    Ok(Ok(_)) => {
                        if current
                            && this.selected.as_ref().is_some_and(|open| {
                                open.account == undo.0
                                    && (open.id == undo.1
                                        || conversation.is_some()
                                            && open.conversation == conversation)
                            })
                        {
                            this.retire_open(window, cx);
                        }
                        this.after_move(undo, &label, window, cx);
                    }
                    error if current => {
                        this.reader_notice = match error {
                            Ok(Err(error)) => error.to_string(),
                            _ => "Unable to move this message.".into(),
                        };
                        this.reset_move_picker(window, cx);
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn after_move(
        &mut self,
        undo: (String, i64),
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_rows(window, cx);
        // Waking the account at once would claim the move before Undo could be pressed.
        let grace = cx.background_executor().timer(UNDO_GRACE);
        cx.spawn(async move |this, cx| {
            grace.await;
            _ = this.update(cx, |this, cx| {
                this.sync_session.read(cx).wake_all();
            });
        })
        .detach();
        let workspace = cx.entity().downgrade();
        window.push_notification(
            Notification::info(format!("Moved to {label}"))
                .delivery(NotificationDelivery::InApp)
                .action(move |_, _, cx| {
                    let (workspace, undo) = (workspace.clone(), undo.clone());
                    Button::new("undo-move").label("Undo").on_click(cx.listener(
                        move |notification, _, window, cx| {
                            _ = workspace.update(cx, |this, cx| {
                                this.undo_move(undo.clone(), window, cx);
                            });
                            notification.dismiss(window, cx);
                        },
                    ))
                })
                // `action` turns autohide off; Undo is only useful during the grace period.
                .autohide(true),
            cx,
        );
    }

    fn undo_move(&self, (account, id): (String, i64), window: &Window, cx: &Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        let job = self
            .runtime
            .spawn(async move { handle.undo_move(account, id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if matches!(result, Ok(Ok(()))) {
                    this.refresh_rows(window, cx);
                } else {
                    this.status = "Too late to undo: the server already has this change.".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A move that will be tried again keeps its warning until it works. One this server
    /// can never do is reported once and given up on, so it is shown until dismissed.
    fn report_move_failure(
        &mut self,
        account: &str,
        failure: Option<mailstronaut_mail::imap::Error>,
    ) {
        use mailstronaut_mail::imap::Error;
        match failure {
            None | Some(Error::MoveUnavailable) => {
                self.sync_errors.remove(account);
            }
            Some(error) => {
                self.sync_errors.insert(
                    account.to_owned(),
                    crate::mail_sync::Failure {
                        message: format!("A message could not be moved. {error}."),
                        retry_after: None,
                    },
                );
            }
        }
    }

    /// Restart per-account workers for the active configuration.
    fn restart_sync(&mut self, config: &Arc<Config>, window: &Window, cx: &mut Context<Self>) {
        self.start_meaning(config, window, cx);
        self.start_priority(config, window, cx);
        self.sync_errors.clear();
        self.syncing.clear();
        self.sync_progress.clear();
        self.cancelled_fetches.clear();
        self.history = None;
        self.refresh_history(cx);
        let handle = self.handle();
        self.sync_session.update(cx, |session, cx| {
            session.set_window_active(window.is_window_active());
            session.configure(config, handle, cx);
        });
        self.announcer = self.sync_session.read(cx).announcer();
    }

    fn sync_update(
        &mut self,
        update: crate::mail_sync::Update,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        match update {
            crate::mail_sync::Update::Started(account) => {
                self.sync_progress.remove(&account);
                self.cancelled_fetches.remove(&account);
                self.syncing.insert(account);
            }
            crate::mail_sync::Update::Progress(account, progress) => {
                if self.syncing.contains(&account) {
                    let first_window = progress.completed > 0
                        && self
                            .sync_progress
                            .get(&account)
                            .is_none_or(|previous| previous.completed == 0);
                    self.sync_progress.insert(account, progress);
                    if first_window && self.browse.loaded == 0 && self.query.is_empty() {
                        self.refresh_rows(window, cx);
                    }
                }
            }
            crate::mail_sync::Update::Cancelled(account) => {
                self.finish_sync(&account, cx);
                self.cancelled_fetches.insert(account);
                self.refresh_rows(window, cx);
            }
            crate::mail_sync::Update::Finished(account, result) => {
                self.finish_sync(&account, cx);
                match result {
                    Ok(cycle) => {
                        self.report_move_failure(&account, cycle.move_failure);
                        self.last_sync = chrono::Local::now().format("%H:%M").to_string();
                        if cycle.downloaded > 0
                            || cycle.refreshed > 0
                            || cycle.actions > 0
                            || cycle.move_failure.is_some()
                        {
                            self.refresh_rows(window, cx);
                        }
                    }
                    Err(error) => {
                        self.sync_errors.insert(account, error);
                        // Whatever failed, the pass may have put a hidden message back.
                        self.refresh_rows(window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn finish_sync(&mut self, account: &str, cx: &Context<Self>) {
        self.syncing.remove(account);
        self.sync_progress.remove(account);
        self.refresh_history(cx);
        self.wake_cache_indexes();
    }

    fn wake_cache_indexes(&self) {
        if let Some(search) = &self.fuzzy {
            search.wake();
        }
        if let Some(meaning) = &self.meaning {
            meaning.wake();
        }
        if let Some(priority) = &self.priority {
            priority.wake();
        }
    }

    fn sync_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(window, cx);
        self.refresh_history(cx);
        self.sync_session.read(cx).sync_all();
    }

    fn sync_status(&self) -> String {
        let status = if !self.syncing.is_empty() {
            format!("{} · syncing…", self.status)
        } else if self.last_sync.is_empty() {
            self.status.clone()
        } else {
            format!("{} · updated {}", self.status, self.last_sync)
        };
        if self.view == View::Inbox
            && let (Some(history), Some(config)) = (&self.history, self.declaration.active())
            && let Some(label) = history::label(
                &self.scope,
                &self.accounts,
                history,
                config.sync.initial_days,
            )
        {
            return format!("{status} · {label}");
        }
        status
    }

    fn fetch_progress(&self, cx: &Context<Self>) -> Vec<gpui::Div> {
        self.syncing
            .iter()
            .enumerate()
            .map(|(index, account)| {
                let fetch = self.sync_progress.get(account);
                let (label, percent) = Self::format_fetch_progress(account, fetch);
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().child(label.clone()))
                    .child(
                        Progress::new(("sync-progress", index))
                            .w(px(120.))
                            .accessibility_label(label)
                            .loading(percent.is_none())
                            .value(percent.unwrap_or_default()),
                    )
                    .when_some(fetch, |row, fetch| {
                        let cancel = fetch.cancel.clone();
                        row.child(
                            Button::new(format!("cancel-fetch-{account}"))
                                .ghost()
                                .label(if cancel.is_cancelled() {
                                    "Stopping…"
                                } else {
                                    "Cancel"
                                })
                                .disabled(cancel.is_cancelled())
                                .accessibility_label(format!("Cancel fetch for {account}"))
                                .tooltip(format!("Cancel fetch for {account}"))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cancel.cancel();
                                    cx.notify();
                                })),
                        )
                    })
            })
            .collect()
    }

    fn format_fetch_progress(
        account: &str,
        progress: Option<&crate::mail_sync::Fetch>,
    ) -> (String, Option<f32>) {
        let Some(progress) = progress else {
            return (format!("{account} · connecting…"), None);
        };
        if progress.total.is_none() && progress.completed == 0 {
            return (format!("{account} · {}: fetching…", progress.folder), None);
        }
        let label = progress.total.map_or_else(
            || {
                format!(
                    "{} · {}: {} reviewed",
                    account, progress.folder, progress.completed
                )
            },
            |total| {
                format!(
                    "{} · {}: {} / {} reviewed",
                    account, progress.folder, progress.completed, total
                )
            },
        );
        #[expect(
            clippy::cast_possible_truncation,
            reason = "A bounded percentage is sufficient precision for the progress indicator"
        )]
        let percent = progress.total.filter(|total| *total > 0).map(|total| {
            (100.0 * f64::from(progress.completed.min(total)) / f64::from(total)) as f32
        });
        (label, percent)
    }

    fn sync_alert(&self, cx: &Context<Self>) -> impl IntoElement {
        div().when_some(self.sync_errors.iter().next(), |view, (account, error)| {
            let name = self
                .declaration
                .active()
                .and_then(|config| config.accounts.iter().find(|item| &item.id == account))
                .map_or_else(|| account.clone(), |item| self.account_label(item));
            let mut message = error.retry_after.map_or_else(
                || error.message.clone(),
                |delay| {
                    format!(
                        "{}. Retrying after a {}s delay.",
                        error.message.trim_end_matches('.'),
                        delay.as_secs()
                    )
                },
            );
            if self.sync_errors.len() > 1 {
                message.push_str(" More accounts need attention.");
            }
            let dismiss = account.clone();
            let retry = account.clone();
            view.p_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Alert::warning(format!("sync-error-{account}"), message)
                        .title(name.clone())
                        .text_color(cx.theme().foreground)
                        .w_0()
                        .min_w_0()
                        .flex_1()
                        .on_close(cx.listener(move |this, _, _, cx| {
                            this.sync_errors.remove(&dismiss);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(format!("retry-sync-{account}"))
                        .label("Retry")
                        .accessibility_label(format!("Retry sync for {name}"))
                        .disabled(self.syncing.contains(account))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sync_session.read(cx).retry(&retry);
                        })),
                )
        })
    }

    fn permanent_move_alert(&self, cx: &Context<Self>) -> impl IntoElement {
        div().when_some(self.move_warnings.iter().next(), |view, (account, context)| {
            let (id, context) = (account.clone(), context.clone());
            let name = self.declaration.active().and_then(|config| config.accounts.iter().find(|item| &item.id == account))
                .map_or(account.as_str(), |item| item.display_name.as_str());
            view.child(Alert::warning(format!("move-refused-{account}"),
                "This mail server cannot safely move a single message. The message stayed in its original folder.")
                .title(name.to_owned())
                .on_close(cx.listener(move |this, _, window, cx| {
                    this.dismiss_move_warning(id.clone(), context.clone(), window, cx);
                })))
        })
    }

    fn dismiss_move_warning(
        &self,
        account: String,
        context: String,
        window: &Window,
        cx: &Context<Self>,
    ) {
        let Some(handle) = self.handle() else { return };
        let revision = self.declaration.revision();
        let job = self
            .runtime
            .spawn(async move { handle.dismiss_move_warning(account, context).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision == this.declaration.revision() {
                    if matches!(result, Ok(Ok(()))) {
                        this.refresh_rows(window, cx);
                    } else {
                        this.status = "Unable to dismiss the move warning.".into();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Workspace {
    fn bind_keys(cx: &mut Context<Self>) {
        cx.bind_keys([
            KeyBinding::new("ctrl-k", FocusSearch, Some("Mailstronaut")),
            KeyBinding::new("ctrl-r", RefreshCache, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-d", ToggleAppearance, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-t", FocusReader, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-l", FocusLinks, Some("Mailstronaut")),
            KeyBinding::new("ctrl-l", FocusMessages, Some("Mailstronaut")),
            KeyBinding::new("ctrl-,", ShowAccounts, Some("Mailstronaut")),
            KeyBinding::new("ctrl-1", ShowInbox, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-r", ReloadConfiguration, Some("Mailstronaut")),
            KeyBinding::new("ctrl-shift-e", ExportConfiguration, Some("Mailstronaut")),
            KeyBinding::new("ctrl-n", ComposeMessage, Some("Mailstronaut")),
            KeyBinding::new("ctrl-2", ShowDrafts, Some("Mailstronaut")),
            KeyBinding::new("ctrl-3", ShowReminders, Some("Mailstronaut")),
            KeyBinding::new("alt-r", Reply, Some("Mailstronaut")),
            KeyBinding::new("alt-a", ReplyAll, Some("Mailstronaut")),
            KeyBinding::new("alt-f", Forward, Some("Mailstronaut")),
            KeyBinding::new("alt-u", ToggleRead, Some("Mailstronaut")),
            KeyBinding::new("alt-s", ToggleStar, Some("Mailstronaut")),
            KeyBinding::new("alt-e", Archive, Some("Mailstronaut")),
            KeyBinding::new("alt-d", Trash, Some("Mailstronaut")),
        ]);
    }

    fn show_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_reader_requests();
        self.view = View::Accounts;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn show_compose(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.cancel_reader_requests();
        self.view = View::Compose;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer
            .update(cx, |composer, cx| composer.refresh_lists(window, cx));
        cx.notify();
    }

    fn show_reminders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_reader_requests();
        self.view = View::Reminders;
        self.read_gate.visibility(false);
        self.html_reader
            .update(cx, crate::html_reader::HtmlReader::hide);
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Route a notification click after configuration and the store are ready.
    pub(crate) fn open_notification(
        &mut self,
        target: PendingOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading_config || self.declaration.active().is_none() || self.handle().is_none() {
            self.pending_open = Some(target);
            return;
        }
        window.activate_window();
        match target {
            PendingOpen::Mail(click) => {
                if !self.accounts.contains(&click.account) {
                    return;
                }
                match click.message {
                    Some(mail) => self.open_new_mail(&click.account, &mail, window, cx),
                    None => self.show_inbox(window, cx),
                }
            }
            PendingOpen::Reminder(record) => {
                let Some(handle) = self.handle() else {
                    return;
                };
                let revision = self.declaration.revision();
                let accounts = self.accounts.clone();
                let job = self.runtime.spawn(async move {
                    crate::reminders::resolve_original(handle, accounts, record).await
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = job.await;
                    _ = this.update_in(cx, |this, window, cx| {
                        if revision != this.declaration.revision() {
                            return;
                        }
                        if let Ok(Ok(Some(record))) = result {
                            this.open_reminder(&record, window, cx);
                        } else {
                            this.status = "This reminder changed or is unavailable.".into();
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// A click on a new-mail notification opens the message it announced, like any explicit open.
    fn open_new_mail(
        &mut self,
        account: &str,
        mail: &mailstronaut_core::store::analysis::NewMail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Scope::Inbox, window, cx);
        self.open(
            Row {
                account: account.to_owned(),
                id: mail.id,
                conversation: None,
                sender: mail.sender.clone(),
                subject: mail.subject.clone(),
                preview: String::new(),
                unread: 0,
                messages: 1,
                received_at: mail.received_at,
                seen: false,
                starred: false,
                due: None,
                expires_at: None,
            },
            window,
            cx,
        );
    }

    /// A reminder names one exact original; opening it is an explicit open like any other.
    fn open_reminder(
        &mut self,
        record: &mailstronaut_core::store::reminders::Reminder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(Scope::Inbox, window, cx);
        self.open(
            Row {
                account: record.account.clone(),
                id: record.message,
                conversation: None,
                sender: record.account.clone(),
                subject: record.title.clone(),
                preview: String::new(),
                unread: 0,
                messages: 1,
                received_at: record.due_at,
                seen: false,
                starred: false,
                due: None,
                expires_at: None,
            },
            window,
            cx,
        );
    }

    fn remind_open(&self, when: &When, window: &mut Window, cx: &mut Context<Self>) {
        let Some((account, id)) = self
            .selected
            .as_ref()
            .map(|row| (row.account.clone(), row.id))
        else {
            return;
        };
        self.reminders.update(cx, |reminders, cx| {
            reminders.schedule(account, id, when, window, cx);
        });
    }

    fn compose(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.accounts.is_empty() || !self.composer.read(cx).has_store() {
            self.status = "Add an account before composing a message.".into();
            cx.notify();
            return;
        }
        self.show_compose(window, cx);
        self.composer
            .update(cx, |composer, cx| composer.compose_new(window, cx));
    }

    fn respond(&mut self, kind: Kind, window: &Window, cx: &mut Context<Self>) {
        let (Some(row), Some(message), Some(config)) = (
            self.selected.as_ref(),
            self.opened_message.as_ref(),
            self.declaration.active(),
        ) else {
            return;
        };
        let Some(account) = config.accounts.iter().find(|item| item.id == row.account) else {
            return;
        };
        match mailstronaut_core::reply::prepare(account, &config.accounts, message, kind) {
            Ok(draft) => {
                self.show_compose(window, cx);
                self.composer
                    .update(cx, |composer, cx| composer.open_prepared(draft, window, cx));
            }
            Err(error) => {
                self.reader_notice = error.to_string();
                cx.notify();
            }
        }
    }

    fn show_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Scope::Inbox, window, cx);
    }

    fn navigate(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.browse = crate::mailbox::Browse::default();
            self.clear_page(window, cx);
        }
        self.view = View::Inbox;
        self.composer.update(cx, |composer, cx| composer.flush(cx));
        self.read_gate.visibility(window.is_window_active());
        window.focus(&self.focus, cx);
        self.refresh(window, cx);
        if self.reader_mode == ReaderMode::Html {
            self.present_html(cx);
        }
        cx.notify();
    }

    fn clear_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retire_open(window, cx);
        self.list.update(cx, |list, cx| {
            crate::mailbox::replace(list, Vec::new(), window, cx);
            list.scroll_to_item(IndexPath::new(0), gpui::ScrollStrategy::Top, window, cx);
        });
    }

    fn load_more_rows(&mut self, window: &Window, cx: &Context<Self>) {
        let Some(handle) = self.handle() else {
            return;
        };
        if !self.browse.more || self.browse.pending || self.list.read(cx).delegate().loading {
            return;
        }
        self.browse.pending = true;
        let (generation, revision) = (self.query_generation, self.declaration.revision());
        let after = self.browse.after;
        let (accounts, scope, query, fuzzy) = (
            self.accounts.clone(),
            self.scope.clone(),
            self.query.clone(),
            self.fuzzy.clone(),
        );
        let cancel = self.query_cancel.clone();
        let filter = self.filter.clone();
        let meaning = self
            .meaning
            .clone()
            .filter(|_| self.search_mode == SearchMode::Related);
        let job = self.runtime.spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => Err("Local query cancelled.".to_owned()),
                result = conversation_rows(&handle, fuzzy.as_ref(), meaning.as_ref(), accounts, scope, query, after, filter) =>
                    result,
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, _, cx| {
                if generation != this.query_generation || revision != this.declaration.revision() {
                    return;
                }
                this.browse.pending = false;
                match result {
                    Ok(Ok((rows, _))) => {
                        this.browse.more = rows.len() == 100;
                        if let Some(last) = rows.last() {
                            this.browse.after = Some(Cursor {
                                received_at: last.latest.received_at,
                                message_id: last.latest.id,
                            });
                        }
                        this.browse.loaded += rows.len();
                        let rows = rows.into_iter().map(Row::from).collect::<Vec<_>>();
                        this.list.update(cx, |list, cx| {
                            let delegate = list.delegate_mut();
                            delegate.append(rows);
                            delegate.more = this.browse.more;
                            cx.notify();
                        });
                        this.status = this.page_status(this.browse.loaded);
                    }
                    error => {
                        this.list.update(cx, |list, cx| {
                            list.delegate_mut().more = false;
                            cx.notify();
                        });
                        this.status = match error {
                            Ok(Err(error)) => error,
                            _ => "Local query interrupted. Refresh to retry.".into(),
                        };
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Declarative Kit settings composition delegates all field behavior to existing preference components."
    )]
    fn account_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let loading = self.loading_config;
        let reload_disabled = loading || self.demo || self.preferences.read(cx).busy();
        let workspace = cx.entity().downgrade();
        let export_workspace = workspace.clone();
        let export_disabled = self.declaration.active().is_none() || loading;
        let config = self.declaration.active();
        let ownership = self.preferences.read(cx).ownership_label();
        let source_details = self.preferences.clone();
        let general = SettingPage::new("Configuration")
            .description("Preferences use your native Nix source. Managed and user-authored expressions stay read-only here.")
            .resettable(false)
            .group(
                SettingGroup::new()
                    .title("Native Nix")
                    .description(ownership)
                    .item(SettingItem::render(move |_, _, cx| crate::preferences::Preferences::source_details(&source_details, cx))
                        .keywords(["source", "path", "Nix", "configuration", "Home Manager"]))
                    .item(SettingItem::render(move |options, _, _| {
                        let workspace = workspace.clone();
                        Button::new("reload-config")
                            .label(if loading { "Loading…" } else { "Reload configuration" })
                            .disabled(options.is_disabled() || reload_disabled)
                            .on_click(move |_, window, cx| {
                                _ = workspace.update(cx, |this, cx| this.load_configuration(window, cx));
                            })
                    }).keywords(["reload", "configuration", "Nix", "Home Manager"]))
                    .item(SettingItem::render(move |options, _, _| {
                        let workspace = export_workspace.clone();
                        Button::new("export-nix").label("Export to Nix").tooltip("Ctrl+Shift+E").disabled(options.is_disabled() || export_disabled)
                            .on_click(move |_, window, cx| {
                                _ = workspace.update(cx, |this, cx| this.export_configuration(window, cx));
                            })
                    }).keywords(["export", "Nix", "Home Manager", "snapshot"])),
            );
        let general = if config.is_some() {
            general
                .group(crate::preferences::Preferences::appearance(
                    &self.preferences,
                    cx,
                ))
                .group(crate::preferences::Preferences::sync(&self.preferences, cx))
                .group(crate::preferences::Preferences::notifications(
                    &self.preferences,
                    cx,
                ))
                .group(crate::preferences::Preferences::search(
                    &self.preferences,
                    cx,
                    self.meaning
                        .as_ref()
                        .map(crate::mail_semantic::Meaning::state),
                ))
                .group(crate::preferences::Preferences::priority(
                    &self.preferences,
                    cx,
                ))
                .group(crate::preferences::Preferences::times(
                    &self.preferences,
                    cx,
                ))
        } else {
            general.group(crate::preferences::Preferences::setup(
                &self.preferences,
                cx,
            ))
        };
        let general = general.group(SettingGroup::new().title("Account status").item({
            let status = self.declaration.error().map_or_else(
                || {
                    if self.account_views.is_empty() {
                        "No accounts are active. Add an account here or in your Nix declaration."
                            .to_owned()
                    } else {
                        "Choose an account in Settings to sign in or sync mail.".to_owned()
                    }
                },
                |error| {
                    if config.is_some() {
                        format!("{error}. The last valid configuration remains active.")
                    } else {
                        format!("{error}. No accounts have been started.")
                    }
                },
            );
            SettingItem::render(move |_, _, _| div().text_sm().child(status.clone()))
                .keywords(["accounts", "status", "errors", "reload"])
        }));
        let editor = self.preferences.read(cx);
        let settings = Settings::new(("mail-settings", self.declaration.revision()))
            .default_selected_index(editor.page_after_reload())
            .sidebar_width(px(220.))
            .page(general)
            .pages(
                self.account_views
                    .iter()
                    .map(|view| crate::accounts::AccountPane::page(view, cx)),
            )
            .page(crate::onboarding::Onboarding::page(&self.onboarding));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(settings))
            .child(crate::preferences::Preferences::footer(
                &self.preferences,
                cx,
            ))
    }

    fn export_configuration(&self, window: &Window, cx: &Context<Self>) {
        let Some(config) = self.declaration.active().cloned() else {
            return;
        };
        let revision = self.declaration.revision();
        let job = self
            .runtime
            .spawn_blocking(move || mailstronaut_config::native::export(&config));
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if revision != this.declaration.revision() || this.view != View::Accounts {
                    return;
                }
                match result {
                    Ok(Ok(text)) => {
                        crate::config_export::Export::show(text, this.runtime.clone(), window, cx);
                    }
                    _ => window.push_notification(
                        Notification::error("Unable to export this configuration.")
                            .delivery(NotificationDelivery::InApp),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    /// Unread mail worth a badge: mail in Sent, Drafts, Trash or Junk asks for nothing.
    fn unread_badge(&self, folder: &Folder) -> usize {
        match folder.role {
            Some(FolderRole::Sent | FolderRole::Drafts | FolderRole::Trash | FolderRole::Junk) => 0,
            _ => self.unread.get(&folder.id).copied().unwrap_or_default(),
        }
    }

    fn count_badge(count: usize) -> impl IntoElement {
        // Badge intentionally overhangs its child. Reserve that space inside
        // the sidebar suffix so 99+ is never clipped at the menu's right edge.
        div()
            .w_9()
            .h_5()
            .flex_shrink_0()
            .pr_3()
            .pt_1()
            .child(Badge::new().count(count).child(div().w_6().h_3()))
    }

    fn mail_routes(&self, attention: usize, cx: &Context<Self>) -> SidebarMenu {
        let overdue = self.reminders.read(cx).overdue();
        let unread: usize = self
            .folders
            .iter()
            .filter(|folder| folder.inbox)
            .map(|folder| self.unread_badge(folder))
            .sum();
        SidebarMenu::new()
            .child(
                SidebarMenuItem::new("Unified inbox")
                    .icon(IconName::Inbox)
                    .active(self.view == View::Inbox && self.scope == Scope::Inbox)
                    .suffix(move |_, _| Self::count_badge(unread))
                    .on_click(cx.listener(|this, _, window, cx| this.show_inbox(window, cx))),
            )
            .child(
                SidebarMenuItem::new("Drafts & outbox")
                    .icon(IconName::Folder)
                    .active(self.view == View::Compose)
                    .suffix(move |_, _| Self::count_badge(attention))
                    .on_click(cx.listener(|this, _, window, cx| this.show_compose(window, cx))),
            )
            .child(
                SidebarMenuItem::new("Reminders")
                    .icon(IconName::Calendar)
                    .active(self.view == View::Reminders)
                    .suffix(move |_, _| Self::count_badge(overdue))
                    .on_click(cx.listener(|this, _, window, cx| this.show_reminders(window, cx))),
            )
    }

    fn account_label(&self, account: &mailstronaut_config::Account) -> String {
        let duplicate = self.declaration.active().is_some_and(|config| {
            config
                .accounts
                .iter()
                .filter(|other| other.display_name == account.display_name)
                .count()
                > 1
        });
        if duplicate {
            format!("{} · {}", account.id, account.display_name)
        } else {
            account.display_name.clone()
        }
    }

    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let attention = self.composer.read(cx).attention();
        Sidebar::new("mail-sidebar")
            .w_full()
            .collapsible(false)
            .child(SidebarGroup::new("Mail").child(self.mail_routes(attention, cx)))
            .child(
                SidebarGroup::new("Accounts").child(
                    SidebarMenu::new().children(
                        self.declaration
                            .active()
                            .into_iter()
                            .flat_map(|config| &config.accounts)
                            .map(|account| {
                                let scope = Scope::Account(account.id.clone());
                                SidebarMenuItem::new(self.account_label(account))
                                    .icon(IconName::Inbox)
                                    .accessibility_label(format!(
                                        "All cached mail · {} · {}",
                                        account.display_name, account.id
                                    ))
                                    .active(self.view == View::Inbox && self.scope == scope)
                                    .default_open(true)
                                    .click_to_open(true)
                                    .click_to_toggle(false)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate(scope.clone(), window, cx);
                                    }))
                                    .children(
                                        self.folders
                                            .iter()
                                            .filter(|folder| folder.account == account.id)
                                            .map(|folder| {
                                                let scope = Scope::Folder {
                                                    account: account.id.clone(),
                                                    folder: folder.id,
                                                };
                                                let wire = folder.name.clone();
                                                let unread = self.unread_badge(folder);
                                                SidebarMenuItem::new(folder.label())
                                                    .icon(if folder.inbox {
                                                        IconName::Inbox
                                                    } else {
                                                        IconName::Folder
                                                    })
                                                    .suffix(move |_, _| Self::count_badge(unread))
                                                    .accessibility_label(format!(
                                                        "Cached folder · {} · {} · {unread} unread",
                                                        account.display_name,
                                                        folder.label()
                                                    ))
                                                    .active(
                                                        self.view == View::Inbox
                                                            && self.scope == scope,
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.navigate(
                                                                scope.clone(),
                                                                window,
                                                                cx,
                                                            );
                                                            // Standard folders already sync;
                                                            // this covers any other folder.
                                                            if let Scope::Folder {
                                                                account, ..
                                                            } = &scope
                                                            {
                                                                this.sync_session
                                                                    .read(cx)
                                                                    .include_folder(
                                                                        account,
                                                                        wire.clone(),
                                                                    );
                                                            }
                                                        },
                                                    ))
                                            }),
                                    )
                            }),
                    ),
                ),
            )
            .footer(
                div().w_full().p_2().child(
                    SidebarMenuItem::new("Settings")
                        .icon(IconName::Settings)
                        .active(self.view == View::Accounts)
                        .on_click(cx.listener(|this, _, window, cx| this.show_accounts(window, cx)))
                        .render("settings-navigation", window, cx),
                ),
            )
    }

    fn mailbox(&self, cx: &Context<Self>) -> impl IntoElement {
        if let Some(error) = &self.startup_error {
            return Empty::new()
                .size_full()
                .header(
                    EmptyHeader::new()
                        .title(EmptyTitle::new().child("Local mail unavailable"))
                        .description(EmptyDescription::new().child(error.clone())),
                )
                .content(
                    EmptyContent::new().child(
                        Button::new("retry-mail-store")
                            .label("Retry")
                            .accessibility_label("Retry opening local mail")
                            .loading(self.loading_config)
                            .disabled(self.loading_config || self.preferences.read(cx).busy())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_configuration(window, cx);
                            })),
                    ),
                )
                .into_any_element();
        }
        h_resizable("mail-panes")
            .child(
                resizable_panel()
                    .size(px(360.))
                    .size_range(px(280.)..px(600.))
                    .child(self.message_list(cx)),
            )
            .child(resizable_panel().child(self.reader(cx)))
            .into_any_element()
    }

    fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        div()
            .flex()
            .items_center()
            .gap_4()
            .p_4()
            .border_b_1()
            .border_color(border)
            .child(div().flex_1().when(self.view == View::Inbox, |view| {
                let filters = self.filters.clone();
                view.flex()
                    .items_center()
                    .gap_2()
                    .child(Input::new(&self.search))
                    .when(self.meaning.is_some(), |view| view.child(
                        Button::new("related-mail")
                            .icon(AssetIcon::Sparkles)
                            .label("Related")
                            .selected(self.search_mode == SearchMode::Related && related_intent(&self.query))
                            .disabled(!related_intent(&self.query) || !self.meaning.as_ref().is_some_and(|engine| matches!(engine.state(), crate::mail_semantic::State::Ready { .. })))
                            .accessibility_label("Find related mail by meaning")
                            .tooltip("Related mail across languages. Suggestions may include unrelated mail; quotes and field filters stay literal.")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.search_mode = if this.search_mode == SearchMode::Related { SearchMode::Words } else { SearchMode::Related };
                                this.browse = crate::mailbox::Browse::default();
                                this.clear_page(window, cx);
                                this.refresh_rows(window, cx);
                            })),
                    ))
                    .child(
                        Popover::new("mail-search-filters")
                            .trigger(
                                Button::new("search-filters")
                                    .icon(AssetIcon::ListFilter)
                                    .label(if self.filter.is_empty() {
                                        "Filters"
                                    } else {
                                        "Filters active"
                                    })
                                    .accessibility_label("Search filters"),
                            )
                            .content(move |_, _, _| filters.clone()),
                    )
            }))
            .child(
                Button::new("compose")
                    .primary()
                    .icon(AssetIcon::SquarePen)
                    .accessibility_label("Compose")
                    .tooltip("Compose · Ctrl+N")
                    .disabled(self.accounts.is_empty() || !self.composer.read(cx).has_store())
                    .on_click(cx.listener(|this, _, window, cx| this.compose(window, cx))),
            )
            .when(self.view == View::Inbox, |view| {
                view.child(
                    Button::new("refresh")
                        .icon(AssetIcon::RefreshCw)
                        .accessibility_label("Sync mail or refresh cache")
                        .tooltip("Sync mail / refresh cache · Ctrl+R")
                        .loading(!self.syncing.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| this.sync_now(window, cx))),
                )
            })
            .child(
                Button::new("appearance")
                    .ghost()
                    .icon(AssetIcon::SunMoon)
                    .accessibility_label("Toggle light and dark appearance")
                    .tooltip("Toggle appearance · Ctrl+Shift+D")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_appearance(window, cx);
                    })),
            )
    }

    fn message_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let loading = self.list.read(cx).delegate().loading;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.sync_alert(cx))
            .child(self.permanent_move_alert(cx))
            .when(
                self.search_mode == SearchMode::Related && related_intent(&self.query),
                |view| {
                    view.child(Alert::info(
                        "related-mail-notice",
                        "Related mail · Suggestions by meaning, not exact word matches.",
                    ))
                },
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(List::new(&self.list).size_full()),
            )
            .when(!self.browse.more && self.can_fetch_older(), |view| {
                view.child(
                    Button::new("fetch-older-mail")
                        .ghost()
                        .label("Fetch older mail")
                        .loading(self.older_fetch.is_some())
                        .disabled(loading || self.older_fetch.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.fetch_older(window, cx))),
                )
            })
    }

    /// Where older mail for the current view would come from, leaving out folders already
    /// fetched back to their beginning.
    fn older_targets(&self) -> Vec<(Account, String, bool)> {
        let Some(config) = self.declaration.active() else {
            return Vec::new();
        };
        let inbox_of = |account: &Account| {
            let folder = account
                .folders
                .inbox
                .clone()
                .unwrap_or_else(|| "INBOX".into());
            (account.clone(), folder, true)
        };
        let targets: Vec<_> = match &self.scope {
            Scope::Inbox => config.accounts.iter().map(inbox_of).collect(),
            Scope::Account(id) => config
                .accounts
                .iter()
                .filter(|account| &account.id == id)
                .map(inbox_of)
                .collect(),
            Scope::Folder { account, folder } => config
                .accounts
                .iter()
                .filter(|item| &item.id == account)
                .filter_map(|item| {
                    let found = self.folders.iter().find(|row| &row.id == folder)?;
                    Some((item.clone(), found.name.clone(), found.inbox))
                })
                .collect(),
        };
        targets
            .into_iter()
            .filter(|(account, folder, _)| {
                !matches!(
                    self.older_reached
                        .get(&(account.id.clone(), folder.clone())),
                    Some(None)
                )
            })
            .collect()
    }

    fn can_fetch_older(&self) -> bool {
        self.view == View::Inbox
            && self.query.trim().is_empty()
            && self.filter.is_empty()
            && self.handle().is_some()
            && !self.older_targets().is_empty()
    }

    fn fetch_older(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(store) = self.handle() else {
            return;
        };
        if self.older_fetch.is_some() {
            return;
        }
        let revision = self.declaration.revision();
        self.older_fetch = Some(revision);
        let cancel = self.account_lifetime.child_token();
        let targets = self.older_targets();
        let reached = self.older_reached.clone();
        let access = self.account_access.clone();
        let job = self.runtime.spawn(async move {
            let mut progress = Vec::new();
            let mut downloaded = 0;
            for (account, folder, inbox) in targets {
                let key = (account.id.clone(), folder.clone());
                let resume = reached.get(&key).copied().flatten();
                match access
                    .fetch_older(&store, &account, &folder, inbox, resume, &cancel)
                    .await
                {
                    Ok(older) => {
                        downloaded += older.downloaded;
                        progress.push((key, older.resume));
                    }
                    Err(error) => return (progress, downloaded, Some(error)),
                }
            }
            (progress, downloaded, None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.older_fetch != Some(revision) {
                    return;
                }
                this.older_fetch = None;
                let notice = match result {
                    Ok((progress, downloaded, failure)) => {
                        this.older_reached.extend(progress);
                        if let Some(search) = &this.fuzzy {
                            search.wake();
                        }
                        if let Some(meaning) = &this.meaning {
                            meaning.wake();
                        }
                        if let Some(priority) = &this.priority {
                            priority.wake();
                        }
                        failure.unwrap_or_else(|| match downloaded {
                            0 => "There is no older mail on the server.".to_owned(),
                            1 => "Fetched 1 older message.".to_owned(),
                            count => format!("Fetched {count} older messages."),
                        })
                    }
                    Err(_) => "Fetching older mail was interrupted.".to_owned(),
                };
                window.push_notification(
                    Notification::new()
                        .message(notice)
                        .delivery(NotificationDelivery::InApp),
                    cx,
                );
                this.refresh_history(cx);
                this.refresh_rows(window, cx);
            });
        })
        .detach();
        cx.notify();
    }
    fn reader(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .id("reader-scroll")
            .size_full()
            .flex()
            .flex_col()
            .p_8()
            .child(self.selected.as_ref().map_or_else(
                || {
                    div().child(div().text_2xl().child("Room to focus.")).child(
                        div()
                            .mt_3()
                            .text_color(muted)
                            .child("Open a message to read it here."),
                    )
                },
                |row| {
                    div()
                        .mb_6()
                        .child(div().text_2xl().font_semibold().child(row.subject.clone()))
                        .child(
                            div()
                                .mt_2()
                                .text_sm()
                                .text_color(muted)
                                .child(format!("{} · {}", row.sender, row.account)),
                        )
                },
            ))
            .when_some(self.selected.as_ref(), |view, _| {
                view.child(self.message_actions(cx))
                    .child(self.thread_reader.clone())
                    .child(self.reader_tools(cx))
            })
            .when_some(self.opened_message.as_ref(), |view, message| {
                view.child(self.message_details(message, cx))
                    .child(Self::received_attachments(message, cx))
                    .child(self.remote_attachments(cx))
                    .child(self.analysis_notes(cx))
            })
            .when(!self.reader_notice.is_empty(), |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(self.reader_notice.clone()),
                )
            })
            .when(
                self.selected.is_some()
                    && self.reader_mode == ReaderMode::Text
                    && self.body.is_empty(),
                |view| {
                    view.child(
                        div().flex_1().min_h_0().child(
                            Textarea::new(&self.text_reader)
                                .readonly(true)
                                .appearance(false)
                                .bordered(false)
                                .aria_label("Message text")
                                .accessibility_id("message-text")
                                .h_full()
                                .w_full(),
                        ),
                    )
                },
            )
            .when(
                self.selected.is_some()
                    && self.reader_mode == ReaderMode::Html
                    && self.body.is_empty(),
                |view| view.child(div().flex_1().min_h_0().child(self.html_reader.clone())),
            )
            .when(!self.body.is_empty(), |view| view.child(self.body.clone()))
    }

    fn received_attachments(
        message: &mailstronaut_core::mime::Projection,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let listed = message.listed_attachments();
        div().when(!listed.is_empty(), |view| {
            view.mb_3()
                .child(AttachmentGroup::new("received-attachments").children(
                    listed.into_iter().map(|(index, part)| {
                        let name = part.name.clone().unwrap_or_else(|| "Attachment".into());
                        let kind = part.media_type.clone().unwrap_or_default();
                        Attachment::new()
                            .media(AttachmentMedia::new().child(AssetIcon::FileText))
                            .id(("received-attachment", index))
                            .w_full()
                            .actions(Self::attachment_actions(index, false, &name, cx))
                            .content(
                                AttachmentContent::new()
                                    .title(AttachmentTitle::new(name))
                                    .description(AttachmentDescription::new(format!(
                                        "{} · {kind}",
                                        crate::composer::size_label(part.decoded_size)
                                    ))),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.save_attachment(index, window, cx);
                            }))
                    }),
                ))
        })
    }

    /// Why this message was flagged, and each date found with the words it came from. Mail
    /// is untrusted, so the quote is shown as plain text and only ever as a quote.
    fn analysis_notes(&self, cx: &Context<Self>) -> impl IntoElement {
        let notes: Vec<String> = self
            .opened_analysis
            .as_ref()
            .map_or_else(Vec::new, |analysis| {
                if self.corrected_deadline.is_some() {
                    return Vec::new();
                }
                crate::mail_priority::explanation(analysis)
                    .into_iter()
                    .chain(crate::mail_priority::deadline_notes(analysis))
                    .collect()
            });
        let muted = cx.theme().muted_foreground;
        div()
            .when(!notes.is_empty(), |view| {
                view.mb_3()
                    .children(notes.into_iter().enumerate().map(|(index, note)| {
                        Label::new(note)
                            .id(("analysis-note", index))
                            .text_sm()
                            .text_color(muted)
                    }))
            })
            .child(self.deadline_controls(cx))
    }

    /// Attachments of a message too large to keep are listed like any others, and
    /// downloaded from the server only when saved.
    fn remote_attachments(&self, cx: &Context<Self>) -> impl IntoElement {
        let omitted = self.opened_omitted.as_ref();
        div().when_some(omitted, |view, omitted| {
            let muted = cx.theme().muted_foreground;
            view.mb_3()
                .child(
                    Label::new(format!(
                        "This message is {}, too large to keep on this device. Its attachments stay on the server until you open or save them.",
                        crate::composer::size_label(omitted.size as usize)
                    ))
                    .text_sm()
                    .text_color(muted)
                    .mb_2(),
                )
                .when(!omitted.attachments.is_empty(), |view| {
                    view.child(AttachmentGroup::new("remote-attachments").children(
                        omitted.attachments.iter().enumerate().map(|(index, part)| {
                            let name = part.name.clone().unwrap_or_else(|| "Attachment".into());
                            let size = usize::try_from(part.size).unwrap_or(usize::MAX);
                            Attachment::new()
                            .media(AttachmentMedia::new().child(AssetIcon::FileText))
                                .id(("remote-attachment", index))
                                .w_full()
                                .actions(Self::attachment_actions(index, true, &name, cx))
                                .content(
                                    AttachmentContent::new()
                                        .title(AttachmentTitle::new(name))
                                        .description(AttachmentDescription::new(format!(
                                            "On the server · about {}",
                                            crate::composer::size_label(size)
                                        ))),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save_remote_attachment(index, window, cx);
                                }))
                        }),
                    ))
                })
        })
    }

    /// The chooser comes first, so that nothing is downloaded for a save that is cancelled.
    fn save_remote_attachment(&self, index: usize, window: &Window, cx: &Context<Self>) {
        let (Some(row), Some(part), Some(store)) = (
            self.selected.clone(),
            self.opened_omitted
                .as_ref()
                .and_then(|omitted| omitted.attachments.get(index))
                .cloned(),
            self.handle(),
        ) else {
            return;
        };
        let Some(account) = self
            .declaration
            .active()
            .and_then(|config| config.accounts.iter().find(|item| item.id == row.account))
            .cloned()
        else {
            return;
        };
        let suggested = part
            .name
            .as_deref()
            .filter(|name| mailstronaut_mail::compose::valid_attachment_name(name))
            .unwrap_or("attachment")
            .to_owned();
        let directory = directories::UserDirs::new()
            .and_then(|dirs| {
                dirs.download_dir()
                    .map(std::path::Path::to_path_buf)
                    .or_else(|| Some(dirs.home_dir().to_path_buf()))
            })
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        let (runtime, access, cancel) = (
            self.runtime.clone(),
            self.account_access.clone(),
            self.account_lifetime.child_token(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let Some(choice) = cancel.run_until_cancelled(chosen).await else {
                return;
            };
            let notice = match choice {
                Ok(Ok(Some(path))) => {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            Notification::new()
                                .id::<AttachmentDownload>()
                                .message("Downloading the attachment from the server…")
                                .autohide(false)
                                .delivery(NotificationDelivery::InApp),
                            cx,
                        );
                    });
                    let download = cancel.clone();
                    let saved = runtime
                        .spawn(async move {
                            let bytes = access
                                .fetch_attachment(
                                    &store,
                                    &account,
                                    row.id,
                                    &part.section,
                                    &download,
                                )
                                .await?;
                            // Decoding already-fetched bytes can finish after the account retired.
                            if download.is_cancelled() {
                                return Err("Attachment download cancelled.".to_owned());
                            }
                            tokio::fs::write(path, bytes).await.map_err(|_| {
                                "The attachment could not be written there.".to_owned()
                            })
                        })
                        .await;
                    Some(match saved {
                        Ok(Ok(())) => "Attachment saved.".to_owned(),
                        Ok(Err(error)) => error,
                        Err(_) => "Saving the attachment was interrupted.".to_owned(),
                    })
                }
                Ok(Ok(None)) => None,
                _ => Some("The file chooser is unavailable. Nothing was saved.".to_owned()),
            };
            if !cancel.is_cancelled()
                && let Some(notice) = notice
            {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::new()
                            .id::<AttachmentDownload>()
                            .message(notice)
                            .delivery(NotificationDelivery::InApp),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// Saving is explicit and goes through the portal's chooser. Attachments are never
    /// opened automatically, and a sender-supplied name is only a suggestion.
    fn save_attachment(&self, index: usize, window: &Window, cx: &Context<Self>) {
        let Some(message) = self.opened_message.clone() else {
            return;
        };
        let suggested = message
            .attachments
            .get(index)
            .and_then(|part| part.name.as_deref())
            .filter(|name| mailstronaut_mail::compose::valid_attachment_name(name))
            .unwrap_or("attachment")
            .to_owned();
        let directory = directories::UserDirs::new()
            .and_then(|dirs| {
                dirs.download_dir()
                    .map(std::path::Path::to_path_buf)
                    .or_else(|| Some(dirs.home_dir().to_path_buf()))
            })
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let notice = match chosen.await {
                Ok(Ok(Some(path))) => {
                    let saved = runtime
                        .spawn_blocking(move || {
                            let bytes = message
                                .attachment_bytes(index)
                                .map_err(|_| "This attachment could not be decoded.")?;
                            std::fs::write(path, bytes)
                                .map_err(|_| "The attachment could not be written there.")
                        })
                        .await;
                    match saved {
                        Ok(Ok(())) => Some("Attachment saved.".to_owned()),
                        Ok(Err(error)) => Some(error.to_owned()),
                        Err(_) => Some("Saving the attachment was interrupted.".to_owned()),
                    }
                }
                Ok(Ok(None)) => None,
                _ => Some("The file chooser is unavailable. Nothing was saved.".to_owned()),
            };
            if let Some(notice) = notice {
                _ = this.update(cx, |this, cx| {
                    this.reader_notice = notice;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn message_details(
        &self,
        message: &mailstronaut_core::mime::Projection,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let headers = &message.headers;
        let zone = &self.list.read(cx).delegate().time_zone;
        gpui::component::collapsible::Collapsible::new()
            .open(self.headers_open)
            .content(
                div()
                    .id("message-details-scroll")
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .mb_3()
                    .child(
                        DescriptionList::new()
                            .columns(1)
                            .bordered(false)
                            .item("From", crate::mailbox::addresses(&headers.from), 1)
                            .item("To", crate::mailbox::addresses(&headers.to), 1)
                            .item("Cc", crate::mailbox::addresses(&headers.cc), 1)
                            .item(
                                "Sent",
                                headers.date.map_or_else(
                                    || "Date unavailable".into(),
                                    |date| crate::mailbox::date(date, zone),
                                ),
                                1,
                            )
                            .item(
                                "Received",
                                self.selected.as_ref().map_or_else(
                                    || "Date unavailable".into(),
                                    |row| crate::mailbox::date(row.received_at, zone),
                                ),
                                1,
                            ),
                    ),
            )
    }

    fn message_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let unavailable = self.opened_message.is_none() || self.demo;
        let (seen, starred) = self
            .selected
            .as_ref()
            .map_or((false, false), |row| (row.seen, row.starred));
        Toolbar::new("message-actions")
            .mb_3()
            .content(
                ToolbarGroup::new("message-respond")
                    .label("Respond")
                    .child(
                        Button::new("reply")
                            .outline()
                            .label("Reply")
                            .tooltip("Alt+R")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::Reply, window, cx);
                            })),
                    )
                    .child(
                        Button::new("reply-all")
                            .ghost()
                            .label("Reply all")
                            .tooltip("Alt+A")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::ReplyAll, window, cx);
                            })),
                    )
                    .child(
                        Button::new("forward")
                            .ghost()
                            .label("Forward")
                            .tooltip("Alt+F · plain text, without attachments")
                            .disabled(unavailable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.respond(Kind::ForwardText, window, cx);
                            })),
                    ),
            )
            .content(self.move_controls(cx))
            .content(self.remind_control(cx))
            .content(
                ToolbarGroup::new("message-state")
                    .label("Message state")
                    .child(
                        Button::new("toggle-read")
                            .ghost()
                            .label(if seen { "Mark unread" } else { "Mark read" })
                            .tooltip("Alt+U")
                            .disabled(self.demo)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_flag(FlagChange::Seen, window, cx);
                            })),
                    )
                    .child(
                        Button::new("toggle-star")
                            .ghost()
                            .label(if starred { "Unstar" } else { "Star" })
                            .tooltip("Alt+S")
                            .disabled(self.demo)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_flag(FlagChange::Starred, window, cx);
                            })),
                    ),
            )
    }

    fn remind_control(&self, cx: &Context<Self>) -> impl IntoElement {
        let workspace = cx.entity().downgrade();
        let scheduled = self
            .selected
            .as_ref()
            .is_some_and(|row| self.reminders.read(cx).is_scheduled(&row.account, row.id));
        Button::new("remind-me")
            .ghost()
            .label(if scheduled {
                "Reminder set"
            } else {
                "Remind me"
            })
            .disabled(self.demo)
            .dropdown_menu(move |menu, _, _| {
                let preset = |menu: gpui::component::menu::PopupMenu,
                              label: &'static str,
                              when: When| {
                    let workspace = workspace.clone();
                    menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        _ = workspace.update(cx, |this, cx| this.remind_open(&when, window, cx));
                    }))
                };
                let menu = preset(menu, "In one hour", When::Hour);
                let menu = preset(menu, "This evening", When::Evening);
                let menu = preset(menu, "Tomorrow morning", When::Tomorrow);
                let workspace = workspace.clone();
                menu.item(
                    PopupMenuItem::new("Pick a time…").on_click(move |_, window, cx| {
                        _ = workspace.update(cx, |this, cx| {
                            if let Some(row) = &this.selected {
                                let (account, id) = (row.account.clone(), row.id);
                                this.reminders.update(cx, |reminders, cx| {
                                    reminders.ask_custom(account, id, window, cx);
                                });
                            }
                        });
                    }),
                )
            })
    }

    fn move_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        ToolbarGroup::new("message-move")
            .label("Move")
            .child(
                Button::new("archive")
                    .ghost()
                    .label("Archive")
                    .tooltip("Alt+E · the whole conversation")
                    .disabled(self.demo)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.move_to_role(FolderRole::Archive, window, cx);
                    })),
            )
            .child(
                Button::new("trash")
                    .ghost()
                    .label("Trash")
                    .tooltip("Alt+D · the whole conversation")
                    .disabled(self.demo)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.move_to_role(FolderRole::Trash, window, cx);
                    })),
            )
            .content(self.move_picker_control())
    }

    fn reader_tools(&self, cx: &Context<Self>) -> impl IntoElement {
        Toolbar::new("reader-tools")
            .mb_4()
            .content(
                TabBar::new("reader-mode")
                    .segmented()
                    .selected_index(usize::from(self.reader_mode == ReaderMode::Html))
                    .on_click(cx.listener(|this, index, window, cx| {
                        if *index == 0 {
                            this.focus_reader(window, cx);
                        } else {
                            this.present_html(cx);
                        }
                    }))
                    .child(Tab::new().label("Text"))
                    .child(Tab::new().label("HTML").disabled(!self.has_html())),
            )
            .content(
                ToolbarGroup::new("reader-text-actions")
                    .label("Message text")
                    .child(
                        Button::new("select-message-text")
                            .ghost()
                            .label("Select text")
                            .tooltip("Text selection and copy · Ctrl+Shift+T")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.focus_reader(window, cx);
                            })),
                    ),
            )
            .content(self.link_menu(cx))
            .content(
                Button::new("message-details")
                    .ghost()
                    .label("Details")
                    .selected(self.headers_open)
                    .disabled(self.opened_message.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.headers_open = !this.headers_open;
                        cx.notify();
                    })),
            )
    }

    fn link_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let links = self.html_reader.read(cx).links();
        let epoch = self.reading_generation;
        let workspace = cx.entity().downgrade();
        let restore_focus = workspace.clone();
        Button::new("reader-web-links")
            .ghost()
            .label("Links")
            .tooltip("Open in browser · first 128 web links · Ctrl+Shift+L")
            .track_focus(&self.link_focus)
            .disabled(
                links.is_empty()
                    || self.reader_mode != ReaderMode::Html
                    || self.link_cancel.is_some(),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for link in links.iter() {
                    let workspace = workspace.clone();
                    let target = link.clone();
                    menu = menu.item(PopupMenuItem::new(link.as_str().to_owned()).on_click(
                        move |_, window, cx| {
                            _ = workspace.update(cx, |this, cx| {
                                this.open_link(epoch, target.clone(), window, cx);
                            });
                        },
                    ));
                }
                menu
            })
            .on_open_change(move |open, window, cx| {
                if !open {
                    let workspace = restore_focus.clone();
                    // This pin restores a context-only trigger handle. Restore
                    // the Button's public focus handle after menu dismissal.
                    window.defer(cx, move |window, cx| {
                        _ = workspace.update(cx, |this, cx| {
                            if epoch == this.reading_generation
                                && this.view == View::Inbox
                                && this.reader_mode == ReaderMode::Html
                                && this.link_cancel.is_none()
                            {
                                window.focus(&this.link_focus, cx);
                            }
                        });
                    });
                }
            })
    }
}

impl Workspace {
    fn with_actions(root: gpui::Div, cx: &Context<Self>) -> gpui::Div {
        root.on_action(cx.listener(|this, _: &ComposeMessage, window, cx| {
            if this.composer.read(cx).has_store() {
                this.compose(window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowDrafts, window, cx| {
            this.show_compose(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ShowReminders, window, cx| {
            this.show_reminders(window, cx);
        }))
        .on_action(cx.listener(|this, _: &Reply, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::Reply, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ReplyAll, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::ReplyAll, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Forward, window, cx| {
            if this.view == View::Inbox {
                this.respond(Kind::ForwardText, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleRead, window, cx| {
            if this.view == View::Inbox {
                this.toggle_flag(FlagChange::Seen, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleStar, window, cx| {
            if this.view == View::Inbox {
                this.toggle_flag(FlagChange::Starred, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Archive, window, cx| {
            if this.view == View::Inbox {
                this.move_to_role(FolderRole::Archive, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Trash, window, cx| {
            if this.view == View::Inbox {
                this.move_to_role(FolderRole::Trash, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
            if this.view != View::Inbox {
                this.show_inbox(window, cx);
            }
            this.search.update(cx, |input, cx| input.focus(window, cx));
        }))
        .on_action(cx.listener(|this, _: &RefreshCache, window, cx| this.sync_now(window, cx)))
        .on_action(cx.listener(|this, _: &ToggleAppearance, window, cx| {
            this.toggle_appearance(window, cx);
        }))
        .on_action(cx.listener(|this, _: &FocusMessages, window, cx| {
            if this.view != View::Inbox {
                this.show_inbox(window, cx);
            }
            this.list.update(cx, |list, cx| list.focus(window, cx));
        }))
        .on_action(cx.listener(|this, _: &FocusReader, window, cx| {
            this.focus_reader(window, cx);
        }))
        .on_action(cx.listener(|this, _: &FocusLinks, window, cx| {
            if this.view == View::Inbox
                && this.reader_mode == ReaderMode::Html
                && this.link_cancel.is_none()
                && !this.html_reader.read(cx).links().is_empty()
            {
                window.focus(&this.link_focus, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &ShowAccounts, window, cx| this.show_accounts(window, cx)))
        .on_action(cx.listener(|this, _: &ShowInbox, window, cx| this.show_inbox(window, cx)))
        .on_action(cx.listener(|this, _: &ReloadConfiguration, window, cx| {
            this.load_configuration(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ExportConfiguration, window, cx| {
            if this.view == View::Accounts {
                this.export_configuration(window, cx);
            }
        }))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let foreground = cx.theme().foreground;
        Self::with_actions(div(), cx)
            .size_full()
            .key_context("Mailstronaut")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .child(gpui::component::TitleBar::new().child("Mailstronaut"))
            .child(self.top_bar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("workspace-panes")
                        .child(
                            resizable_panel()
                                .size(px(200.))
                                .size_range(px(160.)..px(300.))
                                .child(self.sidebar(window, cx)),
                        )
                        .child(
                            resizable_panel().child(
                                div()
                                    .size_full()
                                    .when(self.view == View::Accounts, |view| {
                                        view.child(self.account_settings(cx))
                                    })
                                    .when(self.view == View::Inbox, |view| {
                                        view.child(self.mailbox(cx))
                                    })
                                    .when(self.view == View::Compose, |view| {
                                        view.child(self.composer.clone())
                                    })
                                    .when(self.view == View::Reminders, |view| {
                                        view.child(self.reminders.clone())
                                    }),
                            ),
                        ),
                ),
            )
            // The composer reports its own draft state; mailbox counts would mislead there.
            .when(
                self.view != View::Compose
                    || !self.syncing.is_empty()
                    || !self.cancelled_fetches.is_empty(),
                |view| {
                    view.child(
                        StatusBar::new().px_4().py_2().child(
                            div()
                                .w_full()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_3()
                                .when(self.view != View::Compose, |row| {
                                    row.child(self.sync_status())
                                })
                                .when_some(
                                    self.fuzzy
                                        .as_ref()
                                        .map(crate::mail_search::MailSearch::queued)
                                        .filter(|count| *count > 0 && self.view == View::Inbox),
                                    |row, count| {
                                        row.child(Spinner::new()).child(format!(
                                            "Updating search index · {count} pending"
                                        ))
                                    },
                                )
                                .children(self.fetch_progress(cx))
                                .children(self.cancelled_fetches.iter().map(|account| {
                                    div().child(format!("{account} · fetch cancelled"))
                                })),
                        ),
                    )
                },
            )
    }
}

#[cfg(test)]
mod browse_tests {
    use super::*;
    use mailstronaut_core::store::mailbox::{Flags, Membership};
    use std::num::NonZeroU32;

    #[test]
    fn fetch_status_distinguishes_connecting_unknown_and_measured_progress() {
        let mut fetch = crate::mail_sync::Fetch {
            folder: "INBOX".into(),
            completed: 0,
            total: None,
            cancel: CancellationToken::new(),
        };
        assert_eq!(
            Workspace::format_fetch_progress("account", None),
            ("account · connecting…".into(), None)
        );
        assert_eq!(
            Workspace::format_fetch_progress("account", Some(&fetch)),
            ("account · INBOX: fetching…".into(), None)
        );
        fetch.completed = 7;
        assert_eq!(
            Workspace::format_fetch_progress("account", Some(&fetch)),
            ("account · INBOX: 7 reviewed".into(), None)
        );
        fetch.total = Some(10);
        assert_eq!(
            Workspace::format_fetch_progress("account", Some(&fetch)),
            ("account · INBOX: 7 / 10 reviewed".into(), Some(70.))
        );
    }

    #[tokio::test]
    async fn concurrent_windows_reuse_the_same_store_owner() {
        let root = tempfile::tempdir().expect("private profile");
        let shared: crate::SharedStore = Arc::new(tokio::sync::OnceCell::new());
        let (first, second) = tokio::join!(
            open_shared_store(&shared, root.path().to_owned()),
            open_shared_store(&shared, root.path().to_owned()),
        );
        let first = first.expect("first owner");
        let second = second.expect("same owner");
        assert!(Arc::ptr_eq(&first, &second));
        drop(first);
        drop(second);
        let reopened = open_shared_store(&shared, root.path().to_owned())
            .await
            .expect("reopened view");
        assert!(matches!(
            Worker::open(root.path().to_owned()).await,
            Err(mailstronaut_core::worker::Error::ProfileInUse)
        ));
        drop(reopened);
        let cell = Arc::try_unwrap(shared).unwrap_or_else(|_| panic!("unexpected session owner"));
        let owner = cell.into_inner().expect("initialized owner");
        let owner = Arc::try_unwrap(owner).unwrap_or_else(|_| panic!("unexpected worker owner"));
        owner.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "One scenario exercises initial load, prefix refresh and cancellation with explicit query filters."
    )]
    async fn refreshing_a_loaded_prefix_keeps_its_old_boundary_after_new_mail() {
        let root = tempfile::tempdir().expect("private store");
        let owner = Worker::open(root.path().to_owned()).await.expect("worker");
        let store = owner.handle();
        store.ensure_account("a".into()).await.expect("account");
        let folder = store
            .ensure_folder("a".into(), "INBOX".into(), true)
            .await
            .expect("folder")
            .id;
        let validity = NonZeroU32::new(42).expect("validity");
        store
            .select_folder("a".into(), folder, validity)
            .await
            .expect("select");
        for uid in 1..=205 {
            store
                .cache_message(
                    "a".into(),
                    Membership {
                        folder,
                        uid_validity: validity,
                        uid: NonZeroU32::new(uid).expect("UID"),
                        flags: Flags::default(),
                    },
                    i64::from(uid),
                    None,
                    format!("Subject: Synthetic letter {uid}\r\n\r\nLocal fixture.").into_bytes(),
                )
                .await
                .expect("cache");
        }
        let cancel = CancellationToken::new();
        let accounts = vec!["a".to_owned()];
        let first = conversation_prefix(
            &store,
            None,
            None,
            accounts.clone(),
            Scope::Inbox,
            String::new(),
            None,
            &cancel,
            SearchFilter::default(),
        )
        .await
        .expect("initial prefix");
        assert_eq!(first.len(), 100);
        let last = first.last().expect("boundary");
        let boundary = Cursor {
            received_at: last.latest.received_at,
            message_id: last.latest.id,
        };
        store
            .cache_message(
                "a".into(),
                Membership {
                    folder,
                    uid_validity: validity,
                    uid: NonZeroU32::new(206).expect("UID"),
                    flags: Flags::default(),
                },
                206,
                None,
                b"Subject: New synthetic letter\r\n\r\nArrived later.".to_vec(),
            )
            .await
            .expect("new mail");
        let refreshed = conversation_prefix(
            &store,
            None,
            None,
            accounts.clone(),
            Scope::Inbox,
            String::new(),
            Some(boundary),
            &cancel,
            SearchFilter::default(),
        )
        .await
        .expect("refresh");
        assert_eq!(refreshed.len(), 200);
        assert_eq!(refreshed.first().expect("newest").latest.received_at, 206);
        assert!(
            refreshed
                .iter()
                .any(|row| row.latest.id == boundary.message_id)
        );
        assert!(refreshed.iter().all(|row| !row.latest.seen));
        cancel.cancel();
        assert!(
            conversation_prefix(
                &store,
                None,
                None,
                accounts,
                Scope::Inbox,
                String::new(),
                Some(boundary),
                &cancel,
                SearchFilter::default(),
            )
            .await
            .is_err()
        );
        owner.shutdown().await.expect("shutdown");
    }
}

#[cfg(test)]
mod search_tests;
