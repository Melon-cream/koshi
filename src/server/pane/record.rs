//! Starting and stopping one pane's recorder when the workspace flag changes.

use super::Pane;
use crate::server::record::{RecordTarget, Recorder};

impl Pane {
    /// Start recording this pane's output from now on. A pane that already
    /// records keeps its file.
    pub fn start_recording(&mut self, target: RecordTarget) {
        if self.recorder.is_some() {
            return;
        }
        match Recorder::start(&target) {
            Ok(mut recorder) => {
                if self.alt_screen() {
                    recorder.enter_alt();
                }
                self.recorder = Some(recorder);
            }
            Err(err) => eprintln!("koshi: cannot record pane {}: {err}", self.id),
        }
    }

    /// Stop recording and keep the file. The line the cursor sits on is kept
    /// first, as it can no longer change.
    pub fn stop_recording(&mut self) {
        if let Some(mut recorder) = self.recorder.take() {
            recorder.finish();
        }
    }

    pub fn flush_recording(&mut self) {
        if let Some(recorder) = self.recorder.as_mut() {
            let _ = recorder.flush();
        }
    }
}
