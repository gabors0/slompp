//! The SLOPP stack without a GUI: finds the server binaries (Apache, PHP,
//! MariaDB, Mailpit), lays out an XAMPP-style root (`htdocs`, `apache/`, `php/`,
//! `mysql/`, `phpMyAdmin/`, `adminer/`), writes its configuration and
//! starts, stops and watches the servers.
//!
//! The servers themselves are the stock upstream programs; this crate only
//! manages them, like the XAMPP control panel does.

pub mod adminer;
pub mod brew;
pub mod dbadmin;
pub mod desktop;
pub mod install;
pub mod layout;
pub mod linux;
pub mod mailpit;
pub mod module;
pub mod net;
pub mod phpmyadmin;
pub mod process;
pub mod settings;
pub mod setup;
pub mod stack;
pub mod templates;
pub mod toolchain;
pub mod update;
pub mod winget;

pub use dbadmin::DbAdmin;
pub use layout::Layout;
pub use module::Module;
pub use settings::{Ports, Settings};
pub use stack::{ModuleStatus, PortState, Problem, Stack};
pub use toolchain::Toolchain;
