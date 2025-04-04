use cursive::{
    CbSink, Cursive,
    utils::Counter,
    view::{Nameable, Resizable, ViewWrapper},
    views::{Dialog, DummyView, LinearLayout, PaddedView, ProgressBar, TextView},
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{InstallerState, abort_install_button, prompt_dialog};
use proxmox_installer_common::setup::{InstallConfig, LowLevelMessage, spawn_low_level_installer};

pub struct InstallProgressView {
    view: PaddedView<LinearLayout>,
}

impl InstallProgressView {
    const PROGRESS_TEXT_VIEW_ID: &'static str = "progress-text";

    pub fn new(siv: &mut Cursive) -> Self {
        let cb_sink = siv.cb_sink().clone();
        let state = siv.user_data::<InstallerState>().unwrap();

        let progress_task = {
            let state = state.clone();
            move |counter: Counter| Self::progress_task(counter, cb_sink, state)
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
                .child(
                    TextView::new("starting the installation ..")
                        .center()
                        .with_name(Self::PROGRESS_TEXT_VIEW_ID),
                )
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

    fn progress_task(counter: Counter, cb_sink: CbSink, state: InstallerState) {
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

            let mut lowlevel_log = File::create("/tmp/install-low-level.log")
                .map_err(|err| format!("failed to open low-level installer logfile: {err}"))?;

            let writer = Arc::new(Mutex::new(writer));

            for line in reader.lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(err) => return Err(format!("low-level installer exited early: {err}")),
                };

                // The low-level installer also spews the output of any command it runs on its
                // stdout. Use a very simple heuricstic to determine whether it is actually JSON
                // or not.
                if !line.starts_with('{') || !line.ends_with('}') {
                    let _ = writeln!(lowlevel_log, "{}", line);
                    continue;
                }

                let msg = match serde_json::from_str::<LowLevelMessage>(&line) {
                    Ok(msg) => msg,
                    Err(err) => {
                        // Not a fatal error, so don't abort the installation by returning
                        eprintln!("low-level installer: error while parsing message: '{err}'");
                        eprintln!("    original message was: '{line}'");
                        continue;
                    }
                };

                let result = match msg.clone() {
                    LowLevelMessage::Info { message } => cb_sink.send(Box::new(|siv| {
                        siv.add_layer(Dialog::info(message).title("Information"));
                    })),
                    LowLevelMessage::Error { message } => cb_sink.send(Box::new(|siv| {
                        siv.add_layer(Dialog::info(message).title("Error"));
                    })),
                    LowLevelMessage::Prompt { query } => cb_sink.send({
                        let writer = writer.clone();
                        Box::new(move |siv| Self::show_prompt(siv, &query, writer))
                    }),
                    LowLevelMessage::Progress { ratio, text } => {
                        counter.set((ratio * 100.).floor() as usize);
                        if let Some(text) = text {
                            cb_sink.send(Box::new(move |siv| {
                                siv.call_on_name(
                                    Self::PROGRESS_TEXT_VIEW_ID,
                                    |v: &mut TextView| {
                                        v.set_content(text);
                                    },
                                );
                            }))
                        } else {
                            Ok(())
                        }
                    }
                    LowLevelMessage::Finished { state, message } => {
                        counter.set(100);
                        cb_sink.send(Box::new(move |siv| {
                            siv.call_on_name(Self::PROGRESS_TEXT_VIEW_ID, |v: &mut TextView| {
                                v.set_content(&message);
                            });
                            Self::prepare_for_reboot(siv, state == "ok", &message);
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

    fn show_prompt<W: Write + 'static + Send>(
        siv: &mut Cursive,
        text: &str,
        writer: Arc<Mutex<W>>,
    ) {
        let send_answer = |writer: Arc<Mutex<W>>, answer| {
            if let Ok(mut writer) = writer.lock() {
                let _ = writeln!(
                    writer,
                    "{}",
                    serde_json::json!({
                        "type" : "prompt-answer",
                        "answer" : answer,
                    })
                );
            }
        };

        prompt_dialog(
            siv,
            "Prompt",
            text,
            "OK",
            Box::new({
                let writer = writer.clone();
                move |_| {
                    send_answer(writer.clone(), "ok");
                }
            }),
            "Cancel",
            Box::new(move |_| {
                send_answer(writer.clone(), "cancel");
            }),
        );
    }
}

impl ViewWrapper for InstallProgressView {
    cursive::wrap_impl!(self.view: PaddedView<LinearLayout>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn next_msg<R: BufRead>(reader: &mut R) -> Option<LowLevelMessage> {
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => return None, /* reached EOF */
            Err(err) => panic!("failed to read message: {err}"),
            _ => {}
        }

        match serde_json::from_str::<LowLevelMessage>(&line) {
            Ok(msg) => Some(msg),
            Err(err) => {
                eprintln!("invalid json: '{err}'");
                // Skip over all spurious output that may be produced by the low-level
                // installer, in the same manner as InstallProgressView::progress_task()
                // above does the actual processing.
                next_msg(reader)
            }
        }
    }

    #[test]
    fn run_low_level_installer_test_session() {
        env::set_current_dir("..").expect("failed to change working directory");
        let mut child = spawn_low_level_installer(true)
            .expect("failed to run low-level installer test session");

        let mut reader = child
            .stdout
            .take()
            .map(BufReader::new)
            .expect("failed to get stdin reader");

        let mut writer = child.stdin.take().expect("failed to get stdin writer");

        serde_json::to_writer(&mut writer, &serde_json::json!({ "autoreboot": false }))
            .expect("failed to serialize install config");

        writeln!(writer).expect("failed to write install config: {err}");

        assert_eq!(
            next_msg(&mut reader),
            Some(LowLevelMessage::Prompt {
                query: "Reply anything?".to_owned()
            }),
        );

        serde_json::to_writer(
            &mut writer,
            &serde_json::json!({"type": "prompt-answer", "answer": "ok"}),
        )
        .expect("failed to write prompt answer");
        writeln!(writer).expect("failed to write prompt answer");

        assert_eq!(
            next_msg(&mut reader),
            Some(LowLevelMessage::Info {
                message: "Test Message - got ok".to_owned()
            }),
        );

        for i in (1..=1000).step_by(3) {
            assert_eq!(
                next_msg(&mut reader),
                Some(LowLevelMessage::Progress {
                    ratio: (i as f32) / 1000.,
                    text: Some(format!("foo {i}")),
                }),
            );
        }

        assert_eq!(
            next_msg(&mut reader),
            Some(LowLevelMessage::Finished {
                state: "ok".to_owned(),
                message: "Installation finished - reboot now?".to_owned(),
            }),
        );

        // Should be nothing left to read now
        let mut line = String::new();
        assert_eq!(reader.read_line(&mut line).expect("success"), 0);

        // Give the low-level installer some time to exit properly
        std::thread::sleep(Duration::new(1, 0));

        match child.try_wait() {
            Ok(Some(status)) => assert!(
                status.success(),
                "low-level installer did not exit successfully"
            ),
            Ok(None) => {
                child.kill().expect("could not kill low-level installer");
                panic!("low-level install was not successful");
            }
            Err(err) => panic!("failed to wait for low-level installer: {err}"),
        }
    }
}
