// Copyright (c) 2022-2023 Yuki Kishimoto
// Copyright (c) 2023-2024 Rust Nostr Developers
// Distributed under the MIT software license

#![allow(clippy::drop_non_drop)]
#![allow(non_snake_case)]
#![allow(clippy::new_without_default)]

use wasm_bindgen::prelude::*;

pub mod error;
pub mod event;
pub mod key;
pub mod message;
pub mod nips;
pub mod types;

/// Run some stuff when the Wasm module is instantiated.
///
/// Right now, it does the following:
///
/// * Redirect Rust panics to JavaScript console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen(js_name = NostrLibrary)]
pub struct JsNostrLibrary;

#[wasm_bindgen(js_class = NostrLibrary)]
impl JsNostrLibrary {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self
    }

    #[wasm_bindgen(js_name = gitHashVersion)]
    pub fn git_hash_version(&self) -> Option<String> {
        std::env::var("GIT_HASH").ok()
    }
}
