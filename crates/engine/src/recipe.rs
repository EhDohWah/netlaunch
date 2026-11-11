//! Site recipes: the JSON that says how to log in to a kind of site.
//!
//! A recipe is a list of steps (go to the login page, wait for a field,
//! fill it, click) plus three rule sets that say what the page looks like
//! afterwards: logged in (`success`), asking for a 2FA code (`mfa`), or
//! rejecting the login (`failure`). See "Site recipes" in docs/SPEC.md.
//!
//! The built-in recipes live in `crates/engine/recipes/*.json` and are
//! compiled into the app with `include_str!`, so there are no files to
//! install or lose.

use serde::{Deserialize, Deserializer};

/// One login recipe, as read from JSON.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub id: String,
    /// Shown in the recipe dropdown.
    pub name: String,
    /// How long to wait, after the last step, for one of the rules to
    /// match. When it runs out the window is handed to the admin.
    pub timeout_ms: u64,
    pub steps: Vec<Step>,
    pub success: RuleSet,
    #[serde(default)]
    pub mfa: RuleSet,
    /// Added in M5: without it a rejected password looks like a timeout.
    #[serde(default)]
    pub failure: RuleSet,
}

/// One step. In JSON: `{ "action": "fill", "selector": "...", "value": "..." }`.
///
/// `#[serde(tag = "action")]` makes serde pick the variant from the
/// `action` field, which is how the spec writes steps.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub enum Step {
    /// Opens the site's saved login URL. A recipe can't name another URL,
    /// so it can never send the browser somewhere else.
    ///
    /// Written `Goto {}` rather than `Goto` so that serde rejects extra
    /// fields such as `"url"`: it ignores them on a variant with no braces.
    Goto {},
    /// Waits until an element matching `selector` is visible.
    Wait {
        selector: String,
        /// Defaults to [`DEFAULT_WAIT_MS`].
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Types into the first visible match. The origin is checked first.
    Fill { selector: String, value: FillValue },
    Click {
        selector: String,
        /// If no visible element matches `selector`, press Enter in this
        /// field instead. Many consoles (UniFi, Huawei switches) have a
        /// login button that isn't a `type=submit` button, but nearly all
        /// log in when Enter is pressed in the password field.
        #[serde(default)]
        else_press_enter_in: Option<String>,
    },
    /// Picks an option in a `<select>` by its value (e.g. a Proxmox realm).
    Select { selector: String, value: String },
    /// Presses a key in an element, such as `Enter`.
    Press { selector: String, key: String },
}

/// How long a `wait` step waits when the recipe doesn't say.
pub const DEFAULT_WAIT_MS: u64 = 15_000;

/// What a `fill` step types. Only two placeholders exist; anything else
/// is typed as written.
#[derive(Debug, Clone, PartialEq)]
pub enum FillValue {
    Username,
    /// Substituted inside the engine at fill time. The recipe never holds
    /// the password.
    Password,
    Literal(String),
}

impl<'de> Deserialize<'de> for FillValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        FillValue::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl FillValue {
    /// `{username}` and `{password}` must be the whole value. Any other
    /// brace is rejected, so a typo like `{pasword}` fails when the recipe
    /// loads instead of being typed into the page.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "{username}" => Ok(FillValue::Username),
            "{password}" => Ok(FillValue::Password),
            other if other.contains('{') || other.contains('}') => Err(format!(
                "fill value '{other}': the only placeholders are {{username}} and {{password}}, and each must be the whole value"
            )),
            other => Ok(FillValue::Literal(other.to_string())),
        }
    }
}

/// A set of rules that matches when any one of them does.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    #[serde(default)]
    pub any: Vec<Rule>,
}

/// One detection rule. In JSON: `{ "url_not_contains": "/login" }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// A visible element matches this CSS selector.
    Selector(String),
    /// The page's visible text contains this, ignoring case.
    Text(String),
    UrlContains(String),
    UrlNotContains(String),
    /// No visible element matches this CSS selector, and the page is on
    /// the site's own origin. For devices whose address doesn't change on
    /// login: "the password field is gone" means the site let you in.
    /// It has to hold twice, a second apart, so a page that is only
    /// reloading (after a wrong password, say) doesn't count.
    SelectorGone(String),
}

impl std::fmt::Display for Rule {
    /// For log lines and messages: says which rule matched.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rule::Selector(s) => write!(f, "selector `{s}`"),
            Rule::Text(t) => write!(f, "text \"{t}\""),
            Rule::UrlContains(u) => write!(f, "URL contains \"{u}\""),
            Rule::UrlNotContains(u) => write!(f, "URL doesn't contain \"{u}\""),
            Rule::SelectorGone(s) => write!(f, "`{s}` is gone"),
        }
    }
}

impl Recipe {
    /// Parses and checks a recipe.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let recipe: Recipe = serde_json::from_str(json).map_err(|e| format!("recipe JSON: {e}"))?;
        recipe.validate()?;
        Ok(recipe)
    }

    /// True if a step types the username. A recipe that doesn't (a device
    /// that only asks for a password) lets a site be saved without one.
    pub fn needs_username(&self) -> bool {
        self.steps.iter().any(|s| matches!(s, Step::Fill { value: FillValue::Username, .. }))
    }

    /// Checks what the JSON types alone can't.
    fn validate(&self) -> Result<(), String> {
        let id = &self.id;
        if !matches!(self.steps.first(), Some(Step::Goto {})) {
            return Err(format!("recipe {id}: the first step must be goto"));
        }
        if self.steps.iter().skip(1).any(|s| matches!(s, Step::Goto {})) {
            return Err(format!("recipe {id}: goto may only be the first step"));
        }
        if self.success.any.is_empty() {
            return Err(format!("recipe {id}: needs at least one success rule"));
        }
        if self.timeout_ms == 0 {
            return Err(format!("recipe {id}: timeout_ms must be more than 0"));
        }
        for step in &self.steps {
            let selector = match step {
                Step::Goto {} => continue,
                Step::Wait { selector, .. }
                | Step::Fill { selector, .. }
                | Step::Click { selector, .. }
                | Step::Select { selector, .. }
                | Step::Press { selector, .. } => selector,
            };
            if selector.trim().is_empty() {
                return Err(format!("recipe {id}: a step has an empty selector"));
            }
        }
        Ok(())
    }
}

/// The recipes compiled into the app: (id, JSON).
const BUILTIN: &[(&str, &str)] = &[
    ("generic-form", include_str!("../recipes/generic-form.json")),
    ("two-step-form", include_str!("../recipes/two-step-form.json")),
    ("password-only", include_str!("../recipes/password-only.json")),
];

/// The built-in recipe with this id.
pub fn builtin(id: &str) -> Option<Recipe> {
    let (_, json) = BUILTIN.iter().find(|(builtin_id, _)| *builtin_id == id)?;
    // The JSON is part of the build and a test parses every file, so this
    // only fails if someone edits a file without running the tests.
    Some(Recipe::from_json(json).expect("built-in recipe is valid"))
}

/// Every built-in recipe, for the recipe dropdown.
pub fn builtin_list() -> Vec<Recipe> {
    BUILTIN.iter().filter_map(|(id, _)| builtin(id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_has_its_file_id() {
        for (id, json) in BUILTIN {
            let recipe = Recipe::from_json(json).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(&recipe.id, id);
        }
        assert_eq!(builtin_list().len(), BUILTIN.len());
    }

    #[test]
    fn generic_form_matches_the_spec() {
        let recipe = builtin("generic-form").unwrap();
        assert_eq!(recipe.steps[0], Step::Goto {});
        assert!(recipe.steps.iter().any(|s| matches!(s, Step::Fill { value: FillValue::Password, .. })));
        assert_eq!(recipe.success.any, vec![Rule::UrlNotContains("/login".into())]);
        assert_eq!(recipe.failure.any, vec![Rule::Selector("[role=alert]".into())]);
    }

    #[test]
    fn password_only_types_no_username() {
        let recipe = builtin("password-only").unwrap();
        assert!(!recipe.needs_username());
        assert!(recipe.steps.iter().any(|s| matches!(s, Step::Fill { value: FillValue::Password, .. })));
        assert_eq!(recipe.success.any, vec![Rule::SelectorGone("input[type=password]".into())]);
        assert!(builtin("generic-form").unwrap().needs_username());
        assert!(builtin("two-step-form").unwrap().needs_username());
    }

    #[test]
    fn placeholders_must_be_exact() {
        assert_eq!(FillValue::parse("{username}"), Ok(FillValue::Username));
        assert_eq!(FillValue::parse("{password}"), Ok(FillValue::Password));
        assert_eq!(FillValue::parse("pam"), Ok(FillValue::Literal("pam".into())));
        assert!(FillValue::parse("{pasword}").is_err());
        assert!(FillValue::parse("x{password}").is_err());
        assert!(FillValue::parse("{password} ").is_err());
    }

    fn recipe_with(steps: &str) -> Result<Recipe, String> {
        Recipe::from_json(&format!(
            r#"{{ "id": "t", "name": "t", "timeout_ms": 1000, "steps": {steps},
                  "success": {{ "any": [ {{ "url_contains": "/home" }} ] }} }}"#
        ))
    }

    #[test]
    fn rejects_bad_recipes() {
        assert!(recipe_with(r#"[ { "action": "goto" } ]"#).is_ok());
        // Must start with goto, and only once.
        assert!(recipe_with(r#"[ { "action": "click", "selector": "b" } ]"#).is_err());
        assert!(recipe_with(r#"[ { "action": "goto" }, { "action": "goto" } ]"#).is_err());
        // A recipe can't choose a URL.
        assert!(recipe_with(r#"[ { "action": "goto", "url": "https://elsewhere" } ]"#).is_err());
        // Unknown actions and misspelled fields fail instead of being ignored.
        assert!(recipe_with(r#"[ { "action": "goto" }, { "action": "type", "selector": "a" } ]"#).is_err());
        assert!(recipe_with(r#"[ { "action": "goto" }, { "action": "click", "selecter": "a" } ]"#).is_err());
        assert!(recipe_with(r#"[ { "action": "goto" }, { "action": "click", "selector": " " } ]"#).is_err());
        assert!(recipe_with(r#"[ { "action": "goto" }, { "action": "fill", "selector": "a", "value": "{pw}" } ]"#).is_err());
    }

    #[test]
    fn needs_a_success_rule() {
        let json = r#"{ "id": "t", "name": "t", "timeout_ms": 1000, "steps": [ { "action": "goto" } ],
                        "success": { "any": [] } }"#;
        assert!(Recipe::from_json(json).is_err());
    }
}
