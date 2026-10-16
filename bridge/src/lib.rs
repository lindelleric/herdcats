pub mod adapter;
pub mod claude;
pub mod codex;
pub mod follower;
pub mod resolver;

pub type Result<T> = std::result::Result<T, &'static str>;
pub mod herdr;
pub mod service;

pub mod content;
