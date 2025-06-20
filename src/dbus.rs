use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{Local, TimeDelta};
use derive_more::Debug;
use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream, StreamExt};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};
use zbus::{connection, interface};

use crate::freedesktop::{find_app_name, find_icon_path, tmp_image_from_data};
use crate::markup::markup;
use crate::notification::{Notification, Urgency};

pub fn dbus() -> impl Stream<Item = DbusMessage> {
    iced::stream::channel(100, async move |mut output| {
        // Create the channel to communicate with the GUI
        let (sender, mut receiver) = mpsc::channel(100);

        // Create the NotificationInterface and connect to the DBUS
        let interface = NotificationInterface::new(output.clone());
        let dbus_connection = connection::Builder::session()
            .expect("Unable to connect to session bus")
            .name("org.freedesktop.Notifications")
            .expect("Unable to register name")
            .serve_at("/org/freedesktop/Notifications", interface)
            .expect("Unable to register interface at path")
            .build()
            .await
            .expect("Unable to connect");

        // Get the Signal Emitter to send signals
        let interface_ref = dbus_connection
            .object_server()
            .interface::<_, NotificationInterface>("/org/freedesktop/Notifications")
            .await
            .expect("Unable to get interface");

        // Let the GUI know the DBUS interface is initialized
        let _ = output
            .send(DbusMessage::Initialized(NotificationSignaller(sender)))
            .await;

        tokio::task::spawn(async move {
            let signal_emitter = interface_ref.signal_emitter();
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));

            loop {
                interval.tick().await;
                let Some(message) = receiver.next().await else {
                    continue;
                };

                match message {
                    DbusSignal::NotificationClosed(id, reason) => {
                        NotificationInterface::notification_closed(
                            signal_emitter,
                            id,
                            reason.into(),
                        )
                        .await
                        .expect("Unable to send notification closed signal")
                    }
                    DbusSignal::ActionInvoked(id, key) => {
                        NotificationInterface::action_invoked(signal_emitter, id, key)
                            .await
                            .expect("Unable to send action invoked signal")
                    }
                }
            }
        })
        .await
        .expect("Unable to send DBUS signals from GUI");
    })
}

#[derive(Debug, Clone)]
pub struct NotificationSignaller(mpsc::Sender<DbusSignal>);
impl NotificationSignaller {
    pub fn close_notification(&mut self, id: u32, reason: NotificationClosedReason) {
        self.0
            .try_send(DbusSignal::NotificationClosed(id, reason))
            .expect("Unable to send NotificationClosed signal message")
    }

    pub fn action_invoked(&mut self, id: u32, key: String) {
        self.0
            .try_send(DbusSignal::ActionInvoked(id, key))
            .expect("Unable to send ActionInvoked signal message")
    }
}

#[derive(Debug, Clone)]
pub enum DbusSignal {
    NotificationClosed(u32, NotificationClosedReason),
    ActionInvoked(u32, String),
}

impl From<NotificationClosedReason> for u32 {
    fn from(reason: NotificationClosedReason) -> Self {
        match reason {
            NotificationClosedReason::Expired => 1,
            NotificationClosedReason::DismissedByUser => 2,
            NotificationClosedReason::ClosedByCloseNotification => 3,
            NotificationClosedReason::Undefined => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NotificationClosedReason {
    Expired,
    DismissedByUser,
    ClosedByCloseNotification,
    Undefined,
}

struct NotificationInterface {
    /// The GUI message channel sender
    sender: mpsc::Sender<DbusMessage>,
    /// The next notification id to be used
    next_id: u32,
    /// Set of already used ids
    used_ids: HashSet<u32>,
    /// The path to the default icon
    default_icon: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) enum DbusMessage {
    Initialized(NotificationSignaller),
    Notify(Notification),
    CloseNotification(u32),
}

impl NotificationInterface {
    /// Construct a new NotificationInterface
    fn new(sender: mpsc::Sender<DbusMessage>) -> Self {
        Self {
            sender,
            next_id: 1,
            used_ids: HashSet::new(),
            default_icon: find_icon_path("notifications").expect("Unable to find default icon"),
        }
    }

    /// Get the next available id
    fn get_next_id(&mut self) -> u32 {
        while self.used_ids.contains(&self.next_id) {
            if self.next_id == u32::MAX {
                self.next_id = 1;
            } else {
                self.next_id += 1;
            }
        }
        self.used_ids.insert(self.next_id);
        self.next_id
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationInterface {
    fn get_capabilities(&self) -> Vec<String> {
        info!("GetCapabilities called");
        vec![
            "actions".into(),
            "body".into(),
            "body-markup".into(),
            "body-images".into(),
            "persistence".into(),
        ]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        info!("GetServerInformation called");
        (
            "Baelyk's Notification Server".into(),
            "Baelyk".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
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

        let body = if body.is_empty() {
            None
        } else {
            Some(markup(body))
        };

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
            .and_then(|entry| find_app_name(entry))
            .unwrap_or(app_name.clone());

        let icon = hints
            .image_data
            .as_ref()
            .and_then(tmp_image_from_data)
            .or_else(|| hints.image_path.clone())
            .or_else(|| find_icon_path(&app_icon))
            .or_else(|| hints.icon_data.as_ref().and_then(tmp_image_from_data))
            .unwrap_or(self.default_icon.clone());

        let actions: Vec<(String, String)> = actions
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        let actions = if actions.is_empty() {
            None
        } else {
            Some(actions)
        };

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

        // Inform the GUI of the new notification
        self.sender
            .send(DbusMessage::Notify(notification))
            .await
            .expect("Unable to send message to GUI");

        id
    }

    async fn close_notification(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        id: u32,
    ) {
        info!("CloseNotification called for {id}");
        self.sender
            .send(DbusMessage::CloseNotification(id))
            .await
            .expect("Unable to send message to GUI");
        // 3 means the notification was closed by a call to CloseNotification
        emitter
            .notification_closed(id, 3)
            .await
            .expect("Failed to send notification closed signal");
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        string_key: String,
    ) -> zbus::Result<()>;
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
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    rowstride: i32,
    pub has_alpha: bool,
    bits_per_sample: i32,
    channels: i32,
    #[debug("Vec[{}]", data.len())]
    pub data: Vec<u8>,
}
