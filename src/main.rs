use chrono::{DateTime, Local, TimeDelta};
use clap::Parser;
use derive_more::Debug;
use freedesktop_desktop_entry::{default_paths, get_languages_from_env, Iter};
use image::{RgbImage, RgbaImage};
use log::{debug, error, info, trace, warn};
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use zbus::object_server::SignalContext;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};
use zbus::{connection, interface, proxy};

/// A notification server using Eww to display notifications
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Log level: can be Off, Error, Warn, Info, Debug, or Trace
    #[arg(long, default_value_t = log::LevelFilter::Debug)]
    log: log::LevelFilter,

    /// Freedesktop Icon Theme name
    #[arg(long, default_value = "Gruvbox-Plus-Dark")]
    theme: String,
}

fn setup_logger(log_level: log::LevelFilter) -> Result<(), fern::InitError> {
    // Log to stderr and ~/.local/state/baelyks-notification-server.log
    let log_path = dirs::home_dir()
        .expect("Unable to get the home dir")
        .join(".local/state/")
        .join(env!("CARGO_PKG_NAME"))
        .with_extension("log");

    fern::Dispatch::new()
        .format(|out, message, record| out.finish(format_args!("[{}] {}", record.level(), message)))
        .level(log_level)
        .chain(std::io::stderr())
        .chain(
            fern::Dispatch::new()
                .format(|out, message, _| {
                    out.finish(format_args!(
                        "[{}] {}",
                        Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        message
                    ))
                })
                .chain(fern::log_file(&log_path)?),
        )
        .apply()?;

    info!(
        "Starting {} v{} with log level: {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        log_level
    );

    Ok(())
}

fn escape_string(string: &str) -> String {
    [("'", "\\'"), ("\"", "\\\""), ("\\", "\\\\")]
        .iter()
        .fold(string.to_string(), |string, &(from, to)| {
            string.replace(from, to)
        })
}

fn serialize_notification_time<S>(time: &DateTime<Local>, serialize: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let since = Local::now().signed_duration_since(time);

    let string: &str = if since.num_seconds() < 30 {
        "now"
    } else if since.num_seconds() < 60 {
        "30s ago"
    } else if since.num_minutes() < 60 {
        &format!("{}m ago", since.num_minutes())
    } else if since.num_hours() < 4 {
        &format!("{}h ago", since.num_hours())
    } else if since.num_days() < 1 {
        &time.format("%H:%S").to_string()
    } else if since.num_weeks() < 1 {
        &time.format("%a %H:%M").to_string()
    } else {
        &time.format("%a %h %e").to_string()
    };

    serialize.serialize_str(&string)
}

fn find_app_name(desktop_entry_name: &String) -> Option<String> {
    let locales = get_languages_from_env();
    let mut entries = Iter::new(default_paths()).entries(Some(&locales));

    let desktop_entry_name = desktop_entry_name.to_lowercase();
    if let Some(desktop_entry) =
        entries.find(|desktop_entry| desktop_entry.appid.to_lowercase() == desktop_entry_name)
    {
        if let Some(name) = desktop_entry.name(&locales) {
            return Some(name.into_owned());
        } else {
            debug!("No name found for {}", desktop_entry_name);
        }
    } else {
        debug!("No desktop entry found for {}", desktop_entry_name);
    }

    None
}

fn tmp_path() -> Option<PathBuf> {
    let mut tries = 0;
    while tries < 3 {
        tries += 1;

        let filename: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();
        let path = PathBuf::from(format!("/tmp/{}.png", filename));

        if path.try_exists().is_ok_and(|exists| !exists) {
            return Some(path);
        }
    }

    warn!("Unable to generate a temporary path");
    None
}

fn tmp_image_from_data(image_data: &ImageData) -> Option<PathBuf> {
    // Generate a path in the /tmp directory
    let Some(path) = tmp_path() else {
        return None;
    };

    // Create and save the image
    let save_result = if image_data.has_alpha {
        let Some(image) = RgbaImage::from_raw(
            image_data.width as u32,
            image_data.height as u32,
            image_data.data.clone(),
        ) else {
            warn!("Failed to create RGBA image");
            return None;
        };
        image.save(&path)
    } else {
        let Some(image) = RgbImage::from_raw(
            image_data.width as u32,
            image_data.height as u32,
            image_data.data.clone(),
        ) else {
            warn!("Failed to create RGB image");
            return None;
        };
        image.save(&path)
    };

    if let Err(err) = save_result {
        warn!(
            "Failed to save image to {} with error {}",
            path.display(),
            err
        );
        return None;
    };

    Some(path)
}

/// Gets a path for an icon by first checking if the passed icon is a path that
/// exists, and if not, searches for a matching freedesktop icon.
fn find_icon_path(icon_name_or_path: &str, theme: &str) -> Option<PathBuf> {
    trace!("Checking path {icon_name_or_path}");
    // Paths are supposed to be prepended with "file://" but in practice many are not
    let path: PathBuf = icon_name_or_path.replace("file://", "").into();
    if path.exists() {
        return Some(path);
    }

    trace!("Looking for icon {icon_name_or_path}");
    freedesktop_icons::lookup(icon_name_or_path)
        .with_cache()
        .force_svg()
        .with_theme(theme)
        .find()
        .or(freedesktop_icons::lookup(icon_name_or_path)
            .with_cache()
            .with_size(100)
            .with_theme(theme)
            .find())
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Urgency {
    Low,
    Normal,
    Critical,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    /// Unique ID for the notification
    id: u32,
    /// The time the notification was sent
    #[serde(serialize_with = "serialize_notification_time")]
    time: DateTime<Local>,
    /// The time the notification will expire
    #[serde(skip)]
    expire_time: Option<DateTime<Local>>,
    /// The display name for the application
    name: String,
    /// The resolved path for the icon/image to display
    icon: Option<PathBuf>,
    /// The application provided summary
    summary: String,
    /// The application provided body
    body: String,
    /// The list of actions over two elements: key, label
    actions: Vec<(String, String)>,
    /// The DBUS supplied urgency, defaulting to Normal
    urgency: Urgency,
}

impl Notification {
    //fn ewwify(&self) -> String {
    //let close_cmd = format!("dbus-send --type=method_call --dest=org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications.EwwCloseNotification uint32:{}", self.id);

    //let icon = self
    //.icon
    //.as_ref()
    //.and_then(|path| path.to_str())
    //.unwrap_or("");
    //let time: String = unimplemented!(); // serialize_notification_time(&self.time);

    //let mut button_box = String::from(
    //"(box :class \"buttons\"
    //:space-evenly true
    //:spacing 5",
    //);
    //let buttons: String = self.actions.chunks_exact(2).map(|window| {
    //let key: &String = &escape_string(&window[0]);
    //let text: &String = &escape_string(&window[1]);
    //format!("(button :onclick \"dbus-send --type=method_call --dest=org.freedesktop.Notifications /org/freedesktop/Notifications org.freedesktop.Notifications.EwwActionInvoked uint32:{} string:{}\" (label :text \"{}\"))", self.id, key, text)
    //}).collect();
    //button_box.push_str(&buttons);
    //button_box.push_str(")");

    //format!(
    //"(notification :close_cmd \"{}\"
    //:icon '{}'
    //:app_name \"{}\"
    //:time \"{}\"
    //:summary \"{}\"
    //:body \"{}\"
    //:buttons \'{}\')",
    //close_cmd, icon, self.name, time, self.summary, self.body, button_box
    //)
    //}
}

#[derive(Debug, DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict", rename_all = "kebab-case")]
struct Hints {
    action_icons: Option<bool>,
    category: Option<String>,
    desktop_entry: Option<String>,
    image_data: Option<ImageData>,
    #[zvariant(rename = "image_data")]
    image_data_deprecated: Option<ImageData>,
    image_path: Option<PathBuf>,
    #[zvariant(rename = "image_path")]
    image_path_deprecated: Option<String>,
    #[zvariant(rename = "icon_data")]
    icon_data: Option<ImageData>,
    resident: Option<bool>,
    sound_file: Option<String>,
    sound_name: Option<String>,
    suppress_sound: Option<bool>,
    x: Option<i32>,
    y: Option<i32>,
    urgency: Option<u8>,
}

#[derive(Debug, Deserialize, Serialize, Type)]
#[zvariant(signature = "(iiibiiay)")]
struct ImageData {
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
    bits_per_sample: i32,
    channels: i32,
    #[debug("Vec[{}]", data.len())]
    data: Vec<u8>,
}

struct Notifications {
    /// The next notification id to be used
    next_id: u32,
    /// Map of all current notifications
    notifications: HashMap<u32, Notification>,
    /// List of notifications (by id) displayed on the screen in order
    alerts: Vec<u32>,
    /// The Freedesktop icon theme
    theme: String,
}

impl Notifications {
    fn new(theme: String) -> Self {
        let next_id = 1;
        let notifications = HashMap::new();
        let alerts = Vec::new();
        Notifications {
            next_id,
            notifications,
            alerts,
            theme,
        }
    }

    /// Get the next available id
    fn get_next_id(&mut self) -> u32 {
        while self.notifications.contains_key(&self.next_id) {
            if self.next_id == u32::MAX {
                self.next_id = 1;
            } else {
                self.next_id += 1;
            }
        }
        self.next_id
    }

    /// Add or update notification to the list of notifications and alerts
    fn add_notification(&mut self, notification: Notification) {
        // Add this notification to alerts if it's not already there
        if self
            .alerts
            .iter()
            .find(|&id| *id == notification.id)
            .is_none()
        {
            self.alerts.push(notification.id);
        }

        // Insert the notification into the map by its id, or update if existing
        self.notifications.insert(notification.id, notification);

        // Update the Eww display
        self.update_eww();
    }

    /// Removes a notification from the list of notifications and alerts
    fn remove_notification(&mut self, id: u32) {
        // Remove the notification from alerts
        if let Some((index, _)) = self
            .alerts
            .iter()
            .enumerate()
            .find(|(_, &alert_id)| alert_id == id)
        {
            self.alerts.remove(index);
        }

        // Remove from notifications list
        self.notifications.remove(&id);

        // Update the Eww display
        self.update_eww();
    }

    //fn ewwify(&self) -> String {
    //let mut eww = String::from(
    //"(box :class \"notifications\"
    //:orientation \"vertical\"
    //:space-evenly false",
    //);

    //self.alerts.iter().for_each(|id| {
    //if let Some(notification) = self.notifications.get(id) {
    //eww.push_str("\n  ");
    //eww.push_str(&notification.ewwify());
    //} else {
    //warn!("Notification with id {} not found", id);
    //}
    //});

    //eww.push_str(")");

    //eww
    //}

    fn update_eww(&self) {
        let alerts: Vec<&Notification> = self
            .alerts
            .iter()
            .filter_map(|id| self.notifications.get(id))
            .collect();
        let pretty_eww = serde_json::to_string_pretty(&alerts).unwrap(); // self.ewwify();
        if let Err(error) = std::fs::write("/tmp/eww-notifs-pretty", &pretty_eww) {
            error!("Error writing to /tmp/eww-notifs.pretty: {}", error);
        }

        let mut one_line_eww = serde_json::to_string(&alerts).unwrap(); // pretty_eww.replace("\n", " ");
        one_line_eww.push('\n');
        if let Err(error) = std::fs::remove_file("/tmp/eww-notifs") {
            error!("Error removing /tmp/eww-notifs: {}", error);
        }
        if let Err(error) = std::fs::write("/tmp/eww-notifs", &one_line_eww) {
            error!("Error writing to /tmp/eww-notifs: {}", error);
            error!("Unable to update Eww!");
        }
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<String> {
        info!("GetCapabilities called");
        vec![
            "actions".into(),
            "body".into(),
            "body-markup".into(),
            "persistence".into(),
        ]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        info!("GetServerInformation called");
        (
            "Baelyk's Notification Server".into(),
            "Baelyk".into(),
            "0.0.0".into(),
            "1.2".into(),
        )
    }

    fn notify(
        &mut self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: Hints,
        expire_timeout: i32,
    ) -> u32 {
        info!("Notify called");

        debug!(
            "Notify parameters:
    app_name: {:?},
    replaces_id: {:?},
    app_icon: {:?},
    summary: {:?},
    body: {:?},
    actions: {:?},
    hints: {:#?},
    expire_timeout: {:?}",
            app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout
        );

        let app_name = escape_string(&app_name);
        let summary = escape_string(&summary);
        let body = escape_string(&body);

        let id = if replaces_id == 0 {
            self.get_next_id()
        } else {
            replaces_id
        };

        let urgency = match hints.urgency {
            None => Urgency::Normal,
            Some(0) => Urgency::Low,
            Some(1) => Urgency::Normal,
            Some(2) => Urgency::Critical,
            Some(level) => {
                warn!("Unexpected urgency level {}", level);
                Urgency::Normal
            }
        };

        let time = Local::now();

        let expire_time = if urgency == Urgency::Critical || expire_timeout == 0 {
            None
        } else if expire_timeout == -1 {
            Some(time + TimeDelta::minutes(1))
        } else {
            Some(time + TimeDelta::milliseconds(expire_timeout as i64))
        };

        let name = hints
            .desktop_entry
            .as_ref()
            .and_then(find_app_name)
            .unwrap_or(app_name.clone());

        let icon = hints
            .image_data
            .as_ref()
            .and_then(tmp_image_from_data)
            .or_else(|| hints.image_path.clone())
            .or_else(|| find_icon_path(&app_icon, &self.theme))
            .or_else(|| hints.icon_data.as_ref().and_then(tmp_image_from_data))
            .or_else(|| find_icon_path("notifications", &self.theme));

        let actions = actions
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();

        let notification = Notification {
            id,
            time,
            expire_time,
            name,
            icon,
            summary,
            body,
            actions,
            urgency,
        };

        debug!("Notification created: {:#?}", notification);

        self.add_notification(notification);

        id
    }

    async fn close_notification(
        &mut self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        id: u32,
    ) {
        info!("CloseNotification called for {id}");
        self.remove_notification(id);
        // 3 means the notification was closed by a call to CloseNotification
        Notifications::notification_closed(&ctxt, id, 3)
            .await
            .expect("Failed to send notification closed signal");
    }

    #[zbus(signal)]
    async fn notification_closed(
        ctxt: &SignalContext<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        ctxt: &SignalContext<'_>,
        id: u32,
        string_key: String,
    ) -> zbus::Result<()>;

    // Below are custom methods for receiving interaction from or updating Eww

    async fn eww_request_update(&mut self, #[zbus(signal_context)] ctxt: SignalContext<'_>) {
        trace!("EwwRequestUpdate called");

        if self.alerts.is_empty() {
            return;
        }

        // Prune expired notifications
        let expired: Vec<u32> = self
            .alerts
            .clone()
            .into_iter()
            .filter(|id| {
                let Some(notification) = self.notifications.get(&id) else {
                    warn!("Notification {id} not found");
                    return false;
                };

                notification
                    .expire_time
                    .is_some_and(|expire_time| Local::now() > expire_time)
            })
            .collect();
        futures::future::try_join_all(expired.into_iter().map(|id| {
            self.remove_notification(id);
            Notifications::notification_closed(&ctxt, id, 1)
        }))
        .await
        .expect("Failed to send notification closed signals");

        self.update_eww();
    }

    async fn eww_close_notification(
        &mut self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        id: u32,
    ) {
        info!("EwwCloseNotification called for {id}");
        self.remove_notification(id);
        // 2 means the notification was dismissed by the user
        Notifications::notification_closed(&ctxt, id, 2)
            .await
            .expect("Failed to send notification closed signal");
    }

    async fn eww_action_invoked(
        &mut self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        id: u32,
        string_key: String,
    ) {
        info!("EwwActionInvoked called {string_key} for {id}");
        Notifications::action_invoked(&ctxt, id, string_key)
            .await
            .expect("Failed to forward action invoked signal");
    }
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    fn eww_request_update(&self) -> zbus::Result<()> {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    setup_logger(args.log)?;

    debug!("Command line arguments: {:#?}", args);

    let notifications = Notifications::new(args.theme.clone());

    let _conn = connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at("/org/freedesktop/Notifications", notifications)?
        .build()
        .await?;

    let updater = tokio::task::spawn(async {
        let conn = connection::Connection::session()
            .await
            .expect("Unable to connect to session bus");
        let proxy = NotificationsProxy::new(&conn)
            .await
            .expect("Unable to create proxy");

        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1000));

        loop {
            interval.tick().await;
            proxy
                .eww_request_update()
                .await
                .expect("Unable to request update");
        }
    });

    updater.await?;

    Ok(())
}
