// The isolation application.
//
// Tauri loads this in a sandboxed iframe of its own and routes every message the
// frontend sends to the Rust core through the hook below. Nothing from the main frontend
// runs in here: it is a separate origin with a separate script context, so code that has
// managed to execute inside the application window cannot reach or rewrite this file.
//
// The value of that is not this hook, which passes messages through unchanged. It is that
// injected script cannot talk to the core directly any more. It has to send a message
// that arrives here first, which gives us one place to inspect and reject traffic if a
// reason to ever appears.
//
// Two rules for whoever edits this next. The hook must return the payload it was given,
// because dropping it silently breaks every command with no error anywhere. And nothing
// sensitive may be read or logged here, because this file is as much a part of the
// frontend as the rest.
window.__TAURI_ISOLATION_HOOK__ = (payload) => payload;
