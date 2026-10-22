pub mod adapter;
pub mod claude;
pub mod codex;
pub mod follower;
pub mod resolver;

pub type Result<T> = std::result::Result<T, &'static str>;
pub mod herdr;
pub mod service;

pub mod codex_config;
pub mod content;

pub mod hooks;
pub mod spool;

pub mod approval_screen;

pub mod approval_match;

pub mod question_match;

pub mod question_screen;

pub mod question_keys;
