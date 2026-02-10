//! Visual TUI test harness for cross-control.
//!
//! Runs two daemon instances on loopback with mock backends.
//! Arrow keys move the cursor, letter keys send key events.
//! Shows two screen rectangles, cursor position, and a live event log.

mod app;
mod input_handler;
mod ui;

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use cross_control_daemon::config::{Config, DaemonConfig, IdentityConfig, ScreenConfig};
use cross_control_daemon::{Daemon, DaemonEvent};
use cross_control_input::mock::{MockCapture, MockEmulation};
use cross_control_types::{
    CapturedEvent, DeviceCapability, DeviceId, DeviceInfo, MachineId, Position,
};
use tokio::sync::mpsc;

use app::AppState;

const SCREEN_W: u32 = 1920;
const SCREEN_H: u32 = 1080;

fn test_devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: DeviceId(1),
            name: "Test Keyboard".to_string(),
            capabilities: vec![DeviceCapability::Keyboard],
        },
        DeviceInfo {
            id: DeviceId(2),
            name: "Test Mouse".to_string(),
            capabilities: vec![DeviceCapability::RelativeMouse, DeviceCapability::Scroll],
        },
    ]
}

struct Handles {
    feed: mpsc::Sender<CapturedEvent>,
    shutdown_a: mpsc::Sender<DaemonEvent>,
    shutdown_b: mpsc::Sender<DaemonEvent>,
    app: AppState,
}

async fn setup_daemons() -> Result<Handles, Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_a = cross_control_certgen::generate_certificate("localhost")?;
    let cert_b = cross_control_certgen::generate_certificate("localhost")?;

    let bind: SocketAddr = "127.0.0.1:0".parse()?;
    let transport_a =
        cross_control_protocol::QuicTransport::bind(bind, &cert_a.cert_pem, &cert_a.key_pem)?;
    let transport_b =
        cross_control_protocol::QuicTransport::bind(bind, &cert_b.cert_pem, &cert_b.key_pem)?;

    let addr_b = transport_b.local_addr()?;

    let config_a = Config {
        daemon: DaemonConfig {
            screen_width: SCREEN_W,
            screen_height: SCREEN_H,
            ..DaemonConfig::default()
        },
        identity: IdentityConfig {
            name: "machine-a".to_string(),
        },
        screens: vec![ScreenConfig {
            name: "machine-b".to_string(),
            address: Some(addr_b.to_string()),
            position: Position::Right,
            fingerprint: None,
        }],
        ..Config::default()
    };

    let config_b = Config {
        daemon: DaemonConfig {
            screen_width: SCREEN_W,
            screen_height: SCREEN_H,
            ..DaemonConfig::default()
        },
        identity: IdentityConfig {
            name: "machine-b".to_string(),
        },
        screens: vec![ScreenConfig {
            name: "machine-a".to_string(),
            address: None,
            position: Position::Left,
            fingerprint: None,
        }],
        ..Config::default()
    };

    let (capture_a, feed) = MockCapture::new();
    let emu_a = MockEmulation::new();
    let emu_handle_a = emu_a.handle();

    let (capture_b, _feed_b) = MockCapture::new();
    let emu_b = MockEmulation::new();
    let emu_handle_b = emu_b.handle();

    let mut daemon_a = Daemon::new(
        config_a,
        MachineId::new(),
        transport_a,
        Box::new(capture_a),
        Box::new(emu_a),
    );
    daemon_a.set_local_devices(test_devices());
    let status_a = daemon_a.status_receiver();
    let shutdown_a = daemon_a.event_sender();

    let mut daemon_b = Daemon::new(
        config_b,
        MachineId::new(),
        transport_b,
        Box::new(capture_b),
        Box::new(emu_b),
    );
    daemon_b.set_local_devices(test_devices());
    let status_b = daemon_b.status_receiver();
    let shutdown_b = daemon_b.event_sender();

    tokio::spawn(async move {
        let _ = daemon_b.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    tokio::spawn(async move {
        let _ = daemon_a.run().await;
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let app = AppState::new(
        status_a,
        status_b,
        emu_handle_a,
        emu_handle_b,
        SCREEN_W,
        SCREEN_H,
    );

    Ok(Handles {
        feed,
        shutdown_a,
        shutdown_b,
        app,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handles = setup_daemons().await?;
    let feed = handles.feed;
    let shutdown_a = handles.shutdown_a;
    let shutdown_b = handles.shutdown_b;
    let mut app = handles.app;

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    app.log("TUI harness started. Use arrow keys to move cursor.".to_string());
    app.log("Press letter keys to send key events.".to_string());
    app.log("Press Ctrl+Shift+Esc to release control.".to_string());

    loop {
        app.sync_cursor();
        app.poll_injections();

        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(AppState::tick_rate())? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press
                    && input_handler::handle_key(key, &feed, &mut app).await
                {
                    break;
                }
            }
        }

        if app.quit {
            break;
        }
    }

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    let _ = shutdown_a.send(DaemonEvent::Shutdown).await;
    let _ = shutdown_b.send(DaemonEvent::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    Ok(())
}
