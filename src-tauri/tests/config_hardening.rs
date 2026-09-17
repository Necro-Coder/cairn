//! Reads `tauri.conf.json` and asserts that the WebView hardening is still in place.
//!
//! This is the most important test in the scaffolding, and it exists because of how the
//! failure it guards against behaves. Loosening the content security policy, switching
//! off the isolation pattern or exposing the global Tauri object does not break anything.
//! The application starts, every screen works, every other test passes, and the defence
//! against injected script reaching the command boundary is simply gone.
//!
//! A configuration file nobody checks is a configuration file that drifts. So the file is
//! parsed here and every setting the threat model depends on is asserted by name, with
//! the reason written next to it, so that whoever changes one has to come and argue with
//! this file first.
// Every function in an integration test file is test code, but the lint that forbids
// panicking constructs only relaxes itself inside `#[cfg(test)]` modules and `#[test]`
// functions. The helpers below are neither, and a helper that cannot panic would have to
// return a Result that every assertion then has to unwrap, which buries the assertion.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Loads and parses the production Tauri configuration.
fn config() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

/// Reads a dotted path out of the configuration, failing with the path that was missing.
fn at<'a>(root: &'a Value, path: &str) -> &'a Value {
    let mut current = root;
    for segment in path.split('.') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("tauri.conf.json is missing {path}"));
    }
    current
}

#[test]
fn isolation_pattern_is_in_use() {
    // The isolation pattern puts a sandboxed frame between the application and the core.
    // Without it, any script running in the WebView can invoke a command directly, which
    // turns a cross-site scripting bug into full access to the vault.
    assert_eq!(
        at(&config(), "app.security.pattern.use"),
        "isolation",
        "the isolation pattern is the barrier between injected script and the commands"
    );
}

#[test]
fn the_isolation_application_actually_exists() {
    // A pattern pointing at a directory that is not there is a pattern that does nothing.
    let configuration = config();
    let directory = at(&configuration, "app.security.pattern.options.dir")
        .as_str()
        .expect("the isolation directory must be a string");
    let resolved = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(directory);
    assert!(
        resolved.join("index.html").is_file(),
        "the isolation application is missing its entry point at {}",
        resolved.display()
    );
}

#[test]
fn content_security_policy_allows_no_inline_or_evaluated_script() {
    let configuration = config();
    let policy = at(&configuration, "app.security.csp")
        .as_str()
        .expect("the content security policy must be a string");

    for forbidden in ["unsafe-inline", "unsafe-eval", "unsafe-hashes"] {
        assert!(
            !policy.contains(forbidden),
            "the content security policy must not contain {forbidden}: it is what stops \
             injected script from executing at all"
        );
    }
}

#[test]
fn content_security_policy_locks_down_every_directive_the_threat_model_names() {
    let configuration = config();
    let policy = at(&configuration, "app.security.csp")
        .as_str()
        .expect("the content security policy must be a string");

    let required = [
        (
            "default-src 'self'",
            "nothing loads from anywhere else by default",
        ),
        (
            "script-src 'self'",
            "script comes from the bundle and nowhere else",
        ),
        ("object-src 'none'", "no plugins, no embedded objects"),
        (
            "base-uri 'none'",
            "injected script cannot rewrite relative URLs",
        ),
        ("frame-ancestors 'none'", "the window cannot be framed"),
        ("form-action 'none'", "no form can post anywhere"),
    ];

    for (directive, why) in required {
        assert!(
            policy.contains(directive),
            "the content security policy is missing `{directive}`, which is what ensures {why}"
        );
    }
}

#[test]
fn there_is_no_global_tauri_object() {
    // `window.__TAURI__` would hand every command to anything running in the page.
    assert_eq!(
        at(&config(), "app.withGlobalTauri"),
        &Value::Bool(false),
        "withGlobalTauri exposes the whole command surface on the window object"
    );
}

#[test]
fn prototypes_are_frozen() {
    // Prototype pollution is one of the two realistic ways to compromise a WebView that
    // has no network origin, and freezing the prototypes closes it.
    assert_eq!(
        at(&config(), "app.security.freezePrototype"),
        &Value::Bool(true),
        "freezePrototype is the defence against prototype pollution"
    );
}

#[test]
fn the_asset_protocol_is_disabled() {
    let configuration = config();
    assert_eq!(
        at(&configuration, "app.security.assetProtocol.enable"),
        &Value::Bool(false),
        "the asset protocol serves arbitrary files to the WebView"
    );
    assert_eq!(
        at(&configuration, "app.security.assetProtocol.scope")
            .as_array()
            .map(Vec::len),
        Some(0),
        "an empty scope is the only scope a disabled asset protocol should have"
    );
}

#[test]
fn the_asset_content_security_policy_modification_is_not_disabled() {
    assert_eq!(
        at(
            &config(),
            "app.security.dangerousDisableAssetCspModification"
        ),
        &Value::Bool(false),
        "this setting stops Tauri from applying the policy to bundled assets"
    );
}

#[test]
fn drag_and_drop_into_the_window_is_off() {
    // A file dropped onto the window is untrusted input arriving through a path nothing
    // validates. Import happens through a command that checks what it is given.
    let configuration = config();
    let windows = at(&configuration, "app.windows")
        .as_array()
        .expect("at least one window must be configured");
    for window in windows {
        assert_eq!(
            window.get("dragDropEnabled"),
            Some(&Value::Bool(false)),
            "a window accepting dropped files accepts unvalidated input"
        );
    }
}

#[test]
fn the_window_has_no_system_decoration_and_the_label_the_controls_use() {
    // The header and the tab strip are drawn as one piece, which they cannot be with the
    // Windows title bar stacked on top of them. The cost is that dragging, minimising,
    // maximising and closing have to be implemented, and they are: four commands in
    // `window.rs` that act on the window with this label.
    //
    // The label is asserted here rather than only in `window.rs` because the two files are
    // the two halves of the same agreement. A rename on either side would leave every
    // window control silently refusing, with nothing on screen to say why.
    let configuration = config();
    let windows = at(&configuration, "app.windows")
        .as_array()
        .expect("at least one window must be configured");

    assert_eq!(windows.len(), 1, "this application has exactly one window");

    for window in windows {
        assert_eq!(
            window.get("label"),
            Some(&Value::String("main".to_owned())),
            "the window commands look the window up by this label"
        );
        assert_eq!(
            window.get("decorations"),
            Some(&Value::Bool(false)),
            "the title bar is drawn by the application, not by the system"
        );
    }
}

#[test]
fn the_window_cannot_be_made_smaller_than_the_layout_survives() {
    // 880 by 600 is where the design system stops drawing the geometric compositions and
    // the tab cap drops from six to four. Below it the layout is not merely cramped, it is
    // a layout nobody designed, and with no system decoration there is no snapping
    // behaviour left to rescue it.
    let configuration = config();
    let windows = at(&configuration, "app.windows")
        .as_array()
        .expect("at least one window must be configured");

    for window in windows {
        assert_eq!(
            window.get("minWidth").and_then(Value::as_u64),
            Some(880),
            "880 is the width the interface is designed down to"
        );
        assert_eq!(
            window.get("minHeight").and_then(Value::as_u64),
            Some(600),
            "600 is the height the interface is designed down to"
        );
        assert_eq!(
            window.get("resizable"),
            Some(&Value::Bool(true)),
            "an undecorated window that cannot be resized is a window with no way out"
        );
    }
}

#[test]
fn the_display_name_matches_the_one_the_core_reports() {
    // The name on the window title bar and the name the core returns come from different
    // files. This is what keeps them the same.
    assert_eq!(
        at(&config(), "productName"),
        cairn_lib::commands::app_info::DISPLAY_NAME,
        "productName and DISPLAY_NAME have drifted apart"
    );
}

#[test]
fn the_bundle_identifier_is_the_one_that_was_decided() {
    // Changing this after the application is installed on a phone produces a second,
    // unrelated application rather than an update, so it is pinned by a test.
    assert_eq!(
        at(&config(), "identifier"),
        "io.github.necro-coder.cairn",
        "the bundle identifier is permanent once the application has been installed"
    );
}

#[test]
fn the_configuration_does_not_pin_a_version_of_its_own() {
    // The version has exactly one source, the workspace manifest. A copy here would be a
    // second place to forget.
    assert!(
        config().get("version").is_none(),
        "tauri.conf.json must not carry its own version; it is derived from Cargo.toml"
    );
}

/// The only core permissions this application is allowed to grant, and why.
///
/// Listening is how the interface finds out that the core closed the vault on its own,
/// which has to reach the screen the moment it happens rather than at the next poll.
/// Stopping listening is the other half of the same call: the function `listen` hands back
/// is what the interface calls when it goes away, and it invokes the core the same way.
///
/// Nothing else belongs here. A custom command is reachable without a capability entry, so
/// every command this application exposes needs nothing from this list.
const ALLOWED_PERMISSIONS: [&str; 2] = ["core:event:allow-listen", "core:event:allow-unlisten"];

#[test]
fn no_capability_grants_a_core_permission_outside_the_allowed_list() {
    // Every permission granted here is a core API that script injected into the WebView can
    // ask for. `core:default` is a convenience bundle rather than a minimal list, and it
    // includes path resolution, which is how an attacker turns a scripting bug into the
    // name of the account running the application. The two that are allowed can do one
    // thing: subscribe to events this application emits, and stop. Adding a third is a
    // deliberate act that has to come through this test.
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()));

    let mut checked = 0;
    for entry in entries {
        let path = entry.expect("directory entries are readable").path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }

        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let capability: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));

        let permissions = capability
            .get("permissions")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{} declares no permissions array", path.display()));

        for permission in permissions {
            let name = permission.as_str().unwrap_or_else(|| {
                panic!(
                    "{} grants a permission that is not a string",
                    path.display()
                )
            });

            assert!(
                ALLOWED_PERMISSIONS.contains(&name),
                "{} grants `{name}`, which is not one of {ALLOWED_PERMISSIONS:?}. Every entry \
                 here is a core API that script injected into the WebView could call: \
                 `core:default` alone would hand it path resolution, which is how an attacker \
                 learns the account name. Adding one is a deliberate decision that belongs in \
                 the description next to it, and in this assertion.",
                path.display()
            );
        }
        checked += 1;
    }

    assert!(checked > 0, "no capability files were found to check");
}

#[test]
fn the_window_may_listen_for_the_event_that_says_the_vault_closed() {
    // The other direction of the test above, and the one that would have caught a real
    // defect. With an empty list the subscription is refused at runtime: the call that sets
    // it up rejects, the interface never hears that the core closed the vault, and the
    // timer that would have noticed anyway is never started because it was installed after
    // the subscription. None of that shows in a browser preview, which has no core to
    // refuse anything, so the only thing standing between that and a release is this.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let capability: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));

    let permissions = capability
        .get("permissions")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{} declares no permissions array", path.display()));

    for required in ALLOWED_PERMISSIONS {
        assert!(
            permissions
                .iter()
                .any(|granted| granted.as_str() == Some(required)),
            "{} does not grant `{required}`. Without it the interface cannot subscribe to \
             the event the core emits when it closes the vault, and a vault that closed \
             itself would go on looking open until something else asked.",
            path.display()
        );
    }
}
