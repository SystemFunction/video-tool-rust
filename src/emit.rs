//! Thin wrapper for sending UI messages from worker threads and waking egui.

use std::sync::mpsc::Sender;

use eframe::egui;

use crate::types::{Task, UiMsg};

#[derive(Clone)]
pub struct Emitter {
    pub tx: Sender<UiMsg>,
    pub ctx: egui::Context,
}

impl Emitter {
    pub fn new(tx: Sender<UiMsg>, ctx: egui::Context) -> Self {
        Emitter { tx, ctx }
    }

    pub fn send(&self, msg: UiMsg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }

    pub fn log(&self, task: Task, line: impl Into<String>) {
        self.send(UiMsg::Log(task, line.into()));
    }

    pub fn status(&self, task: Task, text: impl Into<String>) {
        self.send(UiMsg::Status(task, text.into()));
    }

    pub fn progress(&self, task: Task, value: f32) {
        self.send(UiMsg::Progress(task, value));
    }

    pub fn busy(&self, task: Task, busy: bool) {
        self.send(UiMsg::Busy(task, busy));
    }

    pub fn toast(&self, text: impl Into<String>, error: bool) {
        self.send(UiMsg::Toast(text.into(), error));
    }

    pub fn setup_log(&self, text: impl Into<String>) {
        self.send(UiMsg::SetupLog(text.into()));
    }

    pub fn setup_busy(&self, busy: bool) {
        self.send(UiMsg::SetupBusy(busy));
    }
}
