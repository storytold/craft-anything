//! The oracles a kind can be judged by, and the registry that holds them.
//!
//! A kind says what it is; the registry says which oracle can judge it. Adding an oracle is a
//! file and a registration line, the way the project's other seams work.

use craft_anything_engine::evidence::ToolError;
use craft_anything_engine::oracle::{BehaviorOracle, OracleCx, Script};
use craft_anything_engine::workspace::TargetRef;

use super::oracle_console::ConsoleOracle;
use super::oracle_game::GameOracle;
use super::oracle_native::NativeOracle;
use super::oracle_web::WebOracle;

/// The oracles this build knows, in the order they are tried.
pub struct Oracles {
    oracles: Vec<Box<dyn BehaviorOracle>>,
}

impl Default for Oracles {
    fn default() -> Self {
        Oracles::built_in()
    }
}

impl Oracles {
    /// The oracles this build knows.
    ///
    /// The order matters: the first that supports a target wins, so a more specific oracle
    /// comes before a more general one. A game's numbers are its own, not the output of running
    /// it, so the game oracle is tried before the console one.
    pub fn built_in() -> Self {
        let oracles: Vec<Box<dyn BehaviorOracle>> = vec![Box::new(WebOracle), Box::new(GameOracle), Box::new(NativeOracle), Box::new(ConsoleOracle)];
        Oracles { oracles }
    }

    /// The oracle with this id.
    pub fn by_id(&self, id: &str) -> Option<&dyn BehaviorOracle> {
        self.oracles.iter().find(|o| o.id() == id).map(|o| o.as_ref())
    }

    /// The oracles, for a test that checks them all.
    #[cfg(test)]
    pub fn oracles(&self) -> &[Box<dyn BehaviorOracle>] {
        &self.oracles
    }

    /// The oracles, by id.
    pub fn ids(&self) -> Vec<&'static str> {
        self.oracles.iter().map(|o| o.id()).collect()
    }

    /// The oracle that can judge this target, if one can.
    ///
    /// The first that supports it wins: the order is the order they are tried, so a more
    /// specific oracle comes before a more general one.
    pub fn for_target(&self, target: &TargetRef) -> Option<&dyn BehaviorOracle> {
        self.oracles.iter().find(|o| o.supports(target)).map(|o| o.as_ref())
    }

    /// Record the original's behaviour with whichever oracle can judge this target.
    ///
    /// `None` when no oracle applies, or when the one that does has nothing to record: a page
    /// with no control to act on, a program that will not run. Nothing is recorded, and nothing
    /// is judged.
    pub fn record(&self, cx: &OracleCx) -> Result<Option<Script>, ToolError> {
        let Some(oracle) = self.for_target(cx.target) else { return Ok(None) };
        oracle.record(cx)
    }

    /// Judge the build with whichever oracle recorded the original.
    ///
    /// An oracle that needs something the build does not have is refused plainly, before it
    /// starts: "there is no page to drive" is a better answer than a failure halfway through.
    pub fn judge(&self, cx: &OracleCx, script: &Script) -> Result<Vec<craft_anything_engine::oracle::Difference>, ToolError> {
        let Some(oracle) = self.for_target(cx.target) else {
            return Err(ToolError::invalid("No oracle can judge this target."));
        };
        if !oracle.needs().met_by(cx) {
            return Err(ToolError::invalid(format!("The {} oracle needs something this build does not have.", oracle.id())));
        }
        let build = oracle.replay(cx, script)?;
        Ok(oracle.compare(script, &build))
    }

    /// Whether an oracle can judge with what the caller has.
    pub fn can_judge(&self, cx: &OracleCx) -> bool {
        self.for_target(cx.target).is_some_and(|o| o.needs().met_by(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_oracles_are_named() {
        assert_eq!(Oracles::built_in().ids(), ["web", "game-mechanics", "native", "console"]);
    }

    #[test]
    fn a_website_is_judged_by_the_web_oracle() {
        let oracles = Oracles::built_in();
        let target = TargetRef::Website { url: "https://example.com".into() };
        assert_eq!(oracles.for_target(&target).map(|o| o.id()), Some("web"));
    }

    #[test]
    fn the_console_oracle_judges_a_program() {
        let oracles = Oracles::built_in();
        assert_eq!(oracles.by_id("console").map(|o| o.id()), Some("console"));
        // And it says so about a program.
        assert!(oracles.by_id("console").is_some_and(|o| o.supports(&TargetRef::Local { path: "/bin/ls".into() })));
    }

    /// A game and a console program are both a program on this computer, so the kind decides
    /// which oracle judges it. The oracle is asked by id, not picked by the target alone.
    #[test]
    fn the_oracle_is_asked_for_by_id() {
        let oracles = Oracles::built_in();
        assert!(oracles.by_id("console").is_some());
        assert!(oracles.by_id("no-such-oracle").is_none());
    }

    #[test]
    fn a_target_with_no_oracle_is_left_alone() {
        let oracles = Oracles::built_in();
        assert!(oracles.for_target(&TargetRef::None).is_none());
    }
}

#[cfg(test)]
mod conformance {
    use super::*;
    use craft_anything_engine::oracle::{Observation, OracleCx, Script};

    /// Every oracle answers for the same three questions: what it is called, what it can judge,
    /// and what it read. A new oracle that does not is not an oracle.
    fn conforms(oracle: &dyn BehaviorOracle) {
        assert!(!oracle.id().is_empty(), "an oracle has a name");
        // A target it does not support is refused, not guessed at.
        assert!(!oracle.supports(&TargetRef::None) || oracle.id() != "");
    }

    /// The three oracles this build knows are all oracles.
    #[test]
    fn every_built_in_oracle_conforms() {
        for oracle in Oracles::built_in().oracles.iter() {
            conforms(oracle.as_ref());
        }
    }

    /// An oracle's comparison is the engine's: the same probes, the same rules.
    #[test]
    fn the_comparison_is_the_engines_own() {
        let oracles = Oracles::built_in();
        let script = Script {
            version: 1,
            oracle: "test".into(),
            steps: vec![],
            probes: vec![craft_anything_engine::oracle::Probe {
                name: "exit_code".into(),
                kind: craft_anything_engine::oracle::ProbeKind::Exact,
                tolerance: None,
            }],
            original: vec![Observation {
                oracle: "test".into(),
                version: 1,
                readings: vec![Some(craft_anything_engine::oracle::Reading { value: serde_json::json!(0), engine: None })],
                gaps: vec![],
            }],
        };
        let build = vec![Observation {
            oracle: "test".into(),
            version: 1,
            readings: vec![Some(craft_anything_engine::oracle::Reading { value: serde_json::json!(1), engine: None })],
            gaps: vec![],
        }];
        for oracle in oracles.oracles.iter() {
            let differences = oracle.compare(&script, &build);
            assert_eq!(differences.len(), 1, "{}: {}", oracle.id(), differences.len());
            assert!(differences[0].is_failure(), "{}: a different value is a failure", oracle.id());
        }
    }

    /// A context with no build is still a context: the original's side.
    ///
    /// The build's capabilities are absent until the caller says what it is, so an oracle that
    /// needs one is told plainly that there is none.
    #[test]
    fn a_context_without_a_build_is_the_originals_side() {
        let call = |_: &str, _: serde_json::Value| Err(craft_anything_engine::evidence::ToolError::internal("not called"));
        let target = TargetRef::Local { path: "/bin/ls".into() };
        let cx = OracleCx::original(&call, &target, false);
        assert!(cx.program.is_none());
        assert!(cx.page.is_none());
        assert!(cx.report.is_none());
        assert!(!cx.deep);
    }

    /// Each capability says what the build is, and only that.
    #[test]
    fn each_capability_says_what_the_build_is() {
        use craft_anything_engine::oracle::{DrivesAPage, ReadsAReport, RunsAProgram, ThePage, TheProgram, TheReport};
        let program = TheProgram("/bin/ls".into());
        assert_eq!(program.program().to_str(), Some("/bin/ls"));
        let page = ThePage("http://127.0.0.1:8080/".into());
        assert_eq!(page.page_url(), "http://127.0.0.1:8080/");
        let report = TheReport("/tmp/report.json".into());
        assert_eq!(report.report_path().to_str(), Some("/tmp/report.json"));
    }
}

#[cfg(test)]
mod capabilities {
    use super::*;
    use craft_anything_engine::oracle::{Capability, Needs, OracleCx, ThePage, TheProgram, TheReport};

    fn context<'a>(
        call: &'a dyn Fn(&str, serde_json::Value) -> Result<craft_anything_engine::tool::ToolOutput, ToolError>,
        target: &'a TargetRef,
    ) -> OracleCx<'a> {
        OracleCx::original(call, target, false)
    }

    /// Each oracle says what it needs, and the needs are what it actually uses.
    #[test]
    fn each_oracle_says_what_it_needs() {
        let oracles = Oracles::built_in();
        let needs = |id: &str| oracles.by_id(id).map(|o| o.needs().capabilities().to_vec());
        assert_eq!(needs("console"), Some(vec![Capability::Program]));
        assert_eq!(needs("web"), Some(vec![Capability::Page]));
        assert_eq!(needs("game-mechanics"), Some(vec![Capability::Report]));
        assert_eq!(needs("native"), Some(vec![Capability::App]));
    }

    /// A context with only a program does not satisfy an oracle that needs a page.
    #[test]
    fn a_context_with_only_a_program_does_not_meet_a_pages_needs() {
        let call = |_: &str, _: serde_json::Value| Err(ToolError::internal("not called"));
        let target = TargetRef::Local { path: "/bin/ls".into() };
        let program = TheProgram("/bin/ls".into());
        let cx = context(&call, &target).with_program(&program);
        assert!(Needs::program().met_by(&cx));
        assert!(!Needs::page().met_by(&cx));
        assert!(!Needs::report().met_by(&cx));
        assert!(!Needs::app().met_by(&cx));
    }

    /// A context with a page meets the page's needs.
    #[test]
    fn a_context_with_a_page_meets_a_pages_needs() {
        let call = |_: &str, _: serde_json::Value| Err(ToolError::internal("not called"));
        let target = TargetRef::Website { url: "https://example.com".into() };
        let page = ThePage("http://127.0.0.1:8080/".into());
        let cx = context(&call, &target).with_page(&page);
        assert!(Needs::page().met_by(&cx));
        assert!(!Needs::program().met_by(&cx));
    }

    /// An oracle that needs something the build does not have is refused before it starts.
    #[test]
    fn an_oracle_without_what_it_needs_is_refused_plainly() {
        let call = |_: &str, _: serde_json::Value| Err(ToolError::internal("not called"));
        // A website target: the console oracle does not judge websites.
        let target = TargetRef::Website { url: "https://example.com".into() };
        let cx = context(&call, &target);
        let oracles = Oracles::built_in();
        assert!(!oracles.can_judge(&cx), "a website is not judged by the console oracle");
        let err = oracles.judge(&cx, &crate::rebuild::oracle_web::script(vec![], vec![])).expect_err("it is refused");
        assert!(err.message.contains("web"), "{}", err.message);
    }

    /// A report is a capability like any other.
    #[test]
    fn a_report_is_a_capability_like_any_other() {
        let call = |_: &str, _: serde_json::Value| Err(ToolError::internal("not called"));
        let target = TargetRef::Local { path: "/bin/ls".into() };
        let report = TheReport("/tmp/report.json".into());
        let cx = context(&call, &target).with_report(&report);
        assert!(Needs::report().met_by(&cx));
    }
}
