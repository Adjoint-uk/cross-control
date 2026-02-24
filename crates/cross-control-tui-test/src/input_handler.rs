//! Maps terminal keyboard events to mock capture events.

use cross_control_types::{ButtonState, CapturedEvent, DeviceId, InputEvent, KeyCode};
use crossterm::event::{KeyCode as CtKeyCode, KeyEvent};
use tokio::sync::mpsc;

use crate::app::AppState;

/// Mouse movement step size in pixels per arrow key press.
const MOUSE_STEP: i32 = 80;

/// Handle a terminal key event, converting it to mock capture events.
///
/// Returns `true` if the app should quit.
pub async fn handle_key(
    key: KeyEvent,
    feed: &mpsc::Sender<CapturedEvent>,
    app: &mut AppState,
) -> bool {
    // Quit on 'q'
    if key.code == CtKeyCode::Char('q') && key.modifiers.is_empty() {
        return true;
    }

    match key.code {
        // Arrow keys -> MouseMove
        CtKeyCode::Left => {
            send_mouse_move(feed, -MOUSE_STEP, 0, app).await;
        }
        CtKeyCode::Right => {
            send_mouse_move(feed, MOUSE_STEP, 0, app).await;
        }
        CtKeyCode::Up => {
            send_mouse_move(feed, 0, -MOUSE_STEP, app).await;
        }
        CtKeyCode::Down => {
            send_mouse_move(feed, 0, MOUSE_STEP, app).await;
        }

        // F12 -> release hotkey
        CtKeyCode::F(12) => {
            send_key(feed, KeyCode::F12, ButtonState::Pressed, app).await;
            send_key(feed, KeyCode::F12, ButtonState::Released, app).await;
        }

        // Escape
        CtKeyCode::Esc => {
            send_key(feed, KeyCode::Escape, ButtonState::Pressed, app).await;
            send_key(feed, KeyCode::Escape, ButtonState::Released, app).await;
        }

        // Letter keys
        CtKeyCode::Char(c) if c.is_ascii_alphabetic() => {
            if let Some(kc) = char_to_keycode(c) {
                send_key(feed, kc, ButtonState::Pressed, app).await;
                send_key(feed, kc, ButtonState::Released, app).await;
            }
        }

        _ => {}
    }

    false
}

async fn send_mouse_move(feed: &mpsc::Sender<CapturedEvent>, dx: i32, dy: i32, app: &mut AppState) {
    app.log(format!("A: MouseMove dx={dx} dy={dy}"));
    let event = CapturedEvent {
        device_id: DeviceId(2),
        timestamp_us: timestamp(),
        event: InputEvent::MouseMove { dx, dy },
    };
    let _ = feed.send(event).await;
}

async fn send_key(
    feed: &mpsc::Sender<CapturedEvent>,
    code: KeyCode,
    state: ButtonState,
    app: &mut AppState,
) {
    app.log(format!("A: Key({code:?}, {state:?})"));
    let event = CapturedEvent {
        device_id: DeviceId(1),
        timestamp_us: timestamp(),
        event: InputEvent::Key { code, state },
    };
    let _ = feed.send(event).await;
}

#[allow(clippy::cast_possible_truncation)]
fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn char_to_keycode(c: char) -> Option<KeyCode> {
    match c.to_ascii_lowercase() {
        'a' => Some(KeyCode::KeyA),
        'b' => Some(KeyCode::KeyB),
        'c' => Some(KeyCode::KeyC),
        'd' => Some(KeyCode::KeyD),
        'e' => Some(KeyCode::KeyE),
        'f' => Some(KeyCode::KeyF),
        'g' => Some(KeyCode::KeyG),
        'h' => Some(KeyCode::KeyH),
        'i' => Some(KeyCode::KeyI),
        'j' => Some(KeyCode::KeyJ),
        'k' => Some(KeyCode::KeyK),
        'l' => Some(KeyCode::KeyL),
        'm' => Some(KeyCode::KeyM),
        'n' => Some(KeyCode::KeyN),
        'o' => Some(KeyCode::KeyO),
        'p' => Some(KeyCode::KeyP),
        'r' => Some(KeyCode::KeyR),
        's' => Some(KeyCode::KeyS),
        't' => Some(KeyCode::KeyT),
        'u' => Some(KeyCode::KeyU),
        'v' => Some(KeyCode::KeyV),
        'w' => Some(KeyCode::KeyW),
        'x' => Some(KeyCode::KeyX),
        'y' => Some(KeyCode::KeyY),
        'z' => Some(KeyCode::KeyZ),
        _ => None,
    }
}
