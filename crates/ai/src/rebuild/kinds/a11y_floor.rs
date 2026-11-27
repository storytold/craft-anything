//! The accessibility floor the person can hold a website build to (`build.a11y_floor`): judged on
//! the build's own capture, so a barrier the original shares still counts. Passing it is a floor,
//! not an accessibility audit.

use craft_anything_engine::settings::BuildSettings;
use craft_anything_engine::workspace::Claim;
use serde_json::Value;

/// Whether `page` of the build meets the floor: every shown control has a name, every shown
/// picture has an `alt` attribute (an empty one marks it decorative and passes) and the page says
/// its language. `None` when the floor is off, or for a capture without an accessibility outline
/// (older captures).
pub fn floor_claim(page: &str, built: &Value, settings: &BuildSettings) -> Option<Claim> {
    if !settings.a11y_floor {
        return None;
    }
    let a11y = built.get("a11y")?;
    let n = |k: &str| a11y.get(k).and_then(Value::as_u64).unwrap_or(0);
    let mut missing = Vec::new();
    if n("unnamed_controls") > 0 {
        missing.push(format!("{} control(s) without a name", n("unnamed_controls")));
    }
    if n("images_without_alt") > 0 {
        missing.push(format!("{} picture(s) with no alt attribute", n("images_without_alt")));
    }
    if a11y.get("lang").and_then(Value::as_str).is_none_or(str::is_empty) {
        missing.push("the page language isn't set".to_string());
    }
    Some(Claim::new(format!("{page} meets the accessibility floor"), missing.is_empty(), missing.join(", ")))
}
