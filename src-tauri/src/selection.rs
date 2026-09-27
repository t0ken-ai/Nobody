//! Automatic reading requires a recent completed mouse selection in an allowed
//! read-only context. Manual translation never passes through this gate.
use crate::document::Document;
use serde_json::Value;
use std::time::{Duration, Instant};

struct Candidate {
    identity: String,
    context: String,
    since: Instant,
}

#[derive(Default)]
pub struct Gate {
    seen_gesture: String,
    candidate: Option<Candidate>,
}

impl Gate {
    /// Consume gestures even in ignored contexts. A filename/input selection
    /// must not become eligible later merely because focus moves to prose.
    /// Retaining the consumed ID on clear also suppresses restored old ranges.
    pub fn clear(&mut self) {
        self.candidate = None;
    }

    /// Returns true once, after the selection is stable for 700 ms. The native
    /// age cap applies only when arming a gesture, allowing an already-armed
    /// selection to wait for the current translation without reusing old input.
    pub fn observe(&mut self, capture: &Value, identity: &str, now: Instant, busy: bool) -> bool {
        let gesture = capture["gestureId"].as_str().unwrap_or("");
        let context = capture["contextId"].as_str().unwrap_or("");
        let fresh = !gesture.is_empty() && gesture != self.seen_gesture;
        if fresh {
            self.seen_gesture = gesture.into();
            self.clear();
        }
        let text = capture["text"].as_str().unwrap_or("");
        if capture["autoEligible"] != true
            || capture["mouseDown"] == true
            || context.is_empty()
            || !useful_text(text)
        {
            self.clear();
            return false;
        }
        if fresh
            && capture["gestureAgeMs"]
                .as_u64()
                .is_some_and(|age| age <= 2000)
        {
            self.candidate = Some(Candidate {
                identity: identity.into(),
                context: context.into(),
                since: now,
            });
            return false;
        }
        let Some(candidate) = self.candidate.as_mut() else {
            return false;
        };
        if candidate.context != context {
            self.clear();
            return false;
        }
        if candidate.identity != identity {
            candidate.identity = identity.into();
            candidate.since = now;
            return false;
        }
        if busy || now.duration_since(candidate.since) < Duration::from_millis(700) {
            return false;
        }
        self.clear();
        true
    }
}

/// Reuse the existing deterministic protection rules before any progress UI or
/// engine call. Pure paths/numbers/protected code need no original-text popup;
/// ordinary short words remain eligible. This is not a general code detector.
fn useful_text(text: &str) -> bool {
    text.trim().chars().count() >= 2 && !Document::parse(text).prose().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn sample() -> Value {
        json!({"text":"Please retry.","contextId":"app:window:body","gestureId":"app:1", "gestureAgeMs":100,"autoEligible":true,"mouseDown":false})
    }
    #[test]
    fn requires_fresh_mouse_selection_then_fires_once() {
        let mut gate = Gate::default();
        let now = Instant::now();
        let mut c = sample();
        c["gestureId"] = json!("");
        assert!(!gate.observe(&c, "a", now, false));
        c["gestureId"] = json!("app:1");
        assert!(!gate.observe(&c, "a", now, false));
        assert!(!gate.observe(&c, "a", now + Duration::from_millis(699), false));
        assert!(gate.observe(&c, "a", now + Duration::from_millis(700), false));
        assert!(!gate.observe(&c, "a", now + Duration::from_secs(2), false));
        gate.clear();
        assert!(!gate.observe(&c, "a", now + Duration::from_secs(3), false));
    }
    #[test]
    fn editable_dialog_unknown_and_unlisted_contexts_consume_the_gesture() {
        let mut gate = Gate::default();
        let now = Instant::now();
        let mut c = sample();
        c["autoEligible"] = json!(false);
        assert!(!gate.observe(&c, "filename", now, false));
        c["autoEligible"] = json!(true);
        assert!(!gate.observe(&c, "body", now + Duration::from_secs(1), false));
        c["gestureId"] = json!("app:2");
        assert!(!gate.observe(&c, "body", now + Duration::from_secs(2), false));
        assert!(gate.observe(&c, "body", now + Duration::from_secs(3), false));
    }
    #[test]
    fn dragging_and_focus_changes_cancel_candidates_without_blocking_new_selection() {
        let mut gate = Gate::default();
        let now = Instant::now();
        let mut c = sample();
        assert!(!gate.observe(&c, "a", now, false));
        c["mouseDown"] = json!(true);
        assert!(!gate.observe(&c, "a", now + Duration::from_secs(1), false));
        c["mouseDown"] = json!(false);
        c["gestureId"] = json!("app:2");
        assert!(!gate.observe(&c, "b", now + Duration::from_secs(2), false));
        c["contextId"] = json!("other-window");
        assert!(!gate.observe(&c, "b", now + Duration::from_secs(3), false));
        assert!(!gate.observe(&c, "b", now + Duration::from_secs(4), false));
    }
    #[test]
    fn busy_work_and_changing_ranges_wait_but_old_gestures_never_arm() {
        let mut gate = Gate::default();
        let now = Instant::now();
        let mut c = sample();
        c["gestureAgeMs"] = json!(9000);
        assert!(!gate.observe(&c, "a", now, false));
        assert!(!gate.observe(&c, "a", now + Duration::from_secs(1), false));
        c["gestureId"] = json!("app:2");
        c["gestureAgeMs"] = json!(100);
        assert!(!gate.observe(&c, "a", now, false));
        assert!(!gate.observe(&c, "b", now + Duration::from_secs(1), false));
        assert!(!gate.observe(&c, "b", now + Duration::from_secs(2), true));
        assert!(gate.observe(&c, "b", now + Duration::from_secs(3), false));
    }
    #[test]
    fn protected_content_does_not_create_an_echo_popup() {
        for text in [
            "1000",
            "main.ts",
            "user_id",
            "https://example.com",
            "/tmp/report.pdf",
            "```sh\ngit status\n```",
        ] {
            assert!(!useful_text(text), "{text}");
        }
        assert!(useful_text("Retry"));
        assert!(useful_text("Please check src/main.rs."));
    }
}
