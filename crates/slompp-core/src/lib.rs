//! The SLOMPP stack without a GUI: finds the server binaries (Apache, PHP,
//! MariaDB), lays out an XAMPP-style root (`htdocs`, `apache/`, `php/`,
//! `mysql/`, `phpMyAdmin/`), writes its configuration and starts, stops and
//! watches the servers.
//!
//! The servers themselves are the stock upstream programs; this crate only
//! manages them, like the XAMPP control panel does.

pub mod desktop;
pub mod layout;
pub mod module;
pub mod net;
pub mod phpmyadmin;
pub mod process;
pub mod settings;
pub mod setup;
pub mod stack;
pub mod templates;
pub mod toolchain;

pub use layout::Layout;
pub use module::Module;
pub use settings::{Ports, Settings};
pub use stack::{ModuleStatus, PortState, Problem, Stack};
pub use toolchain::Toolchain;
