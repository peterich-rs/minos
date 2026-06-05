use crossterm::event::{KeyEvent, MouseEvent};
use minos_agent_runtime::{ManagerEvent, RawIngest};

pub enum AppEvent {
    Ingest(RawIngest),
    ManagerEvent(ManagerEvent),
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
}

pub fn spawn_ingest_pump(
    mut rx: tokio::sync::broadcast::Receiver<RawIngest>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ingest) => {
                    if tx.send(AppEvent::Ingest(ingest)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub fn spawn_manager_event_pump(
    mut rx: tokio::sync::broadcast::Receiver<ManagerEvent>,
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if tx.send(AppEvent::ManagerEvent(event)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

pub fn spawn_terminal_pump(tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) {
    std::thread::Builder::new()
        .name("minos-tui-terminal".into())
        .spawn(move || {
            let poll_timeout = std::time::Duration::from_millis(250);

            loop {
                match crossterm::event::poll(poll_timeout) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(crossterm::event::Event::Key(key)) => {
                            if tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(crossterm::event::Event::Mouse(mouse)) => {
                            if tx.send(AppEvent::Mouse(mouse)).is_err() {
                                break;
                            }
                        }
                        Ok(crossterm::event::Event::Resize(w, h)) => {
                            if tx.send(AppEvent::Resize(w, h)).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => continue,
                    Err(_) => break,
                }
            }
        })
        .expect("terminal event pump thread should spawn");
}

pub fn spawn_tick_pump(tx: tokio::sync::mpsc::UnboundedSender<AppEvent>, interval_ms: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });
}
