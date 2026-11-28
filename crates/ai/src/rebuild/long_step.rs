//! A builder step that has been going a long time (#2429). A shell command such as `find /` or an
//! endless `ffmpeg` can hold a build for an hour while the feed shows nothing new, so the run says
//! in the feed which step it is, how long it has run and what the person can do about it.
//!
//! Nothing here stops a step: a real build can be long, and a time limit on the builder's own
//! commands is a decision left to the maintainers (docs/ROADMAP.md).

use super::activity::{self, Action};
use super::*;

/// How long one builder step (a tool call or shell command) runs before the feed says so.
pub const LONG_STEP_AFTER: Duration = Duration::from_secs(10 * 60);
/// How long after a notice the same step is named again while it still runs.
pub const LONG_STEP_REPEAT: Duration = Duration::from_secs(30 * 60);
/// The most steps followed at once; a builder that never finishes its calls can't grow this.
const MOST_STEPS: usize = 256;

/// When a step is first named, and how long until it is named again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LongStepLimits {
    pub after: Duration,
    pub repeat: Duration,
}

impl Default for LongStepLimits {
    fn default() -> Self {
        LongStepLimits { after: LONG_STEP_AFTER, repeat: LONG_STEP_REPEAT }
    }
}

/// The tools whose time is the time of the steps inside them: a subagent's own calls are followed
/// one by one, so the hand-off itself isn't named.
const HANDOFF_TOOLS: &[&str] = &["Task", "Agent"];

#[derive(Debug)]
struct Going {
    call_id: String,
    action: Action,
    since: Instant,
    /// When it is named next; `None` once that time can't be reached.
    next: Option<Instant>,
}

/// The builder's steps still running in one turn, and when each one is due a notice.
#[derive(Debug)]
pub(super) struct LongSteps {
    after: Duration,
    repeat: Duration,
    going: Vec<Going>,
}

impl LongSteps {
    pub(super) fn new(limits: LongStepLimits) -> LongSteps {
        LongSteps { after: limits.after, repeat: limits.repeat, going: Vec::new() }
    }

    /// Take in one of the builder's events, seen at `now`. `roots` are left out of the paths a
    /// step names, as the activity log does.
    pub(super) fn observe(&mut self, e: &AgentEvent, now: Instant, roots: &[PathBuf]) {
        match e {
            AgentEvent::ToolStarted { tool, .. } if activity::is_checklist(tool) || HANDOFF_TOOLS.contains(&tool.as_str()) => {}
            AgentEvent::ToolStarted { call_id, tool, input } => {
                self.going.retain(|g| g.call_id != *call_id);
                if self.going.len() >= MOST_STEPS {
                    self.going.remove(0);
                }
                let action = activity::describe(tool, input, roots);
                self.going.push(Going { call_id: call_id.clone(), action, since: now, next: now.checked_add(self.after) });
            }
            AgentEvent::ToolFinished { call_id, .. } => self.going.retain(|g| g.call_id != *call_id),
            AgentEvent::TurnDone { .. } => self.clear(),
            _ => {}
        }
    }

    /// Forget every step: the turn ended, or the run is pausing or stopping.
    pub(super) fn clear(&mut self) {
        self.going.clear();
    }

    /// The notices due at `now`, one per step, each said once and then again only after
    /// the repeat time ([`LONG_STEP_REPEAT`] in a run).
    pub(super) fn due(&mut self, now: Instant) -> Vec<String> {
        let repeat = self.repeat;
        self.going
            .iter_mut()
            .filter(|g| g.next.is_some_and(|at| now >= at))
            .map(|g| {
                g.next = now.checked_add(repeat);
                notice(&g.action, now.saturating_duration_since(g.since))
            })
            .collect()
    }
}

/// What the feed says about a step that has run for `took`.
fn notice(action: &Action, took: Duration) -> String {
    format!(
        "One step has been running for {}: {}. Nothing stops it on its own; if it shouldn't take this long, pause or stop the build.",
        minutes(took),
        action.words()
    )
}

/// A long time in minutes and hours ("12 min", "1 h 5 min").
fn minutes(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MIN: Duration = Duration::from_secs(60);

    fn started(id: &str, tool: &str, input: Value) -> AgentEvent {
        AgentEvent::ToolStarted { call_id: id.into(), tool: tool.into(), input }
    }

    fn bash(id: &str, command: &str) -> AgentEvent {
        started(id, "Bash", json!({ "command": command }))
    }

    fn finished(id: &str) -> AgentEvent {
        AgentEvent::ToolFinished { call_id: id.into(), tool: "Bash".into(), ok: true, summary: String::new(), evidence_id: None }
    }

    fn turn_done() -> AgentEvent {
        AgentEvent::TurnDone { backend: "x".into(), input_tokens: 0, output_tokens: 0, tokens_unknown: true, cost_usd: None, cost_is_estimate: false }
    }

    fn steps() -> LongSteps {
        LongSteps::new(LongStepLimits { after: 10 * MIN, repeat: 30 * MIN })
    }

    #[test]
    fn a_long_step_is_named_once_then_sparsely() {
        let t0 = Instant::now();
        let mut long = steps();
        long.observe(&bash("a", "find / -name game.pak"), t0, &[]);
        assert!(long.due(t0 + 9 * MIN).is_empty());
        let first = long.due(t0 + 10 * MIN);
        assert_eq!(first.len(), 1, "{first:?}");
        assert!(first[0].contains("10 min") && first[0].contains("Running find / -name game.pak"), "{}", first[0]);
        assert!(first[0].contains("pause or stop"), "{}", first[0]);
        // Not again on every poll.
        assert!(long.due(t0 + 11 * MIN).is_empty());
        assert!(long.due(t0 + 39 * MIN).is_empty());
        let again = long.due(t0 + 40 * MIN);
        assert_eq!(again.len(), 1);
        assert!(again[0].contains("40 min"), "{}", again[0]);
        assert!(long.due(t0 + 41 * MIN).is_empty());
        assert!(long.due(t0 + 70 * MIN)[0].contains("1 h 10 min"));
    }

    #[test]
    fn a_short_step_is_never_named() {
        let t0 = Instant::now();
        let mut long = steps();
        long.observe(&bash("a", "cargo build"), t0, &[]);
        assert!(long.due(t0 + 5 * MIN).is_empty());
        long.observe(&finished("a"), t0 + 6 * MIN, &[]);
        assert!(long.due(t0 + 60 * MIN).is_empty());
    }

    #[test]
    fn concurrent_steps_are_followed_separately() {
        let t0 = Instant::now();
        let mut long = steps();
        long.observe(&bash("a", "ffmpeg -i in.mp4 out.mp4"), t0, &[]);
        // A subagent's own call, started later beside it.
        long.observe(&started("sub", "Agent", json!({"description": "look for the data files"})), t0, &[]);
        long.observe(&bash("b", "find / -name x"), t0 + 5 * MIN, &[]);
        let at_10 = long.due(t0 + 10 * MIN);
        assert_eq!(at_10.len(), 1, "the hand-off itself isn't named: {at_10:?}");
        assert!(at_10[0].contains("ffmpeg"));
        long.observe(&finished("a"), t0 + 12 * MIN, &[]);
        let at_15 = long.due(t0 + 15 * MIN);
        assert_eq!(at_15.len(), 1);
        assert!(at_15[0].contains("find / -name x") && at_15[0].contains("10 min"), "{}", at_15[0]);
        assert!(long.due(t0 + 30 * MIN).is_empty());
    }

    #[test]
    fn an_ended_or_stopped_step_is_not_named_afterwards() {
        let t0 = Instant::now();
        let mut ended = steps();
        ended.observe(&bash("a", "sleep 9999"), t0, &[]);
        ended.observe(&turn_done(), t0 + MIN, &[]);
        assert!(ended.due(t0 + 60 * MIN).is_empty(), "the turn ended");

        let mut stopped = steps();
        stopped.observe(&bash("a", "sleep 9999"), t0, &[]);
        stopped.clear();
        assert!(stopped.due(t0 + 60 * MIN).is_empty(), "paused or stopped");

        let mut finished_late = steps();
        finished_late.observe(&bash("a", "sleep 9999"), t0, &[]);
        assert_eq!(finished_late.due(t0 + 10 * MIN).len(), 1);
        finished_late.observe(&finished("a"), t0 + 20 * MIN, &[]);
        assert!(finished_late.due(t0 + 60 * MIN).is_empty());
    }

    #[test]
    fn the_command_is_redacted_and_the_project_left_out() {
        let t0 = Instant::now();
        let mut long = steps();
        let roots = [PathBuf::from("/projects/x/app")];
        let key = "sk-test-not-a-real-key";
        long.observe(&bash("a", &format!("curl -H 'x-api-key: {key}' /projects/x/app/data.bin\nsecond line")), t0, &roots);
        let said = long.due(t0 + 10 * MIN);
        assert!(!said[0].contains(key) && !said[0].contains("/projects/x/app") && !said[0].contains("second line"), "{}", said[0]);
    }

    #[test]
    fn checklist_updates_and_unknown_ids_are_ignored() {
        let t0 = Instant::now();
        let mut long = steps();
        long.observe(&started("t", "TodoWrite", json!({"todos": []})), t0, &[]);
        long.observe(&finished("nobody"), t0, &[]);
        assert!(long.due(t0 + 60 * MIN).is_empty());
    }

    #[test]
    fn many_unfinished_steps_stay_bounded() {
        let t0 = Instant::now();
        let mut long = steps();
        for i in 0..(MOST_STEPS + 50) {
            long.observe(&bash(&format!("c{i}"), "true"), t0, &[]);
        }
        assert_eq!(long.due(t0 + 10 * MIN).len(), MOST_STEPS);
    }

    /// A builder that starts a long command and a short one, finishes the short one on its next
    /// poll and the long one once `long_for` has passed, then ends its turn.
    struct SlowCommand {
        began: Option<Instant>,
        long_for: Duration,
        polls: usize,
        done: bool,
        /// Cancelled once both steps are under way, as Pause or Stop does.
        stop_after_start: Option<CancelToken>,
    }

    impl Conversation for SlowCommand {
        fn send(&mut self, _: &str) {}

        fn poll(&mut self) -> Vec<AgentEvent> {
            self.polls += 1;
            let began = *self.began.get_or_insert_with(Instant::now);
            if self.polls == 1 {
                if let Some(stop) = &self.stop_after_start {
                    stop.cancel();
                }
                return vec![bash("long", "find / -name game.pak"), bash("short", "ls app")];
            }
            if self.polls == 2 {
                return vec![finished("short")];
            }
            if self.done || began.elapsed() < self.long_for {
                return Vec::new();
            }
            self.done = true;
            vec![finished("long"), turn_done()]
        }

        fn answer_consent(&mut self, _: &str, _: bool) {}
        fn stop(&mut self) {}
        fn is_busy(&self) -> bool {
            !self.done
        }
    }

    fn statuses(rx: &std::sync::mpsc::Receiver<RunEvent>) -> Vec<String> {
        rx.try_iter()
            .filter_map(|e| match e {
                RunEvent::Agent(AgentEvent::Status { text }) => Some(text),
                _ => None,
            })
            .collect()
    }

    fn run_turn(stop: bool) -> (Result<String, ToolError>, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "Long step", "Build an app", TargetRef::None).unwrap();
        let mut worker = crate::rebuild::tests::check_worker(&ws, false);
        let (tx, rx) = std::sync::mpsc::channel();
        worker.tx = tx;
        worker.long_step = LongStepLimits { after: Duration::from_millis(600), repeat: Duration::from_secs(3600) };
        let mut agent =
            SlowCommand { began: None, long_for: Duration::from_millis(1500), polls: 0, done: false, stop_after_start: stop.then(|| worker.stop.clone()) };
        let result = worker.converse(&mut ws, &mut agent, "Build it");
        (result, statuses(&rx))
    }

    #[test]
    fn a_running_turn_names_its_long_step_once_in_the_feed() {
        let (result, said) = run_turn(false);
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(said.len(), 1, "one notice, for the long step only: {said:?}");
        assert!(said[0].contains("find / -name game.pak") && !said[0].contains("ls app"), "{}", said[0]);
    }

    #[test]
    fn a_paused_or_stopped_turn_names_nothing() {
        let (result, said) = run_turn(true);
        assert!(result.is_err(), "the stopped turn ends with the stop: {result:?}");
        assert!(said.is_empty(), "{said:?}");
    }

    #[test]
    fn a_far_off_time_does_not_overflow() {
        let t0 = Instant::now();
        let mut long = LongSteps::new(LongStepLimits { after: Duration::MAX, repeat: Duration::MAX });
        long.observe(&bash("a", "x"), t0, &[]);
        assert!(long.due(t0 + 60 * MIN).is_empty());
    }
}
