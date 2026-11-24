# Implementation checklist

This is the **single source of truth** for the current direction and progress.
The application uses **GPUI Kit + Blitz**, with **native Wayland required**.
`[x]` records verified work at the stated scope; `[ ]` records unfinished
implementation or acceptance. Backend evidence remains useful where the code is
retained. Previous UI/browser screenshots, tests and package results do not prove
acceptance of the new stack. See [historical evidence](evidence/README.md).

Use `nix develop`, centralized workspace dependencies/lints and focused commits.
Keep mail, MIME, TLS, OAuth, Nix evaluation, threading, inference and rendering in
existing libraries. Do not implement a second toolkit, browser or search engine.

## Product and interface review

Build a minimal client people can choose for daily mail: fast to scan, easy to
navigate, reliable under errors and comfortable with several accounts. A lean,
well-engineered codebase and the requested useful features matter more than
matching another client's appearance or number of features.

Before adding custom UI, check the pinned GPUI Kit component catalog. Use the
existing component when it solves the problem; record a concrete API limitation
before introducing a custom replacement. [INTERFACE.md](INTERFACE.md) maps the
current components and supplied design references to mail workflows.

- [x] Download and inspect all seven supplied Apple Mail/Thunderbird/Spark/Proton/
  Gmail screenshots. Keep the images and download manifest only in ignored
  `target/design-references/`; do not commit or ship them.
- [x] Audit the pinned GPUI Kit Component/Base catalog against P01–P11 UI
  subproblems and record exact choices/limits in [INTERFACE.md](INTERFACE.md).
  This is source/API research, not implemented feature or runtime acceptance.
- [ ] At each completed UI workflow and before release, compare the actual app
  with those references for usability: navigation, information hierarchy, search,
  reader space, keyboard access and recoverable errors. Record remaining issues;
  screenshots do not establish working mail behavior or performance.
- [ ] After **each implemented feature**, capture the current actual application
  in a private native Wayland session and show the screenshot in chat. Use only
  synthetic mail; identify the feature and any acceptance still unverified. Show
  light/dark or narrow/wide variants when that feature affects them. A mockup,
  reference image or old application screenshot cannot satisfy this step.

## Milestone map

| ID | Outcome | Dependencies |
| --- | --- | --- |
| P00 | Current architecture and reproducible workspace | — |
| P01 | GPUI Kit + Blitz foundation, native Wayland and HTML safety | P00 |
| P02 | Native Nix configuration, credentials and GPUI settings | P00/P01 |
| P03 | Durable store, canonical identity and MIME projections | P02 schema |
| P04 | Reliable IMAP synchronization and offline operation | P02/P03 |
| P05 | Google, iCloud and institutional onboarding | P02/P04 |
| P06 | Native mailbox workspace, conversations and strong local search | P01/P03/P04 |
| P07 | Compose, drafts, attachments, submission and mailbox actions | P04/P05/P06 |
| P08 | Remind me, notifications and optional background operation | P03/P06 |
| P09 | Local priority classification, contextual expiry and deadlines | P03/P06/P08 |
| P10 | Nix/Home Manager, Flatpak and minimal-container CI | P01–P09 |
| P11 | Release and five-provider acceptance | P01–P10 |

All milestones block a complete v0.1 release. Start with the new P01 foundation,
then connect the retained backend and restore mail workflows through GPUI Kit.
Model research must not indefinitely postpone the usable mailbox/compose UI.

## P00 — Architecture and reproducible foundation

- [x] Retain `members = ["crates/*"]`, centralized dependencies, inherited metadata
  and strict practical Clippy policy.
- [x] Retain the Nixpkgs/Rust/Crane/flake foundation and minimal Nix-container CI.
  A previous-stack build is not acceptance of the new GUI package.
- [x] Record the current GPUI Kit, GPUI, Blitz and CPU-raster APIs, exact dependency
  choices, native Wayland route and upstream limitations in DEPENDENCIES/RENDERING.
- [x] Resolve/lock the new graph and native inputs; remove obsolete dependencies
  and build hooks. Avoid duplicate GPUI versions and untracked build downloads.
- [x] Pass current-stack format, strict Clippy, tests, Nix package and installed
  CLI/config smoke. Record exact commands, revision and remaining limits.

## P01 — Native GPUI Kit and Blitz foundation

Read RENDERING.md, DEPENDENCIES.md and [reader component choices](INTERFACE.md#p01-reader-and-shell). Record evidence in
`evidence/P01-gpui-blitz.md`. Earlier renderer evidence is historical.

- [x] Pin matching GPUI Kit/GPUI and Blitz/AnyRender/Vello CPU releases. Use the
  published component ecosystem; carry no large forks or reference-app patches
  without a demonstrated, documented need and an upstream removal path.
- [x] Build a normal GPUI Kit window with `TitleBar`/`Root`, standard assets/components, semantic
  light/dark themes, focus and keyboard bindings. Responsive resize/HiDPI acceptance
  is tracked below; the first native-window fixture is 1280×840.
- [x] Run on native Wayland with `DISPLAY` unset and no XWayland. Verify the actual
  platform connection and packaged window, not merely a compiled backend feature.
- [x] Embed actual Blitz HTML/CSS layout through AnyRender's Vello CPU backend and
  GPUI's standard image API. No child browser window, foreign X surface, alternate
  HTML engine or unnecessary GPU-sharing layer.
- [x] Bound HTML/DOM/resource/frame allocations; keep layout/raster work
  off the UI thread and fence message/resize/theme generations against stale frames.
  [Native owner/viewport evidence](evidence/P01-cached-html.md). Process-memory
  and latency/idle measurements remain open below.
- [x] Render synthetic ordinary and nested email tables, inline CSS, malformed
  HTML and Unicode; record unsupported layout/font/image behavior honestly.
  [Bounded native fixtures and limits](evidence/P01-cached-html.md); this is not
  acceptance of a comprehensive real-world layout corpus.
- [x] Enforce parser-based sanitization and default-deny resource, navigation and
  shell providers. Include no JS runtime, form submission, native commands,
  credential access or automatic external links. Do not assume browser CSP support.
  Remove active subtrees as well as attributes. [Evidence](evidence/P01-passive-viewport.md).
- [x] Verify the passive rendering API against remote/DNS/localhost/file/redirect/
  CSS/font/SVG/tracking fixtures with actual resolver, TCP and readable-file
  positive controls and independent syscall traces. The isolated network namespace
  is test containment, not the application's resource policy.
  [Bounded observer evidence](evidence/P01-passive-viewport.md).
- [x] Repeat bounded resource-policy and retirement acceptance through actual cached-message
  opening, switching, resizing and hiding after the MIME/HTML connector is enabled.
  [Independent native HTTP/file observations and current-frame pixels](evidence/P01-cached-html.md).
- [x] Implement immutable per-render PNG/JPEG CID tables through Blitz's decoded
  raster API; verify painting/alpha, ambiguous IDs, encoding/traversal, missing
  resources, wrong types, budgets and isolation between renders.
  [Renderer and preview evidence](evidence/P01-inline-images.md).
- [x] Connect bounded original-message MIME/CID descriptors to the HTML reader;
  respect truncated/ambiguous MIME parts and verify resource retirement during
  asynchronous message switching. CSS CID backgrounds remain unsupported.
  [MIME/worker/native isolation evidence](evidence/P01-cached-html.md).
- [x] Extract transfer-decoded local PNG/JPEG parts through the existing MIME
  library, preserving opaque IDs and validating original revision, ambiguity,
  encoding, descriptor ranges and compressed-byte budgets. Renderer validation
  remains separate. [Evidence](evidence/P01-passive-viewport.md).
- [x] Forward the app's selected color scheme through Blitz's viewport/media API.
  Respect fixed-light emails; do not invert images or reproduce desktop themes.
- [x] Provide a native read-only text reader using `TextareaState`/`Textarea::readonly(true)`;
  verify Unicode selection/copy, rejected edits, document-end keyboard scrolling
  and selection preservation across theme changes in a private Wayland session.
  [Bounded evidence](evidence/P01-native-reader.md).
- [x] Use `TabBar::segmented`/`Tab` for HTML/text mode and `Toolbar`/`ToolbarGroup`
  for reader actions. Reuse `ScrollableElement`/`ScrollHandle` for scrollbars;
  implement actual viewport rendering, resize, HiDPI and native keyboard scrolling.
  [Installed Wayland evidence](evidence/P01-cached-html.md).
- [x] Establish an appropriate public-API HTML selection/copy route if practical;
  keep native Text/Select text usable for every readable message. A raster alone
  is not text accessibility. Verify actual assistive-technology behavior below.
  [Drag selection, current-frame Unicode copying and the keyboard text alternative
  on private native Wayland](evidence/P01-cached-html.md#selecting-text-in-the-html-viewport).
- [x] Provide a retained passive document API that reuses DOM/font/raster state,
  repaints only bounded physical viewport pixels, forwards logical scale/theme,
  clamps scroll and rejects invalid geometry/excessive CSS extents. Verify pixel
  results across scroll, scale, resize and theme changes. Native UI integration,
  scrollbars and switching are verified above; performance acceptance remains below.
  [Renderer evidence](evidence/P01-passive-viewport.md).
- [x] Implement explicit validated portal links with a standard `Button` (or
  `Link` without `href`). In this pin `Link::on_click` does not override `href` and
  `Link::disabled` does not gate navigation; never pass untrusted URLs to defaults.
  Actual current-frame clicks and a Kit Button/DropdownMenu web-link picker use
  canonical HTTP(S) targets and the ASHPD portal. Source/viewport, keyboard,
  cancellation/refusal, 2x hit and no-fetch observations pass in private fixtures.
  [Bounded installed-package evidence](evidence/P01-web-links.md). Real desktop/
  Flatpak portal activation remains P10/P11 acceptance.
- [x] Verify keyboard behavior and the actual accessibility tree on Wayland.
  The native fixture checks the mailbox, reader, composer, editable accounts,
  Settings export and discard confirmation. Orca Say All reads the multilingual
  message through the native Text view. [28 AT-SPI checks and synthesized speech](evidence/P01-accessibility.md#account-editing-and-discard-confirmation).
  Full daily-workflow and release accessibility remain P11 checks.
- [x] Measure long-email responsiveness, repeated message switches, frame memory
  and hidden idle CPU; drop obsolete documents and stop hidden rendering. Measured in
  a headless software-rendered session with the release binary: 100 switches with no
  wrong paint, long-original opens, a flat resident set after the fix, and zero idle
  CPU while hidden with background operation enabled. Scope and limits:
  [P01 performance](evidence/P01-performance.md).
- [x] Pass current-stack Nix packaging and an early offline Flatpak SDK/runtime
  check, preserving native Wayland and no dependency on a host Nix executable.
  [Local installed Nix reader](evidence/P01-native-reader.md#local-installed-package-reader-recheck)
  and [pinned SDK/Wayland Flatpak](evidence/P10-flatpak.md#local-sdk-and-installed-runtime-recheck).

An upstream limitation leaves its row open. Investigate practical options within
this stack; do not restore the previous architecture or fabricate a reader.

## P02 — Native Nix configuration and encrypted secrets

Reuse `mailstronaut-config`; read CONFIGURATION.md. The input remains a native
`.nix` expression. Home Manager directly installs that source or manages an
explicitly mutable native Nix file. An optional small HM module may manage file
ownership and activation, but must not mirror the application's schema or convert
to another configuration format. No hidden TOML, imported Nixpkgs or host Nix daemon
is required by the application.
Use the [settings component mapping](INTERFACE.md#p02-settings-and-p05-onboarding).

- [x] Retain typed schema/defaults/unknown-field rejection and the bounded pure
  Snix same-binary helper, resource limits, cancellation/reaping and redacted IPC.
- [x] Retain `config init/check`, explicit paths, no-overwrite/permissions checks
  and last-valid configuration state. [Backend evidence](evidence/P02-accounts.md).
- [x] Retain scoped oo7 storage, encrypted persistence, rotation, account/purpose
  isolation, lock and unavailable behavior against a private native Secret Service.
  Existing protocol evidence does not prove new UI or Flatpak portal acceptance.
- [x] Connect declared password accounts to native GPUI Kit Settings pages,
  source-path copy, masked encrypted password save, explicit bounded TLS/PEEK sync
  and offline reading. Verify invalid reload retention, explicit-open-only Seen,
  restart and disable-without-purge in the private Wayland fixture.
  [Bounded current-stack evidence](evidence/P02-gpui-accounts.md).
  This does not complete editable onboarding, provider login or all P02/P04 gates.
- [x] Build settings/onboarding with GPUI Kit `Settings`, `SettingPage`,
  `SettingGroup`, `SettingItem` and existing `SettingField` controls; reuse built-in
  page navigation/search and disabled-state behavior. Use masked `Input` for secret
  entry and explicit foreground keyring/OAuth actions, never a generic stored field.
  Basic preferences, encrypted credential pages and native first-run/manual/
  Google/iCloud declaration forms passed private Wayland workflows.
  [Scoped onboarding evidence](evidence/P05-onboarding.md); live-provider and
  installed-package acceptance are separate requirements.
- [x] Show the native source path/ownership/last reload with `DescriptionList`,
  a `Clipboard` path-copy action and persistent `Alert` reload errors.
  Protect managed, store-backed, read-only and unadopted user-authored expressions;
  credentials remain editable through the keyring even with managed configuration.
  [Preferences and ownership evidence](evidence/P02-preferences.md).
- [x] Provide the native `config export/reconcile` CLI and optional HM module:
  raw declared keys precede defaults, standard Snix serialization preserves data,
  app-owned sources use cooperative locking and checked atomic replacement, and
  invalid/unadopted/read-only/foreign-link changes are rejected. Verify raw helper
  bounds/redaction, native round trips, precedence/omissions/lists and conflicts.
  [Bounded native Nix evidence](evidence/P02-native-nix.md).
- [x] Connect editable UI preferences to opt-in HM `mutableSettings` with one writable native `config.nix`:
  declared attributes win on activation, undeclared UI values survive, lists replace
  as whole values, and removing a declaration does not implicitly delete data.
  Preserve explicitly declared keys before applying schema defaults. Explain that
  UI edits to declared values are reapplied from HM on the next switch.
  Appearance, automatic sync/polling, notification/privacy, timezone and reminder
  time fields stage Save/Discard; the shared writer preserves raw omissions and
  rejects stale sources. [Bounded native acceptance](evidence/P02-preferences.md).
  Full Home Manager generation activation remains a separate P10 gate.
- [x] Add **Export to Nix** preview/copy from Settings with a validated secret-free native
  attrset snapshot that can be imported as HM `settings` or installed directly.
  Include supported account declarations and secret references, never credentials
  or mailbox/runtime state. Do not rewrite the user's original expression or dotfiles.
  Use `Dialog` + read-only `Textarea`, `Clipboard` and a Save `Button` invoking
  the portal; use an in-app `Notification` for completed copy/save feedback.
  Native preview/Clipboard/read-only/unchanged-source/Seen and light/dark checks
  pass; no-clobber private save publication is unit-tested and missing-portal
  feedback is exercised. [Evidence](evidence/P02-native-nix.md).
- [ ] Accept successful/cancelled/denied FileChooser export through the native
  desktop and Flatpak portals. [Save, cancellation, refusal and no-clobber passed
  on a private native session bus](evidence/P02-native-nix.md#local-native-filechooser-export).
  Live desktop portals and Flatpak document grants remain open.
- [x] Verify edit/save/restart/export/reimport equivalence, native string escaping,
  default/omitted fields, activation precedence, concurrent-write conflicts, invalid
  reload retention and mutable/immutable transitions in disposable profiles.
  The [same-home generation fixture](evidence/P02-preferences.md#home-manager-ownership-transitions)
  passed the native Settings round trip, declared precedence and safe ownership
  changes. The configuration integration tests also passed writer conflicts,
  string escaping and last-valid reload retention.
- [ ] Reconfigure account workers atomically; stale callbacks and invalid/removed
  configuration cannot replace the last valid account state or expose secrets.
  Explicit older-mail requests and server attachment saves now retire with an
  accepted account declaration. [Private Wayland and Dovecot checks](evidence/P02-gpui-accounts.md#explicit-fetches-during-configuration-reload)
  passed for reuse of folder IDs and delayed chooser replies.
- [ ] Verify the same `.nix` source through disposable Home Manager integration,
  native package and Flatpak regular-file/document access. Never activate user dotfiles.
- [ ] Prove no password/token appears in config, Nix store, Debug, tracing or failed
  diagnostics; test disabled accounts separately from explicit data purge.
- [ ] Verify real desktop unlock and Flatpak Secret portal behavior, without a
  plaintext credential fallback or an externally executed password command.

## P03 — Local store and canonical message projections

Reuse `mailstronaut-core`, rusqlite/migrations, mail-parser and mail-threading.
Existing synthetic backend tests/evidence remain relevant; rerun them with the
new dependency graph before claiming current-package acceptance.

- [x] Verify private XDG paths, one blocking store owner/profile lock, bounded
  commands, WAL/foreign keys/busy timeout and migration backup/rollback. Present
  unavailable/corrupt stores with `Empty`/`Alert` and a real Retry action; use
  `AlertDialog` only for explicitly destructive purge/reset decisions.
  [Store, worker and native Wayland recovery checks](evidence/P03-store-retry.md).
- [x] Verify atomic SHA-256 blob ingestion/GC, cross-handle transactions and cache
  clearing that preserves drafts/outbox/reminders; add crash-injection acceptance.
  [Named checks for each clause, including real kills at three points of one pass](evidence/P03-durability.md).
- [x] Complete bounded MIME/non-UTF8/alternative/nested/malformed/encrypted fixtures,
  attachment/CID descriptors and source/body revisions. No generated summaries.
  [Current synthetic MIME fixture results](evidence/P03.md#bounded-attachment-descriptors-and-revisions).
- [x] Complete account/folder/UIDVALIDITY/UID identity, multi-membership/provider
  deduplication and duplicate Message-ID tests; never merge different accounts.
  [Named tests per clause, including a UIDVALIDITY reset on a real server](evidence/P03-identity.md).
- [x] Retain account-local RFC5256/JWZ grouping with subject merging disabled,
  revision-fenced stable conversations and the 37-case upstream conformance corpus.
- [x] Complete [missing/late/Sent ancestry, cycles, duplicate IDs and merge behavior](evidence/P03-scale.md#late-sent-ancestry-at-the-account-limit)
  at measured mailbox scale; sequence numbers never become persistent identity.
- [x] Verify paginated unified/search projections, derived-index purge/rebuild,
  partial-cache states and bounded 100k-message behavior. See P06 for strong search.
  [Queries are measured at 100k and an over-limit account still lists its
  mail](evidence/P03-scale.md); [scoped history coverage](evidence/P04-history.md#scoped-cache-coverage)
  distinguishes interrupted and capped history.
  [Live indexing, query refresh and 100,000-message measurements](evidence/P03-index-progress.md).
  Rebuild progress uses `Spinner`, or `Progress` only with a known bounded total;
  [components present store state](INTERFACE.md#p03-store-and-p04-synchronization), not persistence logic.

## P04 — IMAP synchronization and offline operation

- [x] Fetch the complete selected-folder history by default, in bounded UID windows
  with durable per-folder cursors, restart after a kill, and backpressure. An optional
  native `sync.initialDays` cap keeps its meaning when declared; absent or null means the
  complete history. Nothing is deleted by either. [Evidence](evidence/P04-history.md).
- [x] Expose the advanced history age limit in Settings and Home Manager. The private
  Wayland save-and-clear walkthrough and disposable Home Manager activation pass.
  [Age cap and configuration evidence](evidence/P04-history.md#home-manager-history-cap).
- [x] Offer explicit retention eviction, preserving drafts, pending operations,
  local attachment snapshots and other user-owned state.
  [Store regression and private Wayland/Dovecot walkthrough](evidence/P04-history.md#explicit-local-retention).
- [x] Show per-account and per-folder fetch progress with Kit `Progress`. Display the
  measured reviewed count and total when known. Use indeterminate progress during
  connection and fetch setup. The native two-account, cancellation and failure
  walkthroughs cover the status presentation.
- [x] Cancel a running account fetch from the status bar after queued server writes
  settle. Keep cached originals and the committed cursor, and support resuming on
  an explicit sync. [Private Dovecot and Wayland evidence](evidence/P04-history.md#native-fetch-cancellation).
- [x] Fetch configured accounts on Sync mail or Ctrl+R with automatic sync disabled.
  Continue that request's unfinished history and leave polling and IDLE off after
  completion. [Native two-account check](evidence/P04-history.md#explicit-only-sync).
- [x] Keep account errors independently dismissible and retry only the account named
  in the warning. Show scheduled retry delays for transient failures and keep cached
  mail available during server or authentication failures.
  [Native failure recovery](evidence/P04-faults.md#account-specific-retry).

- [x] Retain validated TLS/required STARTTLS, bounded UID pages, PEEK retrieval,
  expunge/flag reconciliation, UIDVALIDITY fencing and transactional cache backend.
  [Protocol evidence](evidence/P04.md); earlier frontend acceptance is historical.
- [ ] Connect this backend to GPUI with explicit sync, cancellable progress/errors,
  independent account state, configuration generations and useful offline reading.
  Reuse `Button::loading`/`Spinner`, Cancel/Retry `Button`s and persistent per-account
  `Alert`s; show offline/partial coverage in `StatusBar`, retaining cached rows.
  The status bar now displays one active account/folder's actual reviewed count and
  measured or indeterminate Kit progress indicator. A synthetic Dovecot/Wayland run
  displayed 1024 of 2304 reviewed and passed restart, cap, UID reset and Seen checks;
  [Multi-account progress](evidence/P04-history.md) and [scoped history
  coverage](evidence/P04-history.md#scoped-cache-coverage) have native evidence.
  Real-provider behavior remains open.
  [Automatic sync passes against a real Dovecot](evidence/P04-background-sync.md);
  live providers remain open.
- [ ] Complete native/extra-CA trust tests, missing STARTTLS/invalid cert failures,
  capability discovery, account cancellation/backoff and fair connection/work budgets.
- [x] Complete SPECIAL-USE discovery, explicit folder overrides and verified
  UTF-7/UTF-8 names, without lossy folder identity.
  [Evidence](evidence/P04-background-sync.md). Raw UTF-8 wire names stay refused, since
  UTF8=ACCEPT is not enabled; invalid names display as sent.
- [ ] Complete bounded recent sync, older paging, partial-cache status, on-demand
  bodies/attachments and safe UIDVALIDITY/gap/reset reconciliation.
  [On-demand attachments](evidence/P04-large-mail.md) and [older mail from the
  mailbox](evidence/P04-older-mail.md) pass on a real server; partial-cache status
  and gap reconciliation remain open.
  Use `ListDelegate::has_more/load_more` or an explicit Load older `Button` for
  backend paging; never report an invented progress percentage.
- [x] Implement IDLE/DONE/renew/reselect with polling fallback, non-INBOX flags,
  reconnect, offline/resume recovery and changes made by other clients.
  [Real Dovecot and scripted-server evidence](evidence/P04-background-sync.md); a real
  suspend and live providers remain unrun.
- [x] Preserve local reminder/dismiss/analysis state across moves/reconciliation.
  Neither background fetching, previews nor analysis sets Seen.
  [Store test through a move and reconciliation, with Seen checks in native walkthroughs](evidence/P04-state.md).
- [ ] Exercise disposable protocol servers, drops and authentication failures in
  Nix/container CI; measure idle CPU, cancellation, paging and failure isolation.
  [Mid-sync drops, idle CPU, failure isolation, closing mid-sync and wrong-password attempts are measured natively](evidence/P04-faults.md);
  other authentication failures and container CI remain open.

Optional CONDSTORE/QRESYNC requires separate extension proof before adoption.

## P05 — Required account onboarding and authentication

- [ ] Complete account settings acceptance for editable incoming and outgoing login
  names, including managed and Home Manager reactivation cases. The app-owned
  native source save and reload passed a synthetic Wayland interaction; live
  provider authentication and full Home Manager activation remain unverified.

- [x] Retain public-client OAuth S256/CSRF/code exchange, external portal browser,
  scoped encrypted refresh/rotation and shared IMAP/SMTP sessions. The mock IdP,
  keyring and callback backend tests remain valid. [Evidence](evidence/P05.md).
- [x] Build first-run/account declarations and status with GPUI Kit. Never render
  a provider login form inside email HTML. Shared checked Nix persistence appends
  accounts, rejects duplicates/stale sources and generates fresh credential refs.
  Use `Form`/`Field`, `Select`/`SelectState` for provider/auth/TLS, `Input` for
  address/login/host and `NumberInput` for ports. Typed schema validation remains
  authoritative; `Field::required` alone does not validate a connection.
  Private native tests cover advanced manual values, Google/iCloud declarations,
  keyboard submission, credential routing, export/restart and unchanged Seen.
  A private encrypted-keyring/Dovecot test verifies that saving a password wakes
  a stopped automatic-sync worker. [Evidence](evidence/P05-onboarding.md).
- [x] Verify Wayland sign-in through the browser portal with a synthetic HTTPS
  provider. Confirm encrypted grant persistence and refresh after restart.
  [Private native OAuth walkthrough](evidence/P05.md#current-wayland-authorization).
- [ ] Verify real browser consent, per-account refresh errors/cancellation and
  reauthentication across provider changes and pending configuration reloads.
- [ ] Ship an application-owned Google desktop OAuth registration and make normal
  Add Google account open browser sign-in without asking users for client IDs.
  Keep custom registrations in Advanced settings and clearly report unavailable
  sign-in in builds without an application registration; never borrow another
  client's identity or fabricate credentials.
- [ ] Verify Google public desktop-client ownership, consent/scopes, OAuth IMAP/SMTP,
  revoked/expired tokens and actual login/sync/controlled send with authorization.
- [ ] Verify iCloud account/app-specific password flows and provider endpoints.
- [ ] Support manual IMAP/SMTP/TLS/CA/auth configuration for INRIA, Politecnico di
  Torino and Université de Pau et des Pays de l'Adour; verify each actual supported
  route, differing login names, aliases and authentication restrictions.
  Put optional SMTP/folder/CA controls in `Collapsible`; use `InputGroup` with a
  Browse/Import `InputGroupButton` for standard portal file selection. Use ordinary
  Sign in/Unlock/Reauthenticate/Cancel buttons with `Spinner`/`Alert`, not a new wizard.
- [ ] Test locked/unavailable keyring, denied/cancelled browser, invalid callback,
  refresh races and logout/purge. Complete native Wayland and Flatpak portal flows.
- [x] Keep live-provider outcomes separate from local mock/compile evidence.
  The [per-provider acceptance record](evidence/P05.md#required-provider-acceptance)
  lists all five live outcomes alongside their synthetic coverage.

## P06 — GPUI mailbox workspace and strong local search

- [x] Replace Older/Newer/page controls with continuous virtualized browsing. Reuse
  Kit `List`/`ListState` (already virtualized), `ListDelegate::has_more`,
  `load_more` and `load_more_threshold` with bounded backend query windows.
  Permit one in-flight batch per query generation; ignore stale completions. Keep
  stable row identities and scroll anchoring; do not materialize all MIME bodies.
  Verify scrolling past 100 rows, search/folder changes, new-mail insertion and
  keyboard navigation without missed or duplicated rows. Keep implementation
  batches internal; there must be no user-facing pagination.
  Native Wayland tests cross 100 rows by wheel and keyboard, preserve the visible
  rows on refresh, and reset on folder and query changes. The 108-original thread
  also loads across the same boundary; passive navigation leaves its Seen state
  unchanged. [Native walkthroughs and arrival tests](evidence/P06-continuous-and-settings.md).
- [x] Verify unread dots in both themes, readable 99+ sidebar counts, one app title,
  icon actions with accessible names/tooltips and a single Settings selection state.
  [Synthetic native walkthrough](evidence/P06-ui-feedback.md).

Use the [mailbox/search component mapping](INTERFACE.md#p06-mailbox-threads-and-search).

- [x] Retain scoped cached-folder queries, literal FTS5 and account-local
  conversation projections. [Backend evidence](evidence/P06-browse.md).
- [x] Replace the row scaffold with GPUI Kit List/ListState/ListDelegate/ListItem,
  one external query field, account/thread/unread indicators and passive arrow
  selection. Confirm once through ListEvent::Confirm; reload canonical unread
  counts after a committed explicit open while retaining the newer body and
  passive group selection. Verify the overlapping write/query/body case and
  matching-original search in the private Wayland fixture.
  [Bounded current-stack evidence](evidence/P06-gpui-mailbox.md).
- [x] Use GPUI Kit `Sidebar`, `SidebarGroup`, `SidebarMenu` and
  `SidebarMenuItem` for unified/per-account folders and Settings. Keep the native
  titlebar as the single app title and the Settings item as one selection surface.
  Reuse active/collapsed states, icons and `Badge::count`; show only implemented
  mail routes. Prefer Sidebar nesting; evaluate `Tree`/`TreeState` only if actual
  folder scale requires it, without maintaining two navigation models.
  [Routes, active states and nesting](evidence/P06-navigation.md); [unread counts on a
  real server](evidence/P04-background-sync.md).
- [x] Deliver `h_resizable`/`resizable_panel` mailbox/list/reader panes and
  `List`/`ListState`/`ListDelegate`/`ListItem` virtualized rows. Disable the list's
  extra query field with `ListState::searchable(false)`. Use `v_virtual_list` only
  when measured variable-height rows require it; do not implement virtualization.
  Compose `Label`, `Icon`, `Badge::count/dot` and explicit row accessibility labels
  for account, sender/subject/date/two-or-three-line preview hierarchy,
  distinct focus/selection/unread/priority states and keyboard commands. Keep search,
  compose and essential actions discoverable without crowding the reading area.
  [Native pane and accessible row checks](evidence/P06-gpui-mailbox.md#resizable-panes-and-accessible-rows)
  cover both dividers, keyboard selection, dates, previews and priority actions.
- [ ] Verify useful empty/loading/offline/error/partial-cache states, long subjects,
  Unicode names, narrow/wide windows and light/dark appearance using synthetic data.
  Reuse list empty/loading hooks, `Empty`, `Skeleton`, `Alert` and `StatusBar`;
  distinguish no results, missing models and partial indexes from an empty mailbox.
- [ ] Preserve stable selection/scroll through updates; cancel stale account,
  folder, query, body and render work. Measure large-list responsiveness.
  Separate query and body/render generations: repaint rows from the canonical
  projection after a local flag commit without cancelling a newer explicit body
  open. The bounded same-scope path is implemented above; complete account/folder,
  conversation-merge, paging, removal and large-list acceptance before this row
  is complete. Preserve unread siblings through every state change.
  [Late IMAP arrivals now preserve the selected group and its scroll anchor](evidence/P06-conversation-merges.md).
- [x] Display each account-local thread once, include cached Sent ancestry, open
  the matching original and allow per-message selection/actions.
  Reuse `Collapsible` originals (or `Accordion::multiple(true)` if appropriate)
  and `DescriptionList` for From/To/Cc/date; do not adopt chat/tail-following behavior.
  [Individual-original flags, replies and header details](evidence/P06-navigation.md#acting-on-an-older-original).
- [x] Mark Seen only after an **explicit open action loads that original's content**.
  Passive focus/selection/restoration/search/previews/refresh never mark read;
  opening one member does not mark its whole thread. Preserve explicit unread.
  Keep `ListDelegate::set_selected_index`/`ListEvent::Select` passive;
  `ListDelegate::confirm`/`ListEvent::Confirm` invokes the existing content-load gate.
  [Native and worker evidence](evidence/P06-continuous-and-settings.md).
- [x] Display extracted two/three-line previews, safe HTML/CID and accessible text
  modes; verify actual Wayland keyboard/copy/link behavior.
  [Wayland renderer and portal checks](evidence/P01-cached-html.md#gpui-kit-0100-recheck)
  and [native multiline AT-SPI text](evidence/P01-accessibility.md#multiline-text-with-gpui-kit-0100).
- [x] Research maintained fuzzy/hybrid libraries and local encoders, with exact
  source/API and isolated engine/resource probes. [Research](SEARCH.md).
- [ ] Finish model/library adoption gates and choose the smallest maintained
  integration meeting quality/resource budgets. [Tantivy is adopted for lexical
  search](SEARCH.md). Existing E5 and Model2Vec candidates fail their fixed relevance
  screen; Potion also exceeds its memory screen. [The MiniLM L12 candidate now passes
  the fixed screen, Rust/reference parity and encoder-only resource limits](evidence/P06-minilm-encoder.md);
  its frozen additional screen also passes (22/24 first, 24/24 top-three), with two
  retained book-pickup/renewal confusions. The [USearch API and 10k/100k vector screen also pass](evidence/P06-vector-index.md),
  selecting the pair for production integration. Combined app/lifecycle budgets and
  robust no-answer discrimination remain open.
- [x] Provide typo-tolerant fuzzy subject/address/body search, accent/partial terms,
  useful ranking, deliberate exact/filter queries and bounded expansion, using a
  maintained library rather than a custom matcher or index.
  [Tantivy, with native acceptance and measurements](evidence/P06-fuzzy-search.md).
- [x] Verify the reusable CPU encoder and persistent vector adapter in the workspace:
  hashed explicit Nix installation, bounded overlap/coverage, exact source/model
  invalidation, filtered retrieval, transactional embedding cache and graph recovery.
  [Implementation and remaining acceptance](evidence/P06-semantic-backend.md).
- [x] Provide a separate canonical semantic-ingestion frontier and typed worker API;
  lexical acknowledgments must not discard inference work. Verify bounded readable
  source hashes, late replay/deletion, upgrade seeding and metadata/UID isolation
  without setting Seen. [Backend evidence](evidence/P06-semantic-ingestion.md).
- [ ] Complete multilingual relevance/no-answer acceptance. The explicit native
  Related mode retrieves multilingual suggestions with honest labels and preserves
  word search as the default; its frozen no-answer screen fails (4/12 false-positive
  queries), so do not mark the overall quality gate complete. Keep the original and
  held-out corpora unchanged; evaluate any existing local reranker on independently
  frozen data, within combined resource budgets. **"reminder about doctoral school
  re-enrollment" finds the French doctoral registration notice without matching
  query words** remains a required native case.
- [x] Verify the native Related button, Settings enable/path save, missing-model word
  fallback, matched-original opening and passive Seen behavior in private Wayland.
  Use standard Kit `Button`, `Alert`, status and `SettingField` controls from the
  [component mapping](INTERFACE.md#p06-mailbox-threads-and-search).
  [Real-model worker and native Dovecot/Wayland evidence](evidence/P06-native-meaning.md).
- [x] Select an explicitly installed search model with the native folder chooser
  in editable Settings. Cancel or deny leaves the Nix source intact, and a
  successful selection is staged until Save.
  [Private Wayland and FileChooser checks](evidence/P06-native-meaning.md#local-model-folder-chooser).
- [x] Keep queries/text/embeddings/inference local. Models are small CPU encoders,
  pinned/hashed/licensed and explicitly installed; no startup downloads or remote
  fallback. Semantic search is independent of enabling the priority classifier.
  [Explicit installation and native ownership/fallback checks](evidence/P06-native-meaning.md).
- [x] Use one native search field with library ranking, scoped account/folder/date/
  sender filters and conversation deduplication. Search never sets Seen.
  Reuse `Input`, a filter `Popover`, sender `Input` and `DatePickerState::range`
  with standard presets/Apply/Clear buttons. Account/folder scope stays in the
  sidebar rather than duplicating its controls in the popover.
  `SearchableVec`/default list substring matching is suitable for small option menus,
  not the required fuzzy or multilingual search engine.
  [The one field, `from:`/`subject:`, scope and Seen pass](evidence/P06-fuzzy-search.md);
  [sender/date controls, bounded metadata reads and filtering before Tantivy's hit
  limit pass](evidence/P06-search-filters.md). Canonical folder IDs now also
  restrict Tantivy's collector before the hit cap, including fuzzy-tier choice;
  [large-folder native and ranked-refresh evidence](evidence/P06-search-scope.md).
- [x] Display actual body matches in unqualified word searches from the original
  that opens. Reuse SQLite FTS5 excerpts, Tantivy's analyzer and GPUI
  `StyledText::with_highlights` with original UTF-8 offsets. Suggestions by meaning,
  field restrictions and quoted queries retain their ordinary previews.
  [Native and backend checks](evidence/P06-fuzzy-search.md#literal-body-excerpts).
- [ ] Maintain a private rebuildable index keyed by canonical original/source/model
  revision; bounded incremental indexing, restart/model change, deletion/purge and
  disabled accounts must not retain stale results or touch user-owned state.
  [The lexical index passes](evidence/P06-fuzzy-search.md); the semantic component
  now versions model/projection policy and source hashes. Canonical ingestion,
  source-fenced presentation and configured-account/model/cap retirement are
  implemented; combined-scale recovery and secure account purge remain open.
- [x] Show partial-cache/index coverage and unavailable/disabled models. Local
  lexical/fuzzy search remains usable, but this does not complete semantic acceptance.
  [Messages not yet indexed are counted in the status bar](evidence/P06-fuzzy-search.md);
  model indexed/pending/excerpt and unavailable states are implemented. The same
  footer reports [partial history in the viewer's scope](evidence/P04-history.md#scoped-cache-coverage).
- [ ] Pass fixed and held-out relevance, filter/isolation/restart/deletion tests,
  the user's French example, and measured 10k/100k CPU/RAM/index/build budgets.

## P07 — Composition, submission and mailbox actions

Use the [composer/action component mapping](INTERFACE.md#p07-composition-and-actions).

- [x] Retain revisioned transactional drafts, lettre MIME/Unicode/Bcc handling,
  immutable originals/Message-ID, fenced SMTP attempts and delivery uncertainty.
  [Backend evidence](evidence/P07.md); previous UI evidence is historical.
- [x] Retain reply/reply-all/text-forward account/alias/ancestry preparation and
  self-recipient exclusion. [Evidence](evidence/P07-reply.md).
- [x] Retain bounded owned attachment snapshots, independent account copies,
  rejection recovery and freeze/hash integrity. [Evidence](evidence/P07-attachments.md).
- [x] Retain read/unread/star/unstar intentions, unsent-only undo, TLS application
  and exact UID/configuration fences. [Evidence](evidence/P07-actions.md).
- [x] Build the composer with `Form`/`Field`, an identity `Select`, recipient/subject
  `Input`s and body/signature `Textarea`; no source-code editor or custom text editor.
  Support To/Cc/Bcc, signature, reply/forward and validation. Keep identity and save/send
  state unambiguous. Use `Collapsible` for Cc/Bcc. Start with plain address input;
  if recipient chips are needed, reuse `InlineToken`/`InputToken` and existing
  replacement APIs rather than custom token editing or a contacts subsystem.
  [Editor, identity, Cc/Bcc, reply/forward, validation and signature](evidence/P07-native-compose.md).
- [ ] Reuse `Attachment`, `AttachmentGroup`, `AttachmentMedia`, `AttachmentContent`,
  `AttachmentTitle`, `AttachmentDescription` and `AttachmentActions` in composer and
  reader. Show filename/type/size and real pending/processing/failed/ready states;
  use built-in remove/retry affordances only where the operation supports them.
- [ ] Connect portal Attach/Remove to owned draft snapshots, and explicit received
  attachment Save/Open to bounded extraction and safe portal destinations.
  [Save and Open pass for cached and on-demand parts through a private portal](evidence/P04-large-mail.md#opening-a-received-attachment).
  [Draft attachment removal survives restart](evidence/P07-native-compose.md). Real desktop portal and document grants remain open. The
  component is presentation, not a downloader or upload service. Never pass an
  email-supplied URL/path to its image source; use generic icons or validated owned
  raster previews. Verify keyboard actions, long names, failures and narrow layouts.
- [x] Verify autosave, draft switching, source-file access loss, close/restart and
  failure recovery on Wayland. A selected file becomes a private owned snapshot.
  Expose draft/save/outbox state in `StatusBar`, with persistent `Alert` failures.
  [Draft recovery across the file chooser, draft switches and restart](evidence/P07-native-compose.md#draft-recovery-during-attachment-selection).
- [x] Integrate explicit send/offline queue/cancel/retry and redacted TLS/auth errors.
  Preserve fixed frozen bytes and Message-ID; no automatic ambiguous-delivery retry.
  [Accepted, unreachable, rejected, unknown, untrusted-certificate, refused sign-in, retry,
  in-flight cancel and a queued send across a crash pass natively on loopback
  fixtures](evidence/P07-native-compose.md), not on live providers.
- [ ] Complete accepted-DATA/drop reconciliation, provider Sent auto-save and
  separately retriable Sent APPEND. Sent-copy failure must never trigger SMTP resend.
  [Sent APPEND passes against a real server](evidence/P04-background-sync.md), and a lost
  acknowledgment [waits for an explicit decision](evidence/P07-native-compose.md). A failed
  Sent copy stays pending during an outage, is never resubmitted and is saved once by the
  next sync [natively against Dovecot](evidence/P07-native-compose.md); provider-side Sent
  saving is not exercised.
- [ ] Port native flag/undo controls and implement safe archive/trash/move with
  pending operations, UID MOVE/UIDPLUS fallbacks and no unrelated global EXPUNGE.
  Reuse `Toolbar`/`ToolbarGroup`, `Button`, `DropdownButton`/`PopupMenu` and
  `ContextMenuExt`; use searchable `Combobox` for move destinations and explicit
  in-app `Notification::action` for Undo. Keep durable errors beyond toast lifetime.
  [Read, star, archive, trash and move with Undo](evidence/P04-background-sync.md);
  [Undo and servers without MOVE or UIDPLUS pass natively](evidence/P07-native-actions.md).
  [Searchable folder selection, cancellation and Undo](evidence/P07-native-actions.md#searchable-move-destinations)
  have native coverage.
- [ ] Recover interrupted operations and reconcile real server state, retaining
  local reminder/analysis identity and visible permanent failure/undo behavior.
  Use `AlertDialog`/`Dialog` for discard, destructive purge or ambiguous-delivery
  decisions; no routine confirmations for already reversible actions.
- [ ] Verify native composition/attachments/actions on Wayland and Flatpak, plus
  authorized controlled send/reply flows on all required providers.
  [Native Wayland attachment removal, identity switch, forward and discard pass on synthetic
  fixtures](evidence/P07-native-compose.md); the Flatpak portal and live-provider parts remain
  open.

HTML composition, remote draft sync, signing and permanent purge remain deferred.

## P08 — Remind me and background lifetime

Use the [reminder component mapping](INTERFACE.md#p08-reminders-and-background).

- [x] Retain durable reminder generations/claims/receipts and store transitions.
  [Backend/historical evidence](evidence/P08.md). The old UI/scheduler walkthrough
  does not establish GPUI acceptance.
- [x] Use `PopupMenu` for reminder presets and `DatePicker` with
  `DatePickerState::time_precision(TimePrecision::Minute)` for a custom instant. Retain
  original timezone/UTC, reschedule/cancel and per-message routing. Chrono TZ resolves
  the component's civil time; the widget does not implement DST policy or scheduling.
  [Presets, custom picker, list and cancel](evidence/P08-native-reminders.md);
  native elapsed-hour and custom date/time, reschedule, per-original defaults and
  restart/cancel checks pass. The picker uses its default 24-hour clock and
  `set_date_time` for the current reminder or tomorrow morning; avoid duplicating
  the external preset menu inside it.
- [x] Complete native GUI coverage for evening/tomorrow presets, invalid civil
  times and timezone changes with a reminder already scheduled. [Presets, a
  rejected DST gap with correction, and timezone reload pass
  natively](evidence/P08-native-reminders.md#presets-and-time-zone-reload).
- [x] Use `TimeField` for configured morning/evening defaults and searchable
  `Combobox` for editable timezone; verify save/discard/restart through the native
  Settings workflow. [Evidence](evidence/P02-preferences.md).
- [x] Use `DescriptionList` for scheduled-time details, including the original
  timezone and UTC instant. Native light/dark and ordinary/narrow layouts pass
  [focused acceptance](evidence/P08-native-reminders.md#time-details-and-rescheduling).
- [x] Announce new inbox mail through the notification portal while no window is in
  front: one replaced notification per account, revealing only a count unless
  `notifications.showPreview` is set. [Evidence](evidence/P08-new-mail.md); real desktop
  servers remain open.
- [ ] Reconnect notification delivery and action handling; generation fences,
  earliest-due wake, overdue coalescing and restart/resume/clock changes must be safe.
  Use GPUI `Notification` in explicit `InApp` mode for foreground feedback only.
  Its pinned system path submits no actions and is window-bound; retain stable
  backend IDs, portal delivery and Open/Remind again/Dismiss routing for reminders.
  [Portal delivery and actions pass on a private bus](evidence/P08-native-reminders.md);
  action resolution and dismiss/reschedule now run without a window and reject stale
  notification generations; service lifetime and real desktop servers remain open.
- [ ] Verify stable replacement, both crash windows, Open/Remind again/Dismiss,
  denied/disabled notifications, DST/ambiguous times and deleted-message cases.
- [x] Claim single-instance session ownership before UI/backend startup and route
  passive `org.freedesktop.Application.Activate` requests to the existing window.
  Test concurrent forwarding/coalescing, source/demo conflicts, timeout, normal
  close/restart, bus loss and unchanged Seen with the independent store profile
  lock retained. [Bounded native and bus evidence](evidence/P08-instance.md).
- [ ] Complete foreground activation with a fresh caller token on real Wayland
  desktops. The current public GPUI API requests attention from its own seat;
  private Sway retains the other workspace. Do not equate routed Activate with
  successful focus or mark the desktop entry D-Bus activatable yet.
- [x] Prove GPUI's existing native headless attach/detach lifecycle before moving
  backend ownership: failed attachment/retry, two distinct windows in one process,
  retained state and headless event-loop progress, released workspace entities and
  HTML workers. [Bounded native feasibility evidence](evidence/P08-headless.md).
- [x] Move native backend ownership outside the window and implement `--background` and
  window reopening without a second DB writer. Prefer GPUI's existing
  `WindowingRequest::Headless`/`App::request_windowing` lifecycle proved above;
  no hidden GPU window or custom daemon. Explicit Quit stops work. Automatic
  credential access must neither create a collection nor request foreground unlock.
  The automatic sync path now uses an existing unlocked native keyring without
  requesting unlock; private Secret Service and Dovecot tests cover this path.
  The store owner, reminder scheduler, sync loop and new-mail notification
  bookkeeping have moved into retained application state. Private lifecycle tests
  cover the same owner across windows and synthetic Dovecot synchronization with
  no window. Native close, launcher reopening, new-mail notification opening and
  explicit Quit pass a private end-to-end test. A private native walkthrough
  also verifies reminder delivery after close,
  rejection of a stale action, and opening only the intended message on the
  current action. Native `--background` starts without a window, and repeated
  background launches are passive; private sync, reopening, disabled setting and
  missing-display checks pass. Real desktop acceptance remains open.
  [Evidence](evidence/P08-headless.md).
  Sandboxed file-backed OAuth cannot refresh automatically until non-creating
  token rotation is supported; a cached foreground grant may still be used.
- [ ] Implement independent background/autostart/notification settings and zero
  hidden work when disabled; unload renderer/models while headless when enabled.
  Reuse `SettingField::switch` and ownership/disabled-state text for each setting;
  a toggle cannot grant portal permission or mutate HM-managed systemd services.
  The native Nix-backed background switch passes save/restart/export tests;
  `programs.mailstronaut.startOnLogin` independently installs and removes the
  native graphical-session user service. Sandbox permission and startup remain open.
- [ ] Use the standard Background portal for Flatpak permission/revocation and
  Home Manager-managed native service semantics, without host-unit installation
  from the sandbox. The generated native unit passes [disposable Home Manager
  activation](evidence/P10-home-manager-activation.md#opt-in-native-login-startup).
  Test close, suspend, login/restart and missing display.

No delivery guarantee is made while powered off or logged out.

## P09 — On-device priority and contextual deadlines

Read CLASSIFIER.md. Use existing small CPU inference/date libraries, not a custom
transformer, hosted model or summarization service.
Use the [priority/model component mapping](INTERFACE.md#p09-priority-deadlines-and-models).

- [x] Implement versioned evidence/result types, pure eligibility/expiry and
  user-dismissed/read-cleared state that late analysis cannot override.
  [Rules, store and native acceptance](evidence/P09-priority-rules.md).
- [ ] Run bounded local analysis for newly received readable inbox mail independently
  of opening; explicit opt-in backlog, CPU/memory budgets and unload/cancellation.
  [The rules run this way](evidence/P09-priority-rules.md); no application model
  worker exists yet. Research loading costs are measured separately; idle/unload,
  backlog consent and combined budgets remain open. Unread mail up to 30 days old
  is analyzed without asking only by the cheap rules.
- [ ] Select a pinned/hashed/licensed lightweight EN/FR/IT model after reference
  logits, held-out quality, confidence and resource measurements. Abstain on
  unreadable/encrypted/unsupported/ambiguous input; treat model text as untrusted.
  The [frozen API/reference screen](../experiments/p09-priority/NLI-SCREEN.md)
  pins the planned six-layer candidate and nine numerical test pairs before
  evaluation. Full-logit/token/padding parity and isolated safe-loading memory now
  pass, retaining the first over-budget result. This is not the required 180-message
  priority-quality, application lifecycle or combined-model acceptance. The
  [grouped quality corpus and policy](../experiments/p09-priority/QUALITY-SCREEN.md)
  are frozen before classification: 180 original messages plus 18 held-out semantic
  challenges, with 108 development and 90 held-out messages. The final p2 policy
  **failed** held-out quality/benefit: five extra false positives, lower macro F1,
  and 12/14 accepted-deadline precision per language. Do not integrate this
  rejected candidate. [Full quality decision](evidence/P09-model-quality.md).
- [ ] Extract actionable date evidence using existing date parsing and exact spans;
  distinguish deadlines from quoted dates, meetings, negation and marketing.
  [The bounded rule implementation passes its original cases](evidence/P09-priority-rules.md),
  and [policy 3 excludes inline examples and explicitly superseded clauses](evidence/P09-context.md).
  The semantic challenges still expose general recipient attribution limits.
  Keep this open until those cases have independent acceptance.
- [x] Expire date-only deadlines at exclusive next **local calendar midnight**;
  explicit times expire at their instant. "Due tomorrow" remains today/tomorrow,
  never the following day. Past obligations are never newly prioritized.
  [Rules evidence](evidence/P09-priority-rules.md); a policy-2 regression fixes an
  expired request incorrectly receiving a fresh no-deadline TTL, retaining
  mixed future obligations. [Fix and native evidence](evidence/P09-expired-requests.md).
- [x] Choose bounded contextual 24h/3d/7d TTLs without accepted deadlines, anchored
  to receipt and never sliding on restart/resync; persist reason/time/version.
  [24 and 72 hours](evidence/P09-priority-rules.md); seven days is deliberately not
  selected, as CLASSIFIER.md requires without a reliable long-lived context detector.
- [x] Remove a priority original immediately on read/dismiss, and do not resurrect
  it via unread toggling or stale inference. A newly eligible thread member may
  reprioritize that conversation; reading one member does not read all members.
  [Evidence](evidence/P09-priority-rules.md).
- [x] Present priority with `List` sections/headers, `Label`, `Icon`, `Badge::count`
  and semantic theme surfaces, stable ordering and a Show all button. Keep normal
  inbox access without a second custom list engine or independently scrolling inbox.
  [List sections, ordering and Show all pass](evidence/P09-priority-rules.md); the group
  shows at most 20 rows until Show all lists the rest in the same list.
- [ ] Use a small `GroupBox` + `Label`/`Icon` for deadline boxes beneath extracted
  previews; show evidence/timezone/assumptions through a clicked `Popover` or reader
  `DescriptionList`. Use `Dialog`/`Form`/`DatePicker` and Dismiss `Button` for corrections,
  retaining backend persistence/expiry; evidence must not require hover alone.
  [A `Tag` below the preview, Dismiss, and evidence in the reader
  pass](evidence/P09-priority-rules.md), since rows must share one height.
  The [deadline correction walkthrough](evidence/P09-deadline-correction.md)
  covers both date and time editing, cancel/save, timezone and restart.
  Screen-reader acceptance remains open.
- [x] Expose priority rules and reserved local model directory/revision using Kit
  Settings switch/input fields, with native Nix Save/Discard/export ownership.
  Clearly distinguish local rules from unfinished model inference.
  [Native Save and synthetic appearance evidence](evidence/P06-ui-feedback.md).
- [ ] Use Settings `SettingField::switch`, model metadata `DescriptionList`, an
  explicit Import `Button`/portal and `Alert` for absent/unsupported local models.
  Show `Progress` only for measured work; add no model marketplace or dashboards.
- [x] Test INTERNALDATE vs Date, delayed delivery, DST, multiple dates, quote tails,
  unsupported languages, stale read races, malicious prompts and missing models.
  [Evidence](evidence/P09-priority-rules.md). No model exists yet, so these establish
  the rules alone, which is also what a missing model falls back to.
- [x] Keep previews extracted with source/body revision; support a future generated
  summary type without implementing or implying summarization now.
  [MIME provenance and cache behavior](evidence/P09-priority-rules.md#extracted-preview-provenance).

## P10 — Nix/Home Manager, Flatpak and CI

- [x] Keep one maintainable flake, shared native inputs and standard Crane caching;
  prove dependency reuse after a source-only edit. Debug/test use Cranelift/Wild;
  release uses LLVM/thin LTO/one codegen unit. Retire obsolete native-browser hooks.
  [Local source-only reuse and hosted build evidence](evidence/P01-ci-cache.md).
- [x] Build/install through `nix develop`, `nix flake check`, `nix build`, `nix run`
  and direct Home Manager package/config composition. No custom HM format adapter.
  [Local x86_64 package and installed CLI](evidence/P10-home-manager-activation.md#local-nixos-recheck).
- [x] Provide the direct native-file HM recipe and an optional thin
  `programs.mailstronaut` module with `enable`, `package`, native `settings` and
  `mutableSettings = false` by default. Share Rust evaluation/validation/serialization
  for mutable reconciliation/export; no duplicate per-field HM schema or hidden
  preference database. Test generated activation against the installed package
  with disposable, explicitly stubbed HM plumbing. [Evidence](evidence/P02-native-nix.md).
- [x] Verify full Home Manager evaluation/generation activation (without stubbed
  plumbing), immutable links, mutable first/repeated activation, declared-value
  precedence, retained UI edits, explicit empty account lists, invalid/concurrent
  sources and changing ownership without silent overwrite. Document export/import
  and why removing a declaration is different from deleting an account's local data.
  [Real activation in a disposable NixOS home passed all listed cases, including
  competing writers and CLI export/reimport](evidence/P10-home-manager-activation.md).
  [Configuration redeclaration and backup](MAINTENANCE.md); GUI round trip and live
  desktop autostart remain separate acceptance.
- [ ] Verify actual native Wayland launch, assets/fonts/icons, TLS roots, portals,
  keyring, notifications and GPU loader behavior on NixOS.
- [x] Generate production Flatpak archives from the exact Nix package source and
  vendor inputs, check all lockfiles, normalize archive metadata, reject stale pins
  by default and expose an explicit maintainer refresh. Keep only compressed inputs
  after preparation and reuse the optional `nix develop .#flatpak` tool environment.
  [Workflow](../flatpak/README.md); reproducible hashes and worker-count checks pass.
- [x] Build a pinned offline x86_64 Flatpak with matching SDK/Rust inputs and native
  Wayland access. No X11/XWayland requirement, broad home access, host Nix daemon,
  plaintext secrets or uncontrolled model downloads.
  [The production manifest builds offline and passes the private Wayland, portal
  and attachment checks on the local machine](evidence/P10-flatpak.md#local-sdk-and-installed-runtime-recheck).
  Real desktop portals, accelerated graphics and store listing remain release checks.
- [ ] Verify Flatpak config-file/model import, keyring/file/notification/Background
  portals, read-only managed configuration and packaged HTML resources.
  Reuse the P02 Settings/source/export and P05 import components; do not create a
  second sandbox-specific settings UI. Packaging and portal policy remain backend work.
  [The installed sandbox reaches FileChooser/Notification and completes a synthetic
  Secret-keyring and chooser/attachment flow](evidence/P10-flatpak.md). Real desktop
  portal/document grants, model import, notification actions and the Background
  portal remain open.
- [x] Run format/strict release Clippy, backend/MIME/renderer tests, private
  credential fixtures and installed CLI/config checks in the official minimal
  Nix container on x86_64. [Hosted evidence](evidence/P01-ci-cache.md).
- [ ] Add full privacy-matrix and headless native Wayland current-stack acceptance
  to hosted CI. Local private-window evidence does not complete this row.
- [ ] Confirm hosted CI on the actual release source and distinguish local package,
  GUI, security, accessibility, live-provider and distribution evidence.
- [ ] Include licenses/notices/reproducible sources and explicit optional model
  provisioning. Do not publish/sign or submit provider/Flathub forms without consent.

## P11 — v0.1 completion audit

- [ ] Every mandatory P00–P10 row has current, scope-matched evidence.
- [ ] All five account configurations work through their supported auth routes,
  actual login/sync/reconnect/flags and authorized controlled sends.
- [ ] Unified inbox, threads, offline browsing, strong search, drafts, attachments,
  outbox, reminders and background behavior survive expected errors/restarts.
- [ ] The GPUI Kit interface is visually consistent, keyboard/accessibility tested,
  responsive at measured mailbox scale and genuinely native Wayland.
  Use `Kbd::binding_for_action` and Tooltip for real bound shortcuts, standard
  Toolbar/Menu/Dialog/Select/List focus behavior, and `FocusTrapElement::focus_trap`
  only for a genuinely custom overlay. Preserve real keyboard/AT-SPI/screen-reader
  acceptance; component availability/aria source hooks do not check off that proof.
- [ ] Review an actual daily-mail walkthrough against the supplied references:
  find an account/folder, scan unread/priority/deadline rows, search/open the matching
  original, read a thread, reply/attach/save/send, set a reminder and recover from
  offline/auth errors. Compare usability and code complexity, not visual similarity;
  add no unrelated features from the references. Show current app screenshots in chat.
- [x] Verify the HM-first settings round trip: declare, activate, change an allowed
  mutable preference, restart, export to Nix, import into HM and activate again.
  Managed configuration and secrets must retain their stated ownership.
  The [installed release and private keyring](evidence/P02-preferences.md#home-manager-ownership-transitions)
  passed all four generations and the byte-for-byte GUI export comparison.
- [ ] Ordinary HTML emails render reliably within documented Blitz limitations,
  remain network-silent by default and cannot execute active/native content or
  access local resources/other messages/credentials.
- [ ] Local priority/read/deadline/expiry and contextual reminder cases pass;
  model limits, accuracy/languages, cache coverage and unsupported crypto are clear.
- [ ] Nix/Home Manager and Flatpak installation/runtime pass; hosted CI is green.
- [x] Document backup, upgrade and rollback, account removal and cache limitations,
  Thunderbird migration caveats and profile privacy. [Profile recovery](MAINTENANCE.md)
- [ ] Obtain publishing and signing authorization for the release artifacts.

## Implementation sequence and traceability

Finish P01 first, then reconnect P02/P06 to the retained backend, restore P07/P08
interfaces and complete sync/auth/search/classifier/delivery acceptance. Read this
file before choosing the next slice; do not infer progress from old screenshots.

| Requirement | Current contract/gate |
| --- | --- |
| Rust, simple existing components, workspace | ARCHITECTURE/DEPENDENCIES; P00/P01/P03 |
| GPUI Kit + Blitz, native Wayland, reusable components, daily usability | INTERFACE/RENDERING; P01/P06/P07/P11 |
| Native Nix config, HM mutable settings and export to Nix | CONFIGURATION; P02/P10/P11 |
| Five providers | P04/P05/P11 |
| Unified inbox and threads | P03/P06 |
| Explicit-open-only Seen | P06/P07/P09 |
| Fuzzy and multilingual meaning search | SEARCH; P06/P11 |
| Durable compose/attachments/outbox | P03/P07 |
| Remind me and optional work after close | P08 |
| Local priority, contextual expiry, deadline boxes | CLASSIFIER; P09 |
| Nix/Flatpak, minimal Nix-container CI | DELIVERY; P10/P11 |
