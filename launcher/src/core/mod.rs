//! Pure (UI-independent) launcher logic: filesystem layout, ROM verification,
//! settings persistence, and child-process management. Kept free of any Dioxus
//! types so it can be unit-tested on its own (`cargo test -p mb64-launcher`).

pub mod build;
pub mod game;
pub mod paths;
pub mod rom;
pub mod settings;
