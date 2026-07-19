//! Coalesced frame requests for terminal redraws.
//!
//! Default path targets interactive UI (~60 FPS). Streaming can request frames
//! with the same coalescer; delayed frames use `schedule_frame_in`.

use std::time::{Duration, Instant};

/// Minimum gap between emitted frames (≈60 FPS).
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Preferred gap while streaming agent output (same as default; kept for call-site clarity).
pub const STREAMING_FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Copy, Debug)]
struct FrameRequest {
    /// Earliest time this request wants a draw.
    at: Instant,
}

#[derive(Clone)]
pub struct FrameRequester {
    tx: tokio::sync::mpsc::UnboundedSender<FrameRequest>,
}

pub struct FrameRequestReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<()>,
}

impl FrameRequester {
    fn new(tx: tokio::sync::mpsc::UnboundedSender<FrameRequest>) -> Self {
        Self { tx }
    }

    /// Request a draw as soon as the frame scheduler allows.
    pub fn schedule_frame(&self) {
        let _ = self.tx.send(FrameRequest { at: Instant::now() });
    }

    /// Request a draw no earlier than `delay` from now (animations, flash expiry).
    pub fn schedule_frame_in(&self, delay: Duration) {
        let _ = self.tx.send(FrameRequest {
            at: Instant::now() + delay,
        });
    }

    /// Streaming / high-churn updates — same coalescer, explicit call site.
    pub fn schedule_frame_streaming(&self) {
        let _ = self.tx.send(FrameRequest { at: Instant::now() });
        let _ = STREAMING_FRAME_INTERVAL; // documented companion constant
    }
}

impl FrameRequestReceiver {
    pub async fn recv(&mut self) -> Option<()> {
        self.rx.recv().await
    }

    /// Drain coalesced frame tokens without waiting (draw once for the burst).
    pub fn try_recv(&mut self) -> Option<()> {
        self.rx.try_recv().ok()
    }
}

pub fn frame_channel() -> (FrameRequester, FrameRequestReceiver) {
    let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
    let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(FrameScheduler::new(request_rx, frame_tx).run());
    (
        FrameRequester::new(request_tx),
        FrameRequestReceiver { rx: frame_rx },
    )
}

struct FrameScheduler {
    requests: tokio::sync::mpsc::UnboundedReceiver<FrameRequest>,
    frames: tokio::sync::mpsc::UnboundedSender<()>,
    last_emitted_at: Option<Instant>,
}

impl FrameScheduler {
    fn new(
        requests: tokio::sync::mpsc::UnboundedReceiver<FrameRequest>,
        frames: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> Self {
        Self {
            requests,
            frames,
            last_emitted_at: None,
        }
    }

    async fn run(mut self) {
        const FAR_FUTURE: Duration = Duration::from_secs(60 * 60 * 24 * 365);
        let mut next_deadline: Option<Instant> = None;

        loop {
            let target = next_deadline.unwrap_or_else(|| Instant::now() + FAR_FUTURE);
            let deadline = tokio::time::sleep_until(target.into());
            tokio::pin!(deadline);

            tokio::select! {
                request = self.requests.recv() => {
                    let Some(request) = request else {
                        break;
                    };
                    let draw_at = self.clamp_deadline(request.at);
                    next_deadline = Some(next_deadline.map_or(draw_at, |current| current.min(draw_at)));
                }
                _ = &mut deadline => {
                    if next_deadline.is_some() {
                        next_deadline = None;
                        self.last_emitted_at = Some(target);
                        let _ = self.frames.send(());
                    }
                }
            }
        }
    }

    fn clamp_deadline(&self, requested_at: Instant) -> Instant {
        match self.last_emitted_at {
            Some(last) => requested_at.max(last + MIN_FRAME_INTERVAL),
            None => requested_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn schedule_frame_coalesces_bursts() {
        let (requester, mut receiver) = frame_channel();

        requester.schedule_frame();
        requester.schedule_frame();
        requester.schedule_frame();

        tokio::time::timeout(Duration::from_millis(100), receiver.recv())
            .await
            .expect("first frame should arrive");

        assert!(
            tokio::time::timeout(Duration::from_millis(5), receiver.recv())
                .await
                .is_err(),
            "burst should coalesce into one immediate frame"
        );
    }

    #[tokio::test]
    async fn schedule_frame_in_delays_draw() {
        let (requester, mut receiver) = frame_channel();
        requester.schedule_frame_in(Duration::from_millis(40));

        assert!(
            tokio::time::timeout(Duration::from_millis(10), receiver.recv())
                .await
                .is_err(),
            "delayed frame must not fire immediately"
        );

        tokio::time::timeout(Duration::from_millis(80), receiver.recv())
            .await
            .expect("delayed frame should arrive")
            .expect("channel open");
    }
}
