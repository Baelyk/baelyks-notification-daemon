use std::collections::HashMap;

use chrono::Local;
use iced::alignment::Horizontal;
use iced::widget::{button, column, container, mouse_area, row, text, Column, Row};
use iced::{window, Background, Border, Element, Length, Subscription, Task, Theme};
use iced_layershell::reexport::{Anchor, NewLayerShellSettings};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::{to_layer_message, MultiApplication};
use log::{debug, trace, warn};

use crate::dbus::{self, DbusMessage, NotificationClosedReason, NotificationSignaller};
use crate::markup::{BodyElement, RichTextSpan};
use crate::measuring_container::MeasuringContainer;
use crate::notification::{notification_time, Notification, Urgency};

pub fn run() -> Result<(), iced_layershell::Error> {
    State::run(Settings {
        layer_settings: LayerShellSettings {
            start_mode: StartMode::Background,
            ..Default::default()
        },
        default_font: iced::Font::with_name("JetBrains Mono"),
        ..Default::default()
    })
}

struct State {
    /// Map of all current notifications
    notifications: HashMap<u32, Notification>,
    /// List of notifications (by id) displayed on the screen in order
    alerts: Vec<u32>,
    /// DBUS signaller
    signaller: SignallerState,
    /// The id of the window, if it exists
    window_id: Option<iced::window::Id>,
}

enum SignallerState {
    Unitialized,
    Initialized(NotificationSignaller),
}

#[to_layer_message(multi)]
#[derive(Clone, Debug)]
enum Message {
    ActionInvoked(u32, String),
    ContainerResized(u32),
    Dbus(DbusMessage),
    Tick,
    UserDismissed(u32),
}

const FONT_SIZE: f32 = 20.0;
const WIDTH: f32 = 500.0;
const ICON_SIZE: f32 = 80.0;
const SMALL: f32 = 10.0;
const BIG: f32 = 20.0;

impl Default for State {
    fn default() -> Self {
        State {
            notifications: HashMap::new(),
            alerts: Vec::new(),
            signaller: SignallerState::Unitialized,
            window_id: None,
        }
    }
}

impl State {
    fn view_notification(&self, notification: &Notification) -> Element<Message> {
        let image: Element<Message> = if notification
            .icon
            .extension()
            .is_some_and(|extension| extension == "svg")
        {
            iced::widget::svg(notification.icon.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            iced::widget::image(notification.icon.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        let icon = container(image).width(ICON_SIZE).height(ICON_SIZE);

        let header = container(row![
            container(column![
                text(notification.name.clone())
                    .size(FONT_SIZE)
                    .style(text::secondary),
                text(notification.summary.clone()).size(FONT_SIZE)
            ])
            .padding([0, 10])
            .width(Length::Fill),
            container(text(notification_time(&notification.time)).size(FONT_SIZE))
        ]);

        let body = notification
            .body
            .clone()
            .map(|body| self.body_markup(&body));

        let actions: Option<Row<Message>> = notification.actions.as_ref().map(|actions| {
            row(actions
                .iter()
                .cloned()
                .map(|(key, label)| -> Element<Message> {
                    button(text(label).size(FONT_SIZE).align_x(Horizontal::Center))
                        .on_press(Message::ActionInvoked(notification.id, key.clone()))
                        .padding(SMALL)
                        .style(|theme: &Theme, status| match status {
                            button::Status::Active | button::Status::Disabled => button::Style {
                                background: None,
                                text_color: theme.palette().text,
                                border: iced::border::color(theme.palette().text).width(1.0),
                                ..Default::default()
                            },
                            button::Status::Hovered | button::Status::Pressed => button::Style {
                                background: Some(Background::Color(theme.palette().text)),
                                text_color: theme.palette().background,
                                border: iced::border::color(theme.palette().text).width(1.0),
                                ..Default::default()
                            },
                        })
                        .width(Length::Fill)
                        .into()
                }))
            .spacing(SMALL)
        });

        let content = column![row![icon, header].width(Length::Fill)]
            .push_maybe(body)
            .push_maybe(actions)
            .spacing(SMALL);

        let urgency = notification.urgency;
        mouse_area(
            container(content)
                .style(move |theme| {
                    let border_color = if urgency == Urgency::Critical {
                        theme.palette().danger
                    } else {
                        theme.palette().text
                    };
                    let border = Border::default().width(2).color(border_color);
                    container::bordered_box(theme)
                        .border(border)
                        .background(theme.palette().background)
                })
                .padding(BIG)
                .width(WIDTH),
        )
        .on_right_release(Message::UserDismissed(notification.id))
        .into()
    }

    fn body_markup(&self, body: &[BodyElement]) -> Element<Message> {
        Column::from_iter(body.iter().map(|element| {
            match element {
                BodyElement::RichText(spans) => {
                    text::Rich::from_iter(spans.iter().map(|RichTextSpan { style, text }| {
                        let mut font = iced::Font::with_name("JetBrains Mono");
                        if style.bold {
                            font.weight = iced::font::Weight::Bold;
                        }
                        if style.italic {
                            font.style = iced::font::Style::Italic;
                        }
                        iced::widget::span(text.clone())
                            .size(FONT_SIZE)
                            .font(font)
                            .underline(style.underline)
                    }))
                    .into()
                }
                // Tooltip doesn't work?
                BodyElement::Image { src, alt } => iced::widget::tooltip(
                    iced::widget::image(src),
                    text(alt.clone()),
                    iced::widget::tooltip::Position::Top,
                )
                .into(),
            }
        }))
        .into()
    }

    fn remove_expired(&mut self) {
        let expired: Vec<u32> = self
            .alerts
            .iter()
            .filter_map(|id| self.notifications.get(id))
            .filter(|notification| {
                notification
                    .expire_time
                    .is_some_and(|expire_time| Local::now() > expire_time)
            })
            .map(|notification| notification.id)
            .collect();

        expired.into_iter().for_each(|id| {
            debug!("Notification {id} expired");
            if let SignallerState::Initialized(signaller) = &mut self.signaller {
                signaller.close_notification(id, NotificationClosedReason::Expired);
            } else {
                warn!("Signaller not initialized");
            }
            self.remove_notification(id);
        });
    }

    fn remove_notification(&mut self, id: u32) {
        debug!("Removing notification {}", id);

        // Remove the notification from alerts
        if let Some((index, _)) = self
            .alerts
            .iter()
            .enumerate()
            .find(|(_, &alert_id)| alert_id == id)
        {
            self.alerts.remove(index);
        }

        // Remove the notification data
        self.notifications.remove(&id);
    }
}

impl MultiApplication for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();
    type Theme = Theme;

    fn new(_flags: Self::Flags) -> (Self, Task<Self::Message>) {
        (Self::default(), Task::none())
    }

    fn namespace(&self) -> String {
        String::from("Notifications")
    }

    fn view(&self, _window: window::Id) -> Element<Message> {
        // Create a column of notifications from the alerts
        let notifications = Column::from_iter(
            self.alerts
                .iter()
                .filter_map(|id| self.notifications.get(id))
                .map(|notification| self.view_notification(notification)),
        )
        .spacing(SMALL);

        // Wrap the column in a measuring container to dynamically resize the layer shell
        MeasuringContainer::new(notifications.into(), |size| {
            let height = size.height.ceil() as u32;
            Message::ContainerResized(height.clamp(1, 2000))
        })
        .into()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // Prune expired notifications
        self.remove_expired();

        // If there are no alerts to display, close the window
        if self.alerts.is_empty() {
            if let Some(id) = self.window_id {
                debug!("Closing layer shell");
                return Task::done(Message::RemoveWindow(id));
            }
        }

        // Process messages
        match message {
            Message::ContainerResized(height) => {
                trace!("Container resized: {:?}", height);

                let size = (WIDTH as u32, height);
                if let Some(id) = self.window_id {
                    Task::done(Message::SizeChange { id, size })
                } else {
                    warn!("Container resized but window doesn't exist");
                    Task::none()
                }
            }
            Message::ActionInvoked(id, key) => {
                debug!("Action invoked: {} {}", id, key);

                match &mut self.signaller {
                    SignallerState::Unitialized => {
                        warn!("Signaller unitialized, unable to signal action")
                    }
                    SignallerState::Initialized(signaller) => signaller.action_invoked(id, key),
                }

                Task::none()
            }
            Message::Dbus(message) => match message {
                DbusMessage::Initialized(signaller) => {
                    debug!("NotificationSignaller initialized");

                    self.signaller = SignallerState::Initialized(signaller);

                    Task::none()
                }
                DbusMessage::Notify(notification) => {
                    debug!("Received notification {}", notification.id);

                    // Add new notification to alerts
                    self.alerts.push(notification.id);
                    self.notifications.insert(notification.id, notification);

                    // Create the layer shell if it doesn't exist
                    if self.window_id.is_none() {
                        debug!("Creating layer shell");
                        let id = window::Id::unique();
                        self.window_id = Some(id);
                        Task::done(Message::NewLayerShell {
                            settings: NewLayerShellSettings {
                                anchor: Anchor::Top | Anchor::Right,
                                size: Some((1, 1)),
                                margin: Some((60, 20, 0, 0)),
                                ..Default::default()
                            },
                            id,
                        })
                    } else {
                        Task::none()
                    }
                }
                DbusMessage::CloseNotification(id) => {
                    // Remove the notification and send the DBUS signal
                    self.remove_notification(id);
                    if let SignallerState::Initialized(signaller) = &mut self.signaller {
                        signaller.close_notification(
                            id,
                            NotificationClosedReason::ClosedByCloseNotification,
                        );
                    } else {
                        warn!("Signaller not initialized");
                    }

                    Task::none()
                }
            },
            Message::Tick => Task::none(),
            Message::UserDismissed(id) => {
                debug!("User dismissed notification {id}");

                // Remove the notification and send the DBUS signal
                self.remove_notification(id);
                if let SignallerState::Initialized(signaller) = &mut self.signaller {
                    signaller.close_notification(id, NotificationClosedReason::DismissedByUser);
                } else {
                    warn!("Signaller not initialized");
                }

                Task::none()
            }
            _ => unreachable!(),
        }
    }

    fn remove_id(&mut self, _id: window::Id) {
        self.window_id = None;
    }

    fn subscription(&self) -> Subscription<Message> {
        let dbus = Subscription::run(dbus::dbus).map(Message::Dbus);
        // Send a message every second to run update, to update times and remove expired
        // notifications
        let ticker = iced::time::every(iced::time::Duration::from_secs(1)).map(|_| Message::Tick);
        Subscription::batch([dbus, ticker])
    }

    fn style(&self, theme: &Theme) -> iced_layershell::Appearance {
        use iced_layershell::Appearance;
        Appearance {
            background_color: iced::Color::TRANSPARENT,
            text_color: theme.palette().text,
        }
    }

    fn theme(&self) -> Self::Theme {
        iced::Theme::custom(
            "Gruvbox Dark".into(),
            iced::theme::Palette {
                text: iced::color!(0xebdbb2),
                ..iced::theme::Palette::GRUVBOX_DARK
            },
        )
    }
}
