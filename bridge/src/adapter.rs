use crate::{Result, follower::Follower, herdr::Pane};
use std::fs::File;

pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, agent: &str) -> bool;
    fn is_available(&self) -> bool;
    fn resolve_and_open(&self, pane: &Pane) -> Result<(String, File)>;
    fn new_follower(&self, include_content: bool) -> Follower;
}
