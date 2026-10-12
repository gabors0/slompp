//! The database admin tools SLOPP downloads on request: phpMyAdmin, which
//! XAMPP bundles, and Adminer, a single PHP file. Apache serves them at
//! `/phpmyadmin/` and `/adminer/`; both log in as root without a password.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::{Layout, adminer, phpmyadmin};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DbAdmin {
    PhpMyAdmin,
    Adminer,
}

impl DbAdmin {
    pub const ALL: [DbAdmin; 2] = [DbAdmin::PhpMyAdmin, DbAdmin::Adminer];

    pub fn name(self) -> &'static str {
        match self {
            DbAdmin::PhpMyAdmin => "phpMyAdmin",
            DbAdmin::Adminer => "Adminer",
        }
    }

    /// Lowercase, for element ids and the URL.
    pub fn id(self) -> &'static str {
        match self {
            DbAdmin::PhpMyAdmin => "phpmyadmin",
            DbAdmin::Adminer => "adminer",
        }
    }

    /// Where the download comes from, for tooltips.
    pub fn site(self) -> &'static str {
        match self {
            DbAdmin::PhpMyAdmin => "phpmyadmin.net",
            DbAdmin::Adminer => "adminer.org",
        }
    }

    pub fn installed(self, layout: &Layout) -> bool {
        match self {
            DbAdmin::PhpMyAdmin => phpmyadmin::installed(layout),
            DbAdmin::Adminer => adminer::installed(layout),
        }
    }

    /// The file people edit to configure it.
    pub fn config(self, layout: &Layout) -> PathBuf {
        match self {
            DbAdmin::PhpMyAdmin => layout.phpmyadmin_config(),
            DbAdmin::Adminer => layout.adminer_index(),
        }
    }

    /// Downloads the latest release into the stack, keeping an existing
    /// config. Blocking; run it off the UI thread.
    pub fn install(
        self,
        layout: &Layout,
        mysql_port: u16,
        progress: impl FnMut(Progress),
    ) -> anyhow::Result<Installed> {
        match self {
            DbAdmin::PhpMyAdmin => phpmyadmin::install(layout, mysql_port, progress),
            DbAdmin::Adminer => adminer::install(layout, progress),
        }
    }
}

/// What an installer is doing, for a progress display.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Checking,
    /// `fraction` is `None` when the server didn't say how big the file is.
    Downloading {
        version: String,
        fraction: Option<f32>,
    },
    Unpacking {
        version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    /// The installer added its alias to `httpd-slopp.conf`, so a running
    /// Apache needs a restart to serve it.
    pub apache_conf_changed: bool,
}

pub(crate) fn download(
    url: &str,
    to: &Path,
    mut progress: impl FnMut(Option<f32>),
) -> anyhow::Result<()> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .reader();
    let mut file =
        std::fs::File::create(to).with_context(|| format!("writing {}", to.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_reported = 0u64;
    progress(total.map(|_| 0.0));
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("downloading {url}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_reported >= 256 * 1024 {
            last_reported = done;
            progress(total.map(|t| (done as f32 / t as f32).min(1.0)));
        }
    }
    progress(total.map(|_| 1.0));
    Ok(())
}
