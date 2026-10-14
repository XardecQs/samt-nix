/// App version from the repo's top-level `VERSION` file, injected at build time
/// by `build.rs`. The Cargo manifest version stays at `0.0.0` on purpose so a
/// release bump does not invalidate the cached build dependencies.
pub const VERSION: &str = env!("GTA_MO_VERSION");

pub mod color;
pub mod config;
pub mod conflicts;
pub mod constraints;
pub mod db;
pub mod games;
pub mod launcher;
pub mod log;
pub mod meta;
pub mod modloader;
pub mod mount_state;
pub mod overlay;
pub mod prefix_ram;
pub mod preload;
pub mod resolver;
pub mod umu;
pub mod upper;
pub mod userdata;
