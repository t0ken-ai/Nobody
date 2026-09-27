//! Selection-relative window state and geometry, independent of translation.
//! Native adapters report points on macOS and physical pixels on Windows;
//! `scale` converts the popover's logical size into that coordinate space.
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    /// Reject broken accessibility geometry before it reaches window APIs.
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
    }
}

#[derive(Clone, Debug)]
pub struct Anchor {
    rect: Rect,
    work: Rect,
    scale: f64,
    logical: bool,
    cursor: bool,
    visible: bool,
}

/// Identity deliberately excludes geometry: scrolling the same selection must
/// move the window without translating again. Native range IDs distinguish
/// equal text at different positions where the platform exposes that identity.
pub fn identity(capture: &Value, target: &str) -> String {
    format!(
        "{}:{target}:{}:{}:{}",
        capture["pid"], capture["contextId"], capture["selectionId"], capture["text"]
    )
}

/// macOS supplies its visible screen frame in the same points as AX. Windows
/// monitor work areas and UIA ranges are both physical pixels, including
/// negative monitor origins; only the popover's size is DPI-scaled.
pub fn anchor(app: &AppHandle, capture: &Value) -> Option<Anchor> {
    let raw = &capture["anchor"];
    let rect: Rect = serde_json::from_value(raw["rect"].clone()).ok()?;
    let logical = raw["space"] == "logical";
    let (work, scale) = if logical {
        (serde_json::from_value(raw["workArea"].clone()).ok()?, 1.0)
    } else {
        let monitor = app.monitor_from_point(rect.x, rect.y).ok()??;
        let area = monitor.work_area();
        (
            Rect {
                x: area.position.x as f64,
                y: area.position.y as f64,
                width: area.size.width as f64,
                height: area.size.height as f64,
            },
            monitor.scale_factor(),
        )
    };
    if !rect.valid() || !work.valid() || !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some(Anchor {
        rect,
        work,
        scale,
        logical,
        cursor: raw["kind"] == "cursor",
        visible: raw["visible"] != false,
    })
}

/// An invisible rectangle on the very first read is not proof that the user's
/// visible selection is off-screen: providers may return stale/zero geometry.
/// Begin near the pointer; only hide for scrolling after reliable bounds exist.
pub fn initial_anchor(app: &AppHandle, capture: &Value) -> Option<Anchor> {
    anchor(app, capture)
        .filter(|a| a.visible)
        .or_else(|| pointer_anchor(app))
}

/// Last-resort placement for errors before a source can be captured. Regular
/// translation requests use the native anchor, including its pointer fallback.
pub fn pointer_anchor(app: &AppHandle) -> Option<Anchor> {
    let point = app.cursor_position().ok()?;
    let monitor = app
        .monitor_from_point(point.x, point.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let divisor = if cfg!(target_os = "macos") {
        app.primary_monitor().ok()??.scale_factor()
    } else {
        1.0
    };
    Some(Anchor {
        rect: Rect {
            x: point.x / divisor,
            y: point.y / divisor,
            width: 1.0,
            height: 1.0,
        },
        work: Rect {
            x: area.position.x as f64 / divisor,
            y: area.position.y as f64 / divisor,
            width: area.size.width as f64 / divisor,
            height: area.size.height as f64 / divisor,
        },
        scale: if cfg!(target_os = "macos") {
            1.0
        } else {
            scale
        },
        logical: cfg!(target_os = "macos"),
        cursor: true,
        visible: true,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub logical: bool,
    pub scale: f64,
}

/// Prefer eight logical pixels above the range. If neither vertical side fits,
/// use the side with more room and shorten the scrollable popover to avoid the
/// selected text. All clamps use the screen's work area, not a zero origin.
fn place(anchor: &Anchor, requested_height: f64) -> Placement {
    let margin = 8.0 * anchor.scale;
    let work = anchor.work;
    let width = (380.0 * anchor.scale).min((work.width - 2.0 * margin).max(1.0));
    let desired = requested_height.clamp(120.0, 380.0) * anchor.scale;
    let top = work.y + margin;
    let bottom = work.y + work.height - margin;
    let above = (anchor.rect.y - margin - top).max(0.0);
    let below = (bottom - anchor.rect.y - anchor.rect.height - margin).max(0.0);
    let use_above = above >= desired || (below < desired && above >= below);
    let room = if use_above { above } else { below };
    // Very large selections can occupy the whole display. Keep controls usable
    // in that case and clamp to the work area instead of making a zero-height UI.
    let height = desired
        .min(room.max(120.0 * anchor.scale))
        .min((work.height - 2.0 * margin).max(1.0));
    let y = if use_above {
        anchor.rect.y - margin - height
    } else {
        anchor.rect.y + anchor.rect.height + margin
    };
    Placement {
        x: (anchor.rect.x + anchor.rect.width / 2.0 - width / 2.0)
            .clamp(
                work.x + margin,
                (work.x + work.width - margin - width).max(work.x + margin),
            )
            .round(),
        y: y.clamp(top, (bottom - height).max(top)).round(),
        width,
        height,
        logical: anchor.logical,
        scale: anchor.scale,
    }
}

struct Session {
    key: String,
    source: Value,
    anchor: Option<Anchor>,
    pinned: bool,
    dismissed: bool,
    present: bool,
}

#[derive(Default)]
pub struct Popover {
    serial: u64,
    active: Option<Session>,
    height: f64,
    last: Option<Placement>,
    visible: bool,
}
pub enum Update {
    None,
    Hide,
    Place(Placement),
    Resize(Placement),
}
impl Popover {
    /// A new request resets manual placement. The serial prevents a slow old
    /// translation from reappearing after selection changes or dismissal.
    pub fn begin(&mut self, key: String, anchor: Option<Anchor>, source: Option<&Value>) -> u64 {
        self.serial += 1;
        self.active = Some(Session {
            key,
            source: source.map(|capture| serde_json::json!({"pid":capture["pid"], "contextId":capture["contextId"]})).unwrap_or(Value::Null),
            anchor,
            pinned: false,
            dismissed: false,
            present: true,
        });
        self.height = 144.0;
        self.last = None;
        self.serial
    }
    pub fn current(&self, serial: u64) -> bool {
        self.serial == serial && self.active.as_ref().is_some_and(|s| !s.dismissed)
    }
    pub fn active(&self) -> bool {
        self.active.is_some()
    }
    /// Manual results may originate outside the automatic allowlist. Follow
    /// only that exact control, without authorizing new automatic requests.
    pub fn tracking(&self) -> Value {
        self.active
            .as_ref()
            .map(|session| session.source.clone())
            .unwrap_or(Value::Null)
    }
    /// Whole-input writes need not have a selected range. Their existing native
    /// replacement ticket validates changes; a read-only poll must not cancel
    /// them merely because there is no selection during translation.
    pub fn waiting_to_write(&self) -> bool {
        self.active.as_ref().is_some_and(|s| !s.present)
    }
    /// Successful write shortcuts stay invisible; only a failed replacement
    /// or translation reveals their result. Polling must respect that choice.
    pub fn present(&mut self, value: bool) {
        if let Some(active) = self.active.as_mut() {
            active.present = value;
        }
    }
    /// Continue observing during translation, without starting another job.
    /// Freeze pointer fallback so ordinary mouse movement does not chase it.
    pub fn observe(&mut self, key: &str, anchor: Option<Anchor>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.key != key {
            self.clear();
            return;
        }
        if let Some(next) = anchor {
            match active.anchor.as_mut() {
                Some(old) if old.cursor && (next.cursor || !next.visible) => {}
                Some(old) if !old.cursor && next.cursor => old.visible = false,
                _ => active.anchor = Some(next),
            }
        } else if let Some(old) = active.anchor.as_mut() {
            old.visible = false;
        }
    }
    /// Source focus/selection was lost. A later completion is no longer current.
    pub fn clear(&mut self) {
        self.serial += 1;
        self.active = None;
    }
    /// A capture error has no source to follow. Keep its diagnostic readable
    /// until closed or a valid new selection is observed.
    pub fn selection_lost(&mut self) {
        if !self.active.as_ref().is_some_and(|s| s.key == "error") {
            self.clear();
        }
    }
    /// Explicit closing suppresses this selection until a new request starts.
    pub fn dismiss(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.dismissed = true;
        }
    }
    /// Pin before starting the native drag loop so polling cannot fight it.
    pub fn pin(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.pinned = true;
        }
    }
    pub fn resize(&mut self, height: f64) {
        if height.is_finite() {
            self.height = height.clamp(120.0, 380.0).ceil();
        }
    }
    /// Produce window operations under the mutex, execute them after release.
    /// This prevents synchronous window queries from deadlocking the UI thread.
    pub fn update(&mut self) -> Update {
        let active = self.active.as_ref().filter(|s| !s.dismissed && s.present);
        let anchor = active.and_then(|s| s.anchor.as_ref()).filter(|a| a.visible);
        let Some((active, anchor)) = active.zip(anchor) else {
            self.last = None;
            return if std::mem::take(&mut self.visible) {
                Update::Hide
            } else {
                Update::None
            };
        };
        let mut next = place(anchor, self.height);
        if active.pinned && self.visible {
            // Once dragged, changing the source's position must not resize or
            // move the card; only new content may change its height.
            next.height = (self.height.clamp(120.0, 380.0) * anchor.scale)
                .min((anchor.work.height - 16.0 * anchor.scale).max(1.0));
        }
        let previous = self.last.replace(next);
        let was_visible = std::mem::replace(&mut self.visible, true);
        if active.pinned && was_visible {
            if previous.is_some_and(|p| p.height != next.height || p.width != next.width) {
                Update::Resize(next)
            } else {
                Update::None
            }
        } else if !was_visible || previous != Some(next) {
            Update::Place(next)
        } else {
            Update::None
        }
    }
}

/// Keep UI side effects outside the state mutex. Showing uses the configured
/// non-focusing window; dragging and copying are explicit user interactions.
pub fn apply(app: &AppHandle, update: Update) {
    let Some(window) = app.get_webview_window("result") else {
        return;
    };
    match update {
        Update::None => {}
        Update::Hide => {
            let _ = window.hide();
        }
        Update::Place(p) | Update::Resize(p) => {
            let _ = window.set_size(tauri::LogicalSize::new(
                p.width / p.scale,
                p.height / p.scale,
            ));
            if matches!(update, Update::Place(_)) {
                if p.logical {
                    let _ = window.set_position(tauri::LogicalPosition::new(p.x, p.y));
                } else {
                    let _ =
                        window.set_position(tauri::PhysicalPosition::new(p.x as i32, p.y as i32));
                }
                let _ = window.show();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracking_is_limited_to_source_control_and_is_cleared_with_session() {
        let capture = serde_json::json!({"pid":12,"contextId":"12:window:input","text":"Private input","selectionId":"range"});
        let mut popup = Popover::default();
        popup.begin(identity(&capture, "en"), None, Some(&capture));
        assert_eq!(
            popup.tracking(),
            serde_json::json!({"pid":12,"contextId":"12:window:input"})
        );
        let mut other = capture.clone();
        other["contextId"] = serde_json::json!("12:other-window:input");
        assert_ne!(identity(&capture, "en"), identity(&other, "en"));
        popup.observe(&identity(&other, "en"), None);
        assert!(popup.tracking().is_null());
    }
    fn fixture(x: f64, y: f64) -> Anchor {
        Anchor {
            rect: Rect {
                x,
                y,
                width: 160.0,
                height: 20.0,
            },
            work: Rect {
                x: 0.0,
                y: 25.0,
                width: 1000.0,
                height: 750.0,
            },
            scale: 1.0,
            logical: true,
            cursor: false,
            visible: true,
        }
    }
    #[test]
    fn above_below_and_display_edges_leave_text_visible() {
        let above = place(&fixture(300.0, 500.0), 180.0);
        assert_eq!((above.x, above.y), (190.0, 312.0));
        let below = place(&fixture(940.0, 30.0), 180.0);
        assert_eq!((below.x, below.y), (612.0, 58.0));
        let tall = place(&fixture(0.0, 300.0), 380.0);
        assert!(tall.y >= 328.0 && tall.y + tall.height <= 767.0);
    }
    #[test]
    fn negative_monitor_origin_and_high_dpi_keep_physical_units() {
        let mut a = fixture(-1800.0, 500.0);
        a.work = Rect {
            x: -2000.0,
            y: -200.0,
            width: 2000.0,
            height: 1400.0,
        };
        a.logical = false;
        a.scale = 2.0;
        let p = place(&a, 180.0);
        assert_eq!(p.width, 760.0);
        assert_eq!(p.y, 124.0);
        assert!(p.x >= -1984.0 && p.x + p.width <= -16.0);
    }
    #[test]
    fn late_results_closing_and_dragging_do_not_resurrect_or_snap() {
        let mut popup = Popover::default();
        let token = popup.begin("first".into(), Some(fixture(300.0, 500.0)), None);
        assert!(matches!(popup.update(), Update::Place(_)));
        popup.pin();
        popup.observe("first", Some(fixture(300.0, 400.0)));
        assert!(matches!(popup.update(), Update::None));
        popup.dismiss();
        assert!(!popup.current(token));
        assert!(matches!(popup.update(), Update::Hide));
        popup.observe("first", Some(fixture(300.0, 350.0)));
        assert!(matches!(popup.update(), Update::None));
        popup.begin("second".into(), Some(fixture(300.0, 500.0)), None);
        assert!(matches!(popup.update(), Update::Place(_)));
        popup.observe("third", Some(fixture(300.0, 500.0)));
        assert!(!popup.current(token));
        assert!(matches!(popup.update(), Update::Hide));
    }
    #[test]
    fn scrolling_out_of_view_hides_and_pointer_fallback_stays_put() {
        let mut popup = Popover::default();
        let mut a = fixture(300.0, 500.0);
        a.cursor = true;
        popup.begin("first".into(), Some(a.clone()), None);
        popup.update();
        a.rect.x += 100.0;
        popup.observe("first", Some(a));
        assert!(matches!(popup.update(), Update::None));
        let mut selection = fixture(300.0, 500.0);
        selection.visible = false;
        popup.observe("first", Some(selection.clone()));
        // An unreliable initial rectangle must not hide the pointer fallback.
        assert!(matches!(popup.update(), Update::None));
        popup.observe("first", Some(fixture(300.0, 500.0)));
        popup.update();
        popup.observe("first", Some(selection));
        assert!(matches!(popup.update(), Update::Hide));
    }

    #[test]
    fn whole_input_write_waits_without_showing_or_requiring_a_selection() {
        let mut popup = Popover::default();
        let token = popup.begin("input".into(), Some(fixture(300.0, 500.0)), None);
        popup.present(false);
        assert!(popup.waiting_to_write());
        assert!(popup.current(token));
        assert!(matches!(popup.update(), Update::None));
        popup.present(true);
        assert!(!popup.waiting_to_write());
        assert!(matches!(popup.update(), Update::Place(_)));
    }
}
