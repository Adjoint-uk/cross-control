//! Visual TUI test harness for cross-control.
//!
//! Runs four daemon instances on loopback with mock backends in a 2x2 grid:
//!
//!   A (top-left)  | B (top-right)
//!   --------------|---------------
//!   C (bot-left)  | D (bot-right)
//!
//! Arrow keys move the cursor, letter keys send key events.
//! Cursor crosses between screens at shared edges.

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

use cross_control_daemon::config::{Config, DaemonConfig, IdentityConfig, InputConfig, ScreenAdjacency, ScreenConfig};
use cross_control_daemon::{Daemon, DaemonEvent};
use cross_control_input::mock::{MockCapture, MockEmulation};
use cross_control_types::{
    CapturedEvent, DeviceCapability, DeviceId, DeviceInfo, MachineId, Position,
};
use tokio::sync::mpsc;

use app::{AppState, ScreenState};

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

fn release_hotkey() -> InputConfig {
    InputConfig {
        release_hotkey: vec!["F12".to_string()],
    }
}

fn daemon_config() -> DaemonConfig {
    DaemonConfig {
        screen_width: SCREEN_W,
        screen_height: SCREEN_H,
        ..DaemonConfig::default()
    }
}

struct DaemonBundle {
    name: String,
    daemon: Daemon,
    capture_feed: Option<mpsc::Sender<CapturedEvent>>,
    status: tokio::sync::watch::Receiver<cross_control_daemon::DaemonStatus>,
    shutdown: mpsc::Sender<DaemonEvent>,
    emu_handle: cross_control_input::mock::MockEmulationHandle,
}

/// Create a daemon with mock backends. Returns the daemon and its handles.
fn create_daemon(
    name: &str,
    transport: cross_control_protocol::QuicTransport,
    screens: Vec<ScreenConfig>,
    screen_adjacency: Vec<ScreenAdjacency>,
    is_primary: bool,
) -> DaemonBundle {
    let (capture, feed) = MockCapture::new();
    let emu = MockEmulation::new();
    let emu_handle = emu.handle();

    let config = Config {
        daemon: daemon_config(),
        identity: IdentityConfig {
            name: name.to_string(),
        },
        input: release_hotkey(),
        screens,
        screen_adjacency,
        ..Config::default()
    };

    let mut daemon = Daemon::new(
        config,
        MachineId::new(),
        transport,
        Box::new(capture),
        Box::new(emu),
    );
    daemon.set_local_devices(test_devices());
    let status = daemon.status_receiver();
    let shutdown = daemon.event_sender();

    DaemonBundle {
        name: name.to_string(),
        daemon,
        capture_feed: if is_primary { Some(feed) } else { None },
        status,
        shutdown,
        emu_handle,
    }
}

struct Handles {
    feed: mpsc::Sender<CapturedEvent>,
    shutdowns: Vec<mpsc::Sender<DaemonEvent>>,
    app: AppState,
}

async fn setup_daemons() -> Result<Handles, Box<dyn std::error::Error>> {
    // Enable tracing to stderr for debugging connection issues
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("cross_control=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let _ = rustls::crypto::ring::default_provider().install_default();

    let bind: SocketAddr = "127.0.0.1:0".parse()?;

    // Generate certs and bind transports for all 4 daemons
    let cert_a = cross_control_certgen::generate_certificate("localhost")?;
    let cert_b = cross_control_certgen::generate_certificate("localhost")?;
    let cert_c = cross_control_certgen::generate_certificate("localhost")?;
    let cert_d = cross_control_certgen::generate_certificate("localhost")?;

    let transport_a =
        cross_control_protocol::QuicTransport::bind(bind, &cert_a.cert_pem, &cert_a.key_pem)?;
    let transport_b =
        cross_control_protocol::QuicTransport::bind(bind, &cert_b.cert_pem, &cert_b.key_pem)?;
    let transport_c =
        cross_control_protocol::QuicTransport::bind(bind, &cert_c.cert_pem, &cert_c.key_pem)?;
    let transport_d =
        cross_control_protocol::QuicTransport::bind(bind, &cert_d.cert_pem, &cert_d.key_pem)?;

    let addr_b = transport_b.local_addr()?;
    let addr_c = transport_c.local_addr()?;
    let addr_d = transport_d.local_addr()?;

    //  Grid layout:
    //    A | B       A connects outbound to B and C
    //    -----       B connects outbound to D
    //    C | D       C connects outbound to D

    let mut bundle_a = create_daemon(
        "A",
        transport_a,
        vec![
            ScreenConfig {
                name: "B".to_string(),
                address: Some(addr_b.to_string()),
                position: Position::Right,
                fingerprint: None,
            },
            ScreenConfig {
                name: "C".to_string(),
                address: Some(addr_c.to_string()),
                position: Position::Below,
                fingerprint: None,
            },
        ],
        // Full graph edges that A needs for multi-hop navigation.
        // A already knows A↔B and A↔C from its screens config.
        // These describe the remote edges: B↔D and C↔D.
        vec![
            ScreenAdjacency {
                screen: "B".to_string(),
                neighbor: "D".to_string(),
                position: Position::Below,
            },
            ScreenAdjacency {
                screen: "C".to_string(),
                neighbor: "D".to_string(),
                position: Position::Right,
            },
        ],
        true, // primary — gets capture feed
    );

    let bundle_b = create_daemon(
        "B",
        transport_b,
        vec![
            ScreenConfig {
                name: "A".to_string(),
                address: None, // A connects to us
                position: Position::Left,
                fingerprint: None,
            },
            ScreenConfig {
                name: "D".to_string(),
                address: Some(addr_d.to_string()),
                position: Position::Below,
                fingerprint: None,
            },
        ],
        vec![],
        false,
    );

    let bundle_c = create_daemon(
        "C",
        transport_c,
        vec![
            ScreenConfig {
                name: "A".to_string(),
                address: None, // A connects to us
                position: Position::Above,
                fingerprint: None,
            },
            ScreenConfig {
                name: "D".to_string(),
                address: Some(addr_d.to_string()),
                position: Position::Right,
                fingerprint: None,
            },
        ],
        vec![],
        false,
    );

    let bundle_d = create_daemon(
        "D",
        transport_d,
        vec![
            ScreenConfig {
                name: "B".to_string(),
                address: None, // B connects to us
                position: Position::Above,
                fingerprint: None,
            },
            ScreenConfig {
                name: "C".to_string(),
                address: None, // C connects to us
                position: Position::Left,
                fingerprint: None,
            },
        ],
        vec![],
        false,
    );

    let feed = bundle_a.capture_feed.take().unwrap();

    // Collect status receivers and shutdown senders before moving daemons
    let statuses: Vec<_> = [&bundle_a, &bundle_b, &bundle_c, &bundle_d]
        .iter()
        .map(|b| b.status.clone())
        .collect();
    let shutdowns: Vec<_> = [&bundle_a, &bundle_b, &bundle_c, &bundle_d]
        .iter()
        .map(|b| b.shutdown.clone())
        .collect();
    let emu_handles: Vec<_> = vec![
        bundle_a.emu_handle,
        bundle_b.emu_handle,
        bundle_c.emu_handle,
        bundle_d.emu_handle,
    ];
    let names: Vec<_> = vec![
        bundle_a.name.clone(),
        bundle_b.name.clone(),
        bundle_c.name.clone(),
        bundle_d.name.clone(),
    ];

    // Spawn all daemons concurrently. Outbound connections are spawned as
    // background tasks inside each daemon, so accept loops are never blocked.
    let mut daemon_a = bundle_a.daemon;
    let mut daemon_b = bundle_b.daemon;
    let mut daemon_c = bundle_c.daemon;
    let mut daemon_d = bundle_d.daemon;
    tokio::spawn(async move { let _ = daemon_d.run().await; });
    tokio::spawn(async move { let _ = daemon_c.run().await; });
    tokio::spawn(async move { let _ = daemon_b.run().await; });
    tokio::spawn(async move { let _ = daemon_a.run().await; });

    // Wait for all daemons to establish their sessions
    // A should have 2 sessions (B, C), B should have 2 (A, D),
    // C should have 2 (A, D), D should have 2 (B, C)
    let mut wa = statuses[0].clone();
    let mut wb = statuses[1].clone();
    let mut wc = statuses[2].clone();
    let mut wd = statuses[3].clone();
    let connect_result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (a, b, c, d) = (
                wa.borrow().session_count,
                wb.borrow().session_count,
                wc.borrow().session_count,
                wd.borrow().session_count,
            );
            if a >= 2 && b >= 2 && c >= 2 && d >= 2 {
                break;
            }
            tokio::select! {
                _ = wa.changed() => {}
                _ = wb.changed() => {}
                _ = wc.changed() => {}
                _ = wd.changed() => {}
            }
        }
    })
    .await;

    if connect_result.is_err() {
        let (a, b, c, d) = (
            statuses[0].borrow().session_count,
            statuses[1].borrow().session_count,
            statuses[2].borrow().session_count,
            statuses[3].borrow().session_count,
        );
        panic!(
            "Daemons failed to fully connect within 10 seconds.\n\
             Session counts: A={a}, B={b}, C={c}, D={d}\n\
             Expected 2 each. Check connection topology."
        );
    }

    // Allow device announcements to propagate
    tokio::time::sleep(Duration::from_millis(300)).await;

    let screens: Vec<ScreenState> = names
        .into_iter()
        .zip(statuses)
        .zip(emu_handles)
        .map(|((name, status), emulation)| ScreenState {
            name,
            status,
            emulation,
            last_injected_count: 0,
        })
        .collect();

    let app = AppState::new(screens, SCREEN_W, SCREEN_H);

    Ok(Handles {
        feed,
        shutdowns,
        app,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handles = setup_daemons().await?;
    let feed = handles.feed;
    let shutdowns = handles.shutdowns;
    let mut app = handles.app;

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    app.log("4 daemons connected in 2x2 grid!".to_string());
    app.log("A(TL) B(TR) / C(BL) D(BR)".to_string());
    app.log("Arrow keys move cursor. Crosses edges seamlessly.".to_string());
    app.log("F12: release control. q: quit.".to_string());

    loop {
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

    for shutdown in &shutdowns {
        let _ = shutdown.send(DaemonEvent::Shutdown).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    Ok(())
}
