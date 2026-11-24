//! A file chooser and notification server on a private bus, so that native acceptance can
//! drive the real Save, Attach and notification paths in a headless session. It speaks the
//! desktop portal's protocol, and is not a claim about any real portal.

use std::{collections::HashMap, fs::OpenOptions, io::Write as _, path::PathBuf, time::Duration};

use zbus::{
    Connection, fdo,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

/// What the next dialog answers comes from a file the harness writes: `cancel`, `deny`, or
/// one file URI per line. Every request is recorded, including any suggested name.
struct Chooser(PathBuf, tokio::runtime::Handle);

impl Chooser {
    fn answer(
        &self,
        method: &str,
        options: &HashMap<String, OwnedValue>,
        header: &Header<'_>,
        connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        let suggested = options
            .get("current_name")
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or_default();
        writeln!(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.0.join("chooser-requests.jsonl"))
                .map_err(|_| fdo::Error::Failed("fixture receipt".into()))?,
            "{}",
            serde_json::json!({"method":method,"suggested":suggested})
        )
        .map_err(|_| fdo::Error::Failed("fixture receipt".into()))?;
        let token = options
            .get("handle_token")
            .and_then(|value| <&str>::try_from(value).ok())
            .ok_or_else(|| fdo::Error::InvalidArgs("missing request handle".into()))?;
        let sender = header
            .sender()
            .ok_or_else(|| fdo::Error::Failed("missing sender".into()))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .map_err(|_| fdo::Error::InvalidArgs("request path".into()))?;
        let planned = std::fs::read_to_string(self.0.join("chooser-answer")).unwrap_or_default();
        let (response, uris): (u32, Vec<String>) = match planned.trim() {
            "cancel" => (1, Vec::new()),
            "deny" | "" => (2, Vec::new()),
            lines => (0, lines.lines().map(str::to_owned).collect()),
        };
        let mut results = HashMap::new();
        if response == 0 {
            let uris = Value::from(uris)
                .try_to_owned()
                .map_err(|_| fdo::Error::Failed("fixture answer".into()))?;
            results.insert("uris".to_owned(), uris);
        }
        let (connection, response_path) = (connection.clone(), path.clone());
        // The client subscribes to the request object after it receives this reply.
        self.1.spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            connection
                .emit_signal(
                    None::<&str>,
                    response_path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(response, results),
                )
                .await
                .expect("synthetic response");
        });
        Ok(path)
    }
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl Chooser {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "zbus requires a non-const instance property getter."
    )]
    fn version(&self) -> u32 {
        4
    }

    #[zbus(name = "OpenFile")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "D-Bus supplies owned decoded options and the message header."
    )]
    fn open_file(
        &self,
        parent: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        _ = (parent, title);
        self.answer("OpenFile", &options, &header, connection)
    }

    #[zbus(name = "SaveFile")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "D-Bus supplies owned decoded options and the message header."
    )]
    fn save_file(
        &self,
        parent: &str,
        title: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        _ = (parent, title);
        self.answer("SaveFile", &options, &header, connection)
    }
}

/// Examines the actual descriptor from OpenURI.OpenFile on the synthetic session bus.
struct FileOpener(Chooser);

#[zbus::interface(name = "org.freedesktop.portal.OpenURI")]
impl FileOpener {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "zbus requires an instance property getter."
    )]
    fn version(&self) -> u32 {
        5
    }

    #[zbus(name = "OpenFile")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "The D-Bus header and options are decoded by value."
    )]
    fn open_file(
        &self,
        parent: &str,
        fd: zbus::zvariant::OwnedFd,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        use std::io::Read as _;
        let _ = parent;
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        if !options
            .get("ask")
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
            || options.contains_key("writable")
            || options.contains_key("writeable")
            || file.write(b"x").is_ok()
        {
            return Err(fdo::Error::Failed(
                "Expected a read-only descriptor and an application chooser".into(),
            ));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| fdo::Error::Failed("Read synthetic descriptor".into()))?;
        std::fs::write(self.0.0.join("opened-bytes"), bytes)
            .map_err(|_| fdo::Error::Failed("Record synthetic bytes".into()))?;
        self.0
            .answer("OpenAttachment", &options, &header, connection)
    }
}

/// Records what the application asks the desktop to show.
struct Notifier(PathBuf);

impl Notifier {
    fn record(&self, entry: &serde_json::Value) -> fdo::Result<()> {
        writeln!(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.0.join("notifications.jsonl"))
                .map_err(|_| fdo::Error::Failed("fixture receipt".into()))?,
            "{entry}"
        )
        .map_err(|_| fdo::Error::Failed("fixture receipt".into()))
    }
}

#[zbus::interface(name = "org.freedesktop.portal.Notification")]
impl Notifier {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "zbus requires a non-const instance property getter."
    )]
    fn version(&self) -> u32 {
        1
    }

    #[zbus(name = "AddNotification")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus deserializes an owned notification dictionary."
    )]
    fn add(&self, id: &str, notification: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        let text = |key: &str| {
            notification
                .get(key)
                .and_then(|value| <&str>::try_from(value).ok())
                .unwrap_or_default()
        };
        self.record(&serde_json::json!({
            "add": id,
            "title": text("title"),
            "body": text("body"),
            "action": text("default-action"),
        }))
    }

    #[zbus(name = "RemoveNotification")]
    fn remove(&self, id: &str) -> fdo::Result<()> {
        self.record(&serde_json::json!({"remove": id}))
    }
}

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

/// The application secret a sandboxed client reads to open its encrypted keyring. The harness writes
/// the synthetic bytes to `portal-secret`; the fixture never invents a secret.
struct Secrets(PathBuf, tokio::runtime::Handle);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the private bus from experiments/p02-accounts/credentials.sh"]
async fn private_secret_portal_response() {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT").expect("fixture root"));
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
    let fixture = root.join("synthetic-secret-portal");
    std::fs::create_dir_all(&fixture).expect("fixture directory");
    let expected = b"SYNTHETIC-PORTAL-SECRET-NOT-REAL";
    std::fs::write(fixture.join("portal-secret"), expected).expect("synthetic secret");
    let connection = zbus::connection::Builder::session()
        .expect("private bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at(
            PORTAL_PATH,
            Secrets(fixture, tokio::runtime::Handle::current()),
        )
        .expect("secret portal")
        .build()
        .await
        .expect("private portal");
    let actual = tokio::time::timeout(Duration::from_secs(5), ashpd::desktop::secret::retrieve())
        .await
        .expect("bounded portal response")
        .expect("secret retrieval");
    assert_eq!(actual, expected);
    connection.close().await.expect("close portal");
}

#[zbus::interface(name = "org.freedesktop.portal.Secret")]
impl Secrets {
    #[zbus(property, name = "version")]
    #[expect(
        clippy::unused_self,
        reason = "zbus requires a non-const instance property getter."
    )]
    fn version(&self) -> u32 {
        1
    }

    #[zbus(name = "RetrieveSecret")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "D-Bus supplies owned options and the message header."
    )]
    fn retrieve_secret(
        &self,
        fd: zbus::zvariant::OwnedFd,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        let secret = std::fs::read(self.0.join("portal-secret"))
            .map_err(|_| fdo::Error::Failed("fixture secret".into()))?;
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
        file.write_all(&secret)
            .map_err(|_| fdo::Error::Failed("fixture secret".into()))?;
        // Answer like the portal: a request object that later signals its response.
        let token = options
            .get("handle_token")
            .and_then(|value| <&str>::try_from(value).ok())
            .ok_or_else(|| fdo::Error::InvalidArgs("missing request handle".into()))?;
        let sender = header
            .sender()
            .ok_or_else(|| fdo::Error::Failed("missing sender".into()))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .map_err(|_| fdo::Error::InvalidArgs("request path".into()))?;
        let (connection, response_path) = (connection.clone(), path.clone());
        self.1.spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let results: HashMap<String, OwnedValue> = HashMap::new();
            connection
                .emit_signal(
                    None::<&str>,
                    response_path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0u32, results),
                )
                .await
                .expect("synthetic secret response");
        });
        Ok(path)
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "explicit private-bus GUI harness only; runs until the harness stops it"]
async fn serve_portals() {
    let root =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_PORTAL_ROOT").expect("private fixture root"));
    assert!(root.is_absolute());
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .expect("private bus")
            .contains(root.parent().expect("profile").to_str().expect("path")),
        "this must never answer on a real session bus"
    );
    let connection = zbus::connection::Builder::session()
        .expect("private bus")
        .name("org.freedesktop.portal.Desktop")
        .expect("portal name")
        .serve_at(
            PORTAL_PATH,
            Chooser(root.clone(), tokio::runtime::Handle::current()),
        )
        .expect("file chooser")
        .serve_at(
            PORTAL_PATH,
            FileOpener(Chooser(root.clone(), tokio::runtime::Handle::current())),
        )
        .expect("attachment opener")
        .serve_at(PORTAL_PATH, Notifier(root.clone()))
        .expect("notifications")
        .serve_at(
            PORTAL_PATH,
            Secrets(root.clone(), tokio::runtime::Handle::current()),
        )
        .expect("secrets")
        .build()
        .await
        .expect("private portal");
    std::fs::write(root.join("portal-ready"), "").expect("rendezvous");
    // The harness clicks a notification by writing its ID and action to a file.
    let invoke = root.join("notification-invoke");
    loop {
        if let Ok(text) = std::fs::read_to_string(&invoke) {
            let mut fields = text.split_whitespace();
            let (Some(id), Some(action)) = (fields.next(), fields.next()) else {
                std::fs::remove_file(&invoke).expect("discard invalid request");
                continue;
            };
            let arguments = fields
                .next()
                .and_then(|value| value.parse::<i64>().ok())
                .map_or_else(Vec::new, |generation| vec![Value::from(generation)]);
            std::fs::remove_file(&invoke).expect("consumed");
            connection
                .emit_signal(
                    None::<&str>,
                    PORTAL_PATH,
                    "org.freedesktop.portal.Notification",
                    "ActionInvoked",
                    &(id, action, arguments),
                )
                .await
                .expect("synthetic click");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
