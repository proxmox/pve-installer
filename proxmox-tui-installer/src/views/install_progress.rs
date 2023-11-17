use std::{
    io::{BufRead, BufReader, Write},
    str::FromStr,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use cursive::{
    utils::Counter,
    view::{Nameable, Resizable, ViewWrapper},
    views::{Dialog, DummyView, LinearLayout, PaddedView, ProgressBar, TextContent, TextView},
    CbSink, Cursive,
};

use crate::{abort_install_button, prompt_dialog, setup::InstallConfig, InstallerState};
use proxmox_installer_common::setup::spawn_low_level_installer;

pub struct InstallProgressView {
    view: PaddedView<LinearLayout>,
}

impl InstallProgressView {
    pub fn new(siv: &mut Cursive) -> Self {
        let cb_sink = siv.cb_sink().clone();
        let state = siv.user_data::<InstallerState>().unwrap();
        let progress_text = TextContent::new("starting the installation ..");

        let progress_task = {
            let progress_text = progress_text.clone();
            let state = state.clone();
            move |counter: Counter| Self::progress_task(counter, cb_sink, state, progress_text)
        };

        let progress_bar = ProgressBar::new().with_task(progress_task).full_width();
        let view = PaddedView::lrtb(
            1,
            1,
            1,
            1,
            LinearLayout::vertical()
                .child(PaddedView::lrtb(1, 1, 0, 0, progress_bar))
                .child(DummyView)
                .child(TextView::new_with_content(progress_text).center())
                .child(PaddedView::lrtb(
                    1,
                    1,
                    1,
                    0,
                    LinearLayout::horizontal().child(abort_install_button()),
                )),
        );

        Self { view }
    }

    fn progress_task(
        counter: Counter,
        cb_sink: CbSink,
        state: InstallerState,
        progress_text: TextContent,
    ) {
        let mut child = match spawn_low_level_installer(state.in_test_mode) {
            Ok(child) => child,
            Err(err) => {
                let _ = cb_sink.send(Box::new(move |siv| {
                    siv.add_layer(
                        Dialog::text(err.to_string())
                            .title("Error")
                            .button("Ok", Cursive::quit),
                    );
                }));
                return;
            }
        };

        let inner = || {
            let reader = child
                .stdout
                .take()
                .map(BufReader::new)
                .ok_or("failed to get stdin reader")?;

            let mut writer = child.stdin.take().ok_or("failed to get stdin writer")?;

            serde_json::to_writer(&mut writer, &InstallConfig::from(state.options))
                .map_err(|err| format!("failed to serialize install config: {err}"))?;
            writeln!(writer).map_err(|err| format!("failed to write install config: {err}"))?;

            let writer = Arc::new(Mutex::new(writer));

            for line in reader.lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(err) => return Err(format!("low-level installer exited early: {err}")),
                };

                let msg = match line.parse::<UiMessage>() {
                    Ok(msg) => msg,
                    Err(stray) => {
                        // Not a fatal error, so don't abort the installation by returning
                        eprintln!("low-level installer: {stray}");
                        continue;
                    }
                };

                let result = match msg.clone() {
                    UiMessage::Info(s) => cb_sink.send(Box::new(|siv| {
                        siv.add_layer(Dialog::info(s).title("Information"));
                    })),
                    UiMessage::Error(s) => cb_sink.send(Box::new(|siv| {
                        siv.add_layer(Dialog::info(s).title("Error"));
                    })),
                    UiMessage::Prompt(s) => cb_sink.send({
                        let writer = writer.clone();
                        Box::new(move |siv| Self::show_prompt(siv, &s, writer))
                    }),
                    UiMessage::Progress(ratio, s) => {
                        counter.set(ratio);
                        progress_text.set_content(s);
                        Ok(())
                    }
                    UiMessage::Finished(success, msg) => {
                        counter.set(100);
                        progress_text.set_content(msg.to_owned());
                        cb_sink.send(Box::new(move |siv| {
                            Self::prepare_for_reboot(siv, success, &msg)
                        }))
                    }
                };

                if let Err(err) = result {
                    eprintln!("error during message handling: {err}");
                    eprintln!("  message was: '{msg:?}");
                }
            }

            Ok(())
        };

        if let Err(err) = inner() {
            let message = format!("installation failed: {err}");
            cb_sink
                .send(Box::new(|siv| {
                    siv.add_layer(
                        Dialog::text(message)
                            .title("Error")
                            .button("Exit", Cursive::quit),
                    );
                }))
                .unwrap();
        }
    }

    fn prepare_for_reboot(siv: &mut Cursive, success: bool, msg: &str) {
        const DIALOG_ID: &str = "autoreboot-dialog";
        let title = if success { "Success" } else { "Failure" };

        // If the dialog was previously created, just update its content and we're done.
        if let Some(mut dialog) = siv.find_name::<Dialog>(DIALOG_ID) {
            dialog.set_content(TextView::new(msg));
            return;
        }

        // For rebooting, we just need to quit the installer,
        // our caller does the actual reboot.
        siv.add_layer(
            Dialog::text(msg)
                .title(title)
                .button("Reboot now", Cursive::quit)
                .with_name(DIALOG_ID),
        );

        let autoreboot = siv
            .user_data::<InstallerState>()
            .map(|state| state.options.autoreboot)
            .unwrap_or_default();

        if autoreboot && success {
            let cb_sink = siv.cb_sink();
            thread::spawn({
                let cb_sink = cb_sink.clone();
                move || {
                    thread::sleep(Duration::from_secs(5));
                    let _ = cb_sink.send(Box::new(Cursive::quit));
                }
            });
        }
    }

    fn show_prompt<W: Write + 'static>(siv: &mut Cursive, text: &str, writer: Arc<Mutex<W>>) {
        prompt_dialog(
            siv,
            "Prompt",
            text,
            "OK",
            Box::new({
                let writer = writer.clone();
                move |_| {
                    if let Ok(mut writer) = writer.lock() {
                        let _ = writeln!(writer, "ok");
                    }
                }
            }),
            "Cancel",
            Box::new(move |_| {
                if let Ok(mut writer) = writer.lock() {
                    let _ = writeln!(writer);
                }
            }),
        );
    }
}

impl ViewWrapper for InstallProgressView {
    cursive::wrap_impl!(self.view: PaddedView<LinearLayout>);
}

#[derive(Clone, Debug)]
enum UiMessage {
    Info(String),
    Error(String),
    Prompt(String),
    Finished(bool, String),
    Progress(usize, String),
}

impl FromStr for UiMessage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (ty, rest) = s.split_once(": ").ok_or("invalid message: no type")?;

        match ty {
            "message" => Ok(UiMessage::Info(rest.to_owned())),
            "error" => Ok(UiMessage::Error(rest.to_owned())),
            "prompt" => Ok(UiMessage::Prompt(rest.to_owned())),
            "finished" => {
                let (state, rest) = rest.split_once(", ").ok_or("invalid message: no state")?;
                Ok(UiMessage::Finished(state == "ok", rest.to_owned()))
            }
            "progress" => {
                let (percent, rest) = rest.split_once(' ').ok_or("invalid progress message")?;
                Ok(UiMessage::Progress(
                    percent
                        .parse::<f64>()
                        .map(|v| (v * 100.).floor() as usize)
                        .map_err(|err| err.to_string())?,
                    rest.to_owned(),
                ))
            }
            unknown => Err(format!("invalid message type {unknown}, rest: {rest}")),
        }
    }
}
