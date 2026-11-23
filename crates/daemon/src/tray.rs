//! The tray icon on Linux: what the recorder is doing at a glance, and
//! the few things worth doing without opening a window. What it says and
//! offers is `tray_menu`'s, shared with the Windows tray; here it is only
//! handed to the desktop.

use crate::daemon::Msg;
use crate::tray_menu::{self, Action, Entry};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};
use ksni::{Handle, TrayMethods};
use tokio::sync::mpsc;
use zenith_core::ipc::Status;

pub struct Tray {
    status: Status,
    clip_dir: std::path::PathBuf,
    tx: mpsc::UnboundedSender<Msg>,
}

impl Tray {
    fn act(&self, action: Action) {
        action.perform(&self.clip_dir, &self.tx);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "zenith-clips".into()
    }

    fn title(&self) -> String {
        "Zenith".into()
    }

    fn icon_name(&self) -> String {
        match self.status.state.as_str() {
            "capturing" | "saving" => "media-record",
            "failed" | "restarting" => "dialog-warning",
            _ => "zenith-clips",
        }
        .into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Zenith".into(),
            description: format!("{} · clips reach back {} s", tray_menu::headline(&self.status), self.status.buffer_secs),
            ..Default::default()
        }
    }

    /// A click on the icon saves a clip.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.act(Action::Save);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let entries = tray_menu::entries(&self.status);
        // The choices are one group to this desktop, put where the first
        // of them stands.
        let choices: Vec<(&str, Action, bool)> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Choice { label, action, checked } => Some((*label, *action, *checked)),
                _ => None,
            })
            .collect();
        let mut group = Some(RadioGroup {
            selected: choices.iter().position(|choice| choice.2).unwrap_or(0),
            options: choices.iter().map(|choice| RadioItem { label: choice.0.into(), ..Default::default() }).collect(),
            select: Box::new(move |tray: &mut Self, index| {
                if let Some(choice) = choices.get(index) {
                    tray.act(choice.1);
                }
            }),
        });
        entries
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Status(label) => Some(StandardItem { label, enabled: false, ..Default::default() }.into()),
                Entry::Separator => Some(MenuItem::Separator),
                Entry::Item { label, action, enabled } => {
                    Some(StandardItem { label: label.into(), enabled, activate: Box::new(move |tray: &mut Self| tray.act(action)), ..Default::default() }.into())
                }
                Entry::Choice { .. } => group.take().map(Into::into),
            })
            .collect()
    }
}

/// Put the icon in the tray. `None` where there is no tray to put it in;
/// the recorder works the same without.
pub async fn start(status: Status, clip_dir: std::path::PathBuf, tx: mpsc::UnboundedSender<Msg>) -> Option<Handle<Tray>> {
    // Registered whether or not a tray is there yet: the bar may start later.
    match (Tray { status, clip_dir, tx }).assume_sni_available(true).spawn().await {
        Ok(handle) => Some(handle),
        Err(e) => {
            crate::warn(format_args!("zenithd: no tray icon: {e}"));
            None
        }
    }
}

/// Show a new status, if it differs from the one shown.
pub async fn show(handle: &Option<Handle<Tray>>, status: Status, clip_dir: &std::path::Path) {
    if let Some(handle) = handle {
        let clip_dir = clip_dir.to_path_buf();
        handle
            .update(move |tray| {
                tray.status = status;
                tray.clip_dir = clip_dir;
            })
            .await;
    }
}

/// Take the icon out of the tray again.
pub async fn stop(handle: Option<Handle<Tray>>) {
    if let Some(handle) = handle {
        handle.shutdown().await;
    }
}
