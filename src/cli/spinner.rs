use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;

const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Spinner {
    /// Start a spinner with a message. The spinner runs on a background task.
    pub fn start(message: &str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let msg = message.to_string();

        let handle = tokio::spawn(async move {
            let mut i = 0;
            loop {
                if stop_clone.load(Ordering::Relaxed) {
                    break;
                }
                let frame = FRAMES[i % FRAMES.len()];
                print!("\r      {} {}", frame, msg);
                let _ = io::stdout().flush();
                i += 1;
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            }
        });

        Self { stop, handle }
    }

    /// Stop the spinner and show a success message.
    pub async fn done(self, message: &str) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.await;
        print!("\r\x1b[2K");
        println!("      \u{2713} {}", message);
    }

    /// Stop the spinner and show a failure message.
    pub async fn fail(self, message: &str) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.await;
        print!("\r\x1b[2K");
        println!("      \u{2717} {}", message);
    }

    /// Stop the spinner and clear the line without printing anything.
    pub async fn clear(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.await;
        print!("\r\x1b[2K");
        let _ = io::stdout().flush();
    }
}
