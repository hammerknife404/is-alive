//! Background worker with a message channel back to the UI thread.

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct Tx<T> {
    tx: Sender<T>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
}

impl<T> Tx<T> {
    pub fn send(&self, v: T) {
        let _ = self.tx.send(v);
        self.ctx.request_repaint();
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// Sleep in short slices so Stop stays responsive.
    pub fn sleep(&self, d: Duration) {
        let end = Instant::now() + d;
        while !self.cancelled() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
}

impl<T> Drop for Tx<T> {
    fn drop(&mut self) {
        self.ctx.request_repaint();
    }
}

pub struct Job<T> {
    rx: Receiver<T>,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce(Tx<T>) + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let tx = Tx {
            tx,
            ctx: ctx.clone(),
            cancel: cancel.clone(),
        };
        let handle = std::thread::spawn(move || f(tx));
        Self { rx, cancel, handle }
    }

    pub fn drain(&self) -> impl Iterator<Item = T> + '_ {
        self.rx.try_iter()
    }

    pub fn running(&self) -> bool {
        !self.handle.is_finished()
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Poll an optional job: feed messages to `on_msg`, drop it once finished.
/// Returns true while the job is still running.
pub fn pump<T: Send + 'static>(job: &mut Option<Job<T>>, mut on_msg: impl FnMut(T)) -> bool {
    let Some(j) = job else { return false };
    let running = j.running();
    for m in j.drain() {
        on_msg(m);
    }
    if !running {
        *job = None;
    }
    running
}
