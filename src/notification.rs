use chrono::{DateTime, Local};
use std::path::PathBuf;

use crate::markup::BodyElement;

#[derive(Clone, Debug)]
pub struct Notification {
    /// Unique ID for the notification
    pub id: u32,
    /// The resolved path for the icon/image to display
    pub icon: PathBuf,
    /// The display name for the application
    pub name: String,
    /// The application provided summary
    pub summary: String,
    /// The application provided body
    pub body: Option<Vec<BodyElement>>,
    /// The time the notification was sent
    pub time: DateTime<Local>,
    /// The time the notification will expire
    pub expire_time: Option<DateTime<Local>>,
    /// The list of actions over two elements: key, label
    pub actions: Option<Vec<(String, String)>>,
    /// The DBUS supplied urgency, defaulting to Normal
    pub urgency: Urgency,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Urgency {
    Low,
    Normal,
    Critical,
}

pub fn notification_time(time: &DateTime<Local>) -> String {
    let since = Local::now().signed_duration_since(time);

    if since.num_seconds() < 30 {
        "now".into()
    } else if since.num_seconds() < 60 {
        "30s ago".into()
    } else if since.num_minutes() < 60 {
        format!("{}m ago", since.num_minutes())
    } else if since.num_hours() < 4 {
        format!("{}h ago", since.num_hours())
    } else if since.num_days() < 1 {
        time.format("%H:%S").to_string()
    } else if since.num_weeks() < 1 {
        time.format("%a %H:%M").to_string()
    } else {
        time.format("%a %h %e").to_string()
    }
}
