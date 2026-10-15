//! Configuration files for a new stack root.
//!
//! The files people edit (`httpd.conf`, `php.ini`, `my.cnf`, ...) are written
//! once and then left alone. They hard-code only the stack root; anything
//! that depends on the machine (where Apache's modules are, how PHP runs,
//! ports) is passed on the command line at start, as Apache `Define`s and
//! MariaDB options, so editing ports in the panel never touches them.

use std::path::Path;

use crate::Layout;

/// A path as Apache and PHP want it: forward slashes, even on Windows.
pub fn conf_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn httpd_conf(layout: &Layout) -> String {
    let root = conf_path(layout.root());
    let apache = conf_path(&layout.apache());
    let htdocs = conf_path(&layout.htdocs());
    let run = conf_path(&layout.run());
    format!(
        r#"# Apache httpd configuration for SLOPP.
#
# This file is yours to edit; SLOPP never rewrites it. The control panel
# passes these when it starts Apache:
#
#   SLOPP_MODULES      the directory holding Apache's mod_*.so
#   HTTP_PORT          the HTTP port from the panel's settings
#   HTTPS_PORT         the HTTPS port from the panel's settings
#   SSL                defined when a certificate and mod_ssl are available
#   PHP_MODULE         defined when PHP runs inside Apache (PHP_MODULE_PATH)
#   PHP_FPM            defined when PHP runs as PHP-FPM (PHP_FPM_SOCKET)

ServerRoot "{apache}"
DefaultRuntimeDir "{run}"
PidFile "{run}/httpd.pid"

# Apache for Windows has one MPM, built in, and runs the thread-safe
# mod_php. Elsewhere mod_php isn't thread-safe, so it needs prefork.
<IfModule !mpm_winnt_module>
    <IfDefine PHP_MODULE>
        LoadModule mpm_prefork_module "${{SLOPP_MODULES}}/mod_mpm_prefork.so"
    </IfDefine>
    <IfDefine !PHP_MODULE>
        LoadModule mpm_event_module "${{SLOPP_MODULES}}/mod_mpm_event.so"
    </IfDefine>
    <IfModule !unixd_module>
        LoadModule unixd_module "${{SLOPP_MODULES}}/mod_unixd.so"
    </IfModule>
</IfModule>

# Some distributions build these into httpd itself.
<IfModule !log_config_module>
    LoadModule log_config_module "${{SLOPP_MODULES}}/mod_log_config.so"
</IfModule>
<IfModule !version_module>
    LoadModule version_module "${{SLOPP_MODULES}}/mod_version.so"
</IfModule>

LoadModule authn_core_module "${{SLOPP_MODULES}}/mod_authn_core.so"
LoadModule authn_file_module "${{SLOPP_MODULES}}/mod_authn_file.so"
LoadModule authz_core_module "${{SLOPP_MODULES}}/mod_authz_core.so"
LoadModule authz_host_module "${{SLOPP_MODULES}}/mod_authz_host.so"
LoadModule authz_user_module "${{SLOPP_MODULES}}/mod_authz_user.so"
LoadModule auth_basic_module "${{SLOPP_MODULES}}/mod_auth_basic.so"
LoadModule access_compat_module "${{SLOPP_MODULES}}/mod_access_compat.so"
LoadModule alias_module "${{SLOPP_MODULES}}/mod_alias.so"
LoadModule autoindex_module "${{SLOPP_MODULES}}/mod_autoindex.so"
LoadModule dir_module "${{SLOPP_MODULES}}/mod_dir.so"
LoadModule env_module "${{SLOPP_MODULES}}/mod_env.so"
LoadModule filter_module "${{SLOPP_MODULES}}/mod_filter.so"
LoadModule deflate_module "${{SLOPP_MODULES}}/mod_deflate.so"
LoadModule headers_module "${{SLOPP_MODULES}}/mod_headers.so"
LoadModule mime_module "${{SLOPP_MODULES}}/mod_mime.so"
LoadModule rewrite_module "${{SLOPP_MODULES}}/mod_rewrite.so"
LoadModule setenvif_module "${{SLOPP_MODULES}}/mod_setenvif.so"

<IfDefine SSL>
    LoadModule socache_shmcb_module "${{SLOPP_MODULES}}/mod_socache_shmcb.so"
    LoadModule ssl_module "${{SLOPP_MODULES}}/mod_ssl.so"
</IfDefine>
<IfDefine PHP_FPM>
    LoadModule proxy_module "${{SLOPP_MODULES}}/mod_proxy.so"
    LoadModule proxy_fcgi_module "${{SLOPP_MODULES}}/mod_proxy_fcgi.so"
</IfDefine>
<IfDefine PHP_MODULE>
    LoadModule php_module "${{PHP_MODULE_PATH}}"
</IfDefine>

Listen ${{HTTP_PORT}}

ServerName localhost:${{HTTP_PORT}}
ServerAdmin postmaster@localhost

<Directory />
    AllowOverride None
    Require all denied
</Directory>

DocumentRoot "{htdocs}"
<Directory "{htdocs}">
    Options Indexes FollowSymLinks
    AllowOverride All
    Require all granted
</Directory>

<IfModule dir_module>
    DirectoryIndex index.php index.html index.htm
</IfModule>

<Files ".ht*">
    Require all denied
</Files>

ErrorLog "logs/error.log"
LogLevel warn

<IfModule log_config_module>
    LogFormat "%h %l %u %t \"%r\" %>s %b \"%{{Referer}}i\" \"%{{User-Agent}}i\"" combined
    LogFormat "%h %l %u %t \"%r\" %>s %b" common
    CustomLog "logs/access.log" combined
</IfModule>

<IfModule mime_module>
    TypesConfig "conf/mime.types"
    AddType application/x-compress .Z
    AddType application/x-gzip .gz .tgz
</IfModule>

<IfModule mpm_prefork_module>
    StartServers 2
    MinSpareServers 2
    MaxSpareServers 5
    MaxRequestWorkers 50
</IfModule>
<IfModule mpm_event_module>
    StartServers 1
    MinSpareThreads 5
    MaxSpareThreads 25
    ThreadsPerChild 25
    MaxRequestWorkers 100
</IfModule>
<IfModule mpm_winnt_module>
    ThreadsPerChild 100
</IfModule>

EnableSendfile on

# PHP, phpMyAdmin and Adminer
Include "conf/extra/httpd-slopp.conf"

<IfDefine SSL>
    Include "conf/extra/httpd-ssl.conf"
</IfDefine>

# The stack root: {root}
"#
    )
}

pub fn httpd_slopp_conf(layout: &Layout) -> String {
    let php = conf_path(&layout.php());
    let pma = conf_path(&layout.phpmyadmin());
    format!(
        r#"# PHP, phpMyAdmin and Adminer for SLOPP. Yours to edit.

<IfDefine PHP_MODULE>
    PHPIniDir "{php}"
    <FilesMatch "\.php$">
        SetHandler application/x-httpd-php
    </FilesMatch>
    <FilesMatch "\.phps$">
        SetHandler application/x-httpd-php-source
    </FilesMatch>
</IfDefine>

<IfDefine PHP_FPM>
    <FilesMatch "\.php$">
        SetHandler "proxy:unix:${{PHP_FPM_SOCKET}}|fcgi://localhost"
    </FilesMatch>
    # Pass the Authorization header through to PHP.
    SetEnvIfNoCase ^Authorization$ "(.+)" HTTP_AUTHORIZATION=$1
</IfDefine>

Alias /phpmyadmin "{pma}/"
<Directory "{pma}">
    AllowOverride AuthConfig
    # phpMyAdmin logs in as root without a password, like XAMPP's: keep it
    # to this computer.
    Require local
</Directory>

{adminer}"#,
        adminer = adminer_alias(layout),
    )
}

/// Serves `adminer/` at `/adminer`; also appended to older stacks' config
/// when Adminer is installed.
pub fn adminer_alias(layout: &Layout) -> String {
    let dir = conf_path(&layout.adminer());
    format!(
        r#"Alias /adminer "{dir}/"
<Directory "{dir}">
    AllowOverride AuthConfig
    # Adminer logs in as root without a password too.
    Require local
</Directory>
"#
    )
}

pub fn httpd_ssl_conf(layout: &Layout) -> String {
    let htdocs = conf_path(&layout.htdocs());
    let cert = conf_path(&layout.ssl_cert());
    let key = conf_path(&layout.ssl_key());
    format!(
        r#"# HTTPS for SLOPP, with a self-signed certificate for localhost.
# Yours to edit.

Listen ${{HTTPS_PORT}}

SSLProtocol all -SSLv3 -TLSv1 -TLSv1.1
SSLHonorCipherOrder on
SSLSessionCache "shmcb:logs/ssl_scache(512000)"
SSLSessionCacheTimeout 300

<VirtualHost _default_:${{HTTPS_PORT}}>
    DocumentRoot "{htdocs}"
    ServerName localhost:${{HTTPS_PORT}}
    ErrorLog "logs/error.log"
    TransferLog "logs/access.log"

    SSLEngine on
    SSLCertificateFile "{cert}"
    SSLCertificateKeyFile "{key}"
</VirtualHost>
"#
    )
}

pub fn php_ini(layout: &Layout) -> String {
    php_ini_for(layout, cfg!(windows))
}

/// What phpMyAdmin, Adminer and most frameworks need, like XAMPP's PHP.
/// PHP for Windows ships them without loading them, so php.ini lists them
/// there. Elsewhere packages enable theirs in PHP's own ini directory, except
/// where the distribution's php.ini does it (Arch), which SLOPP's replaces:
/// [`toolchain::unloaded_extensions`](crate::toolchain::unloaded_extensions)
/// finds those.
pub(crate) const EXTENSIONS: &[&str] = &[
    "curl",
    "fileinfo",
    "gd",
    "intl",
    "mbstring",
    "mysqli",
    "openssl",
    "pdo_mysql",
    "sodium",
    "zip",
];

fn php_ini_for(layout: &Layout, windows: bool) -> String {
    let mail = php_mail_ini_for(layout, windows);
    let error_log = conf_path(&layout.php_error_log());
    let tmp = conf_path(&layout.tmp());
    let socket = conf_path(&layout.mysql_socket());
    let (extensions, mysql) = if windows {
        let lines: String = EXTENSIONS
            .iter()
            .map(|e| format!("extension={e}\n"))
            .collect();
        (
            format!(
                "; Extensions; add more `extension=` lines here. When Apache starts,\n\
                 ; SLOPP writes run/php/slopp.ini, which PHP reads after this file:\n\
                 ; extension_dir (your PHP's ext folder), mysqli's default port, and\n\
                 ; SMTP and smtp_port, so mail() goes to Mailpit.\n{lines}"
            ),
            String::new(),
        )
    } else {
        (
            "; Extensions come from your PHP installation's own ini directory;\n\
             ; add `extension=` lines here for more.\n"
                .to_string(),
            format!(
                "\n[MySQLi]\nmysqli.default_socket = \"{socket}\"\n\n\
                 [Pdo_mysql]\npdo_mysql.default_socket = \"{socket}\"\n"
            ),
        )
    };
    format!(
        r#"; PHP settings for SLOPP: development-friendly, like XAMPP's.
; Yours to edit.

[PHP]
engine = On
short_open_tag = Off
precision = 14
output_buffering = 4096
max_execution_time = 120
max_input_time = 60
memory_limit = 512M
error_reporting = E_ALL
display_errors = On
display_startup_errors = On
log_errors = On
error_log = "{error_log}"
post_max_size = 40M
upload_max_filesize = 40M
max_file_uploads = 20
file_uploads = On
upload_tmp_dir = "{tmp}"
sys_temp_dir = "{tmp}"
default_charset = "UTF-8"
allow_url_fopen = On

{extensions}
[Date]
; date.timezone = Europe/Berlin

[Session]
session.save_handler = files
session.save_path = "{tmp}"
session.use_strict_mode = 1
session.gc_maxlifetime = 1440
{mysql}{mail}
[opcache]
opcache.enable = 1
opcache.validate_timestamps = 1
opcache.revalidate_freq = 0
"#
    )
}

/// php.ini's mail section, sending `mail()` to Mailpit. Also added to a
/// php.ini written before SLOPP had it ([`crate::setup::ensure_php_mail`]).
pub fn php_mail_ini(layout: &Layout) -> String {
    php_mail_ini_for(layout, cfg!(windows))
}

/// Linux and macOS: through [`Layout::sendmail`]. Windows: over SMTP, whose
/// address is in [`php_scan_ini`]; PHP for Windows refuses a message with
/// no `From:` header unless `sendmail_from` names a sender.
fn php_mail_ini_for(layout: &Layout, windows: bool) -> String {
    if windows {
        return "\n[mail function]\n\
                ; mail() goes to Mailpit: run/php/slopp.ini, which SLOPP writes on every\n\
                ; start, sets SMTP and smtp_port. The sender of mail without a From: header.\n\
                sendmail_from = \"php@localhost\"\n"
            .to_string();
    }
    let sendmail = sh_quote(&conf_path(&layout.sendmail()));
    format!(
        "\n[mail function]\n\
         ; mail() goes to Mailpit: run/sendmail, which SLOPP writes on every start,\n\
         ; hands each message to Mailpit's SMTP port. Change this for real mail.\n\
         sendmail_path = \"{sendmail} -t -i\"\n"
    )
}

/// The php.ini setting [`php_mail_ini_for`] is about: while php.ini doesn't
/// mention it, SLOPP adds the mail section.
pub fn php_mail_setting(windows: bool) -> &'static str {
    if windows {
        "sendmail_from"
    } else {
        "sendmail_path"
    }
}

/// `text` as one word for `sh`, quoted only when it needs to be.
fn sh_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+@:,".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// Linux and macOS: [`Layout::sendmail`], which PHP's `sendmail_path` runs:
/// Mailpit's own `sendmail` aimed at its SMTP port, or, while there's no
/// Mailpit, a refusal PHP logs. Rewritten on every start.
pub fn sendmail_script(mailpit: Option<&Path>, smtp_port: u16) -> String {
    let head = "#!/bin/sh\n# Written by SLOPP on every start: hands PHP's mail to Mailpit.\n";
    match mailpit {
        Some(mailpit) => format!(
            "{head}exec {} sendmail -S 127.0.0.1:{smtp_port} \"$@\"\n",
            sh_quote(&mailpit.to_string_lossy())
        ),
        None => format!(
            "{head}echo \"SLOPP: Mailpit isn't installed, so this mail went nowhere\" >&2\n\
             cat >/dev/null\n\
             exit 1\n"
        ),
    }
}

/// Windows: `slopp.ini` in [`Layout::php_scan_dir`], so the extensions in
/// php.ini load from the PHP that runs, mysqli finds MariaDB on the port in
/// Settings (what `mysqli.default_socket` does elsewhere), and `mail()`
/// talks to Mailpit (what `sendmail_path` does elsewhere). Rewritten on
/// every start.
pub fn php_scan_ini(extension_dir: &Path, mysql_port: u16, smtp_port: u16) -> String {
    format!(
        "; Written by SLOPP on every start; edit php.ini instead.\n\
         extension_dir = \"{}\"\n\
         mysqli.default_host = \"127.0.0.1\"\n\
         mysqli.default_port = {mysql_port}\n\
         SMTP = \"127.0.0.1\"\n\
         smtp_port = {smtp_port}\n",
        conf_path(extension_dir)
    )
}

/// Linux and macOS: `slopp.ini` in [`Layout::php_scan_dir`], loading the
/// extensions this PHP has but doesn't load by itself. Rewritten on every
/// start.
pub fn php_extensions_ini(extensions: &[&str]) -> String {
    let lines: String = extensions
        .iter()
        .map(|e| format!("extension={e}\n"))
        .collect();
    format!(
        "; Written by SLOPP on every start; edit php.ini instead.\n\
         ; Extensions this PHP has that its own configuration doesn't load.\n{lines}"
    )
}

/// PHP-FPM's pool. Internal plumbing, so it's rewritten on every start.
pub fn php_fpm_conf(layout: &Layout) -> String {
    let run = conf_path(&layout.run());
    let socket = conf_path(&layout.php_fpm_socket());
    let log = conf_path(&layout.php_fpm_log());
    format!(
        r#"; Written by SLOPP on every start; edit php.ini instead.
[global]
pid = "{run}/php-fpm.pid"
error_log = "{log}"
daemonize = no

[www]
listen = "{socket}"
listen.mode = 0600
pm = dynamic
pm.max_children = 10
pm.start_servers = 2
pm.min_spare_servers = 1
pm.max_spare_servers = 4
catch_workers_output = yes
decorate_workers_output = no
clear_env = no
"#
    )
}

pub fn my_cnf(layout: &Layout) -> String {
    my_cnf_for(layout, cfg!(windows))
}

/// How clients reach the server: its socket, or on Windows (where MariaDB's
/// counterpart is a named pipe, off by default) TCP on localhost.
fn my_cnf_for(layout: &Layout, windows: bool) -> String {
    let data = conf_path(&layout.mysql_data());
    let socket = conf_path(&layout.mysql_socket());
    let error_log = conf_path(&layout.mysql_error_log());
    let pid_file = conf_path(&layout.mysql_pid_file());
    let tmp = conf_path(&layout.tmp());
    // MariaDB before 10.11 reads this file in the Windows code page, so on
    // Windows its paths, which may hold any letter (`C:\Users\Zalán`), go on
    // the command line instead.
    let (client, server) = if windows {
        (
            "host = 127.0.0.1".to_string(),
            "# SLOPP passes datadir, tmpdir, log_error and pid-file on the command\n\
             # line: older MariaDB reads this file in the Windows code page.\n"
                .to_string(),
        )
    } else {
        (
            format!("socket = \"{socket}\""),
            format!(
                "datadir = \"{data}\"\n\
                 socket = \"{socket}\"\n\
                 pid-file = \"{pid_file}\"\n\
                 tmpdir = \"{tmp}\"\n\
                 log_error = \"{error_log}\"\n"
            ),
        )
    };
    format!(
        r#"# MariaDB settings for SLOPP. Yours to edit.
# The port comes from the control panel's settings.

[client]
{client}
default-character-set = utf8mb4

[mysqld]
{server}bind-address = 127.0.0.1
skip-name-resolve
character-set-server = utf8mb4
collation-server = utf8mb4_general_ci
max_allowed_packet = 64M
innodb_buffer_pool_size = 128M
innodb_log_file_size = 48M
key_buffer_size = 16M

[mysqldump]
max_allowed_packet = 64M
"#
    )
}

/// phpMyAdmin's config: log in as root with no password over the socket
/// (TCP on Windows), as XAMPP does.
pub fn phpmyadmin_config(layout: &Layout, secret: &str, port: u16) -> String {
    phpmyadmin_config_for(layout, secret, port, cfg!(windows))
}

fn phpmyadmin_config_for(layout: &Layout, secret: &str, port: u16, windows: bool) -> String {
    let server = if windows {
        "$cfg['Servers'][$i]['host'] = '127.0.0.1';".to_string()
    } else {
        let socket = conf_path(&layout.mysql_socket());
        format!(
            "$cfg['Servers'][$i]['host'] = 'localhost';\n\
             $cfg['Servers'][$i]['socket'] = '{socket}';"
        )
    };
    let tmp = conf_path(&layout.tmp().join("phpmyadmin"));
    let theme = crate::phpmyadmin::THEME;
    format!(
        r#"<?php
/* phpMyAdmin configuration for SLOPP. Yours to edit. */

$cfg['blowfish_secret'] = '{secret}';

$i = 0;
$i++;
$cfg['Servers'][$i]['auth_type'] = 'config';
$cfg['Servers'][$i]['user'] = 'root';
$cfg['Servers'][$i]['password'] = '';
$cfg['Servers'][$i]['AllowNoPassword'] = true;
{server}
$cfg['Servers'][$i]['port'] = '{port}';

$cfg['ThemeDefault'] = '{theme}';

$cfg['TempDir'] = '{tmp}';
$cfg['UploadDir'] = '';
$cfg['SaveDir'] = '';
"#
    )
}

/// Adminer's `index.php`: logs in as root over the socket (TCP on the MySQL
/// port on Windows) without asking, then hands over to `adminer.php`.
/// Adminer won't log in without a password unless told to, hence `login()`.
pub fn adminer_index(layout: &Layout, port: u16) -> String {
    adminer_index_for(layout, port, cfg!(windows))
}

fn adminer_index_for(layout: &Layout, port: u16, windows: bool) -> String {
    let server = if windows {
        format!("127.0.0.1:{port}")
    } else {
        format!(":{}", conf_path(&layout.mysql_socket()))
    };
    format!(
        r#"<?php
/* Adminer for SLOPP. Yours to edit: "Get Adminer" replaces adminer.php,
 * never this file. It logs in as root without a password, like phpMyAdmin
 * in this stack, so logging out just logs in again. */

function adminer_object()
{{
    class SloppAdminer extends Adminer\Adminer
    {{
        function credentials()
        {{
            return array('{server}', 'root', '');
        }}

        function login($login, $password)
        {{
            return true;
        }}

        // The automatic login below is a form post without Adminer's token.
        function verifyLoginToken()
        {{
            return false;
        }}
    }}
    return new SloppAdminer();
}}

// Log in instead of showing the form: on the way in, and on a bookmarked
// page the browser has no session for. The permanent login brings back
// sessions PHP has expired.
$known = !empty($_COOKIE['adminer_sid']) || !empty($_COOKIE['adminer_permanent']);
if (!$_POST && (!isset($_GET['username']) || !$known)) {{
    $_POST['auth'] = array(
        'driver' => 'server',
        'server' => '',
        'username' => 'root',
        'password' => '',
        'db' => isset($_GET['db']) ? $_GET['db'] : '',
        'permanent' => 1,
    );
}}

include __DIR__ . '/adminer.php';
"#
    )
}

/// Adminer's design: the panel's palette, light or dark with the system.
pub const ADMINER_CSS: &str = include_str!("adminer.css");

/// `htdocs/index.php` for a new stack.
pub fn dashboard() -> &'static str {
    include_str!("dashboard.php")
}

/// Used when no `mime.types` can be found on the system.
pub const MIME_TYPES: &str = "\
text/html                       html htm
text/css                        css
text/plain                      txt
text/xml                        xml
text/javascript                 js mjs
application/json                json
application/pdf                 pdf
application/zip                 zip
application/wasm                wasm
image/png                       png
image/jpeg                      jpeg jpg
image/gif                       gif
image/svg+xml                   svg svgz
image/webp                      webp
image/avif                      avif
image/x-icon                    ico
font/woff                       woff
font/woff2                      woff2
font/ttf                        ttf
audio/mpeg                      mp3
video/mp4                       mp4
video/webm                      webm
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reaches_mysql_over_tcp() {
        let layout = Layout::new("/s");
        let ini = php_ini_for(&layout, true);
        assert!(ini.contains("\nextension=mysqli\n"));
        assert!(!ini.contains("default_socket"));
        assert!(
            php_ini_for(&layout, false).contains("mysqli.default_socket = \"/s/tmp/mysql.sock\"")
        );

        let cnf = my_cnf_for(&layout, true);
        assert!(cnf.contains("[client]\nhost = 127.0.0.1\n"));
        assert!(!cnf.contains("socket"));
        assert!(!cnf.contains("/s/"), "no paths on Windows:\n{cnf}");
        assert!(
            my_cnf_for(&layout, false).contains("[mysqld]\ndatadir = \"/s/mysql/data\"\nsocket = ")
        );

        let pma = phpmyadmin_config_for(&layout, "x", 3307, true);
        assert!(pma.contains("['host'] = '127.0.0.1';\n$cfg['Servers'][$i]['port'] = '3307';"));
        assert!(!pma.contains("socket"));

        assert!(
            adminer_index_for(&layout, 3307, true).contains("array('127.0.0.1:3307', 'root', '')")
        );
        assert!(
            adminer_index_for(&layout, 3307, false)
                .contains("array(':/s/tmp/mysql.sock', 'root', '')")
        );
    }

    #[test]
    fn scan_ini_names_the_extension_dir() {
        assert_eq!(
            php_scan_ini(Path::new(r"C:\php\ext"), 3307, 1025),
            "; Written by SLOPP on every start; edit php.ini instead.\n\
             extension_dir = \"C:/php/ext\"\n\
             mysqli.default_host = \"127.0.0.1\"\n\
             mysqli.default_port = 3307\n\
             SMTP = \"127.0.0.1\"\n\
             smtp_port = 1025\n"
        );
    }

    #[test]
    fn php_mail_goes_to_mailpit() {
        let ini = php_ini_for(&Layout::new("/s"), false);
        assert!(ini.contains("\n[mail function]\n"));
        assert!(ini.contains("\nsendmail_path = \"/s/run/sendmail -t -i\"\n"));
        let windows = php_ini_for(&Layout::new("/s"), true);
        assert!(!windows.contains("sendmail_path"), "{windows}");
        assert!(windows.contains("\n[mail function]\n"), "{windows}");
        assert!(windows.contains("\nsendmail_from = \"php@localhost\"\n"));
        assert!(
            php_mail_ini_for(&Layout::new("/home/me/my stack"), false)
                .contains("sendmail_path = \"'/home/me/my stack/run/sendmail' -t -i\"")
        );

        assert_eq!(
            sendmail_script(Some(Path::new("/usr/bin/mailpit")), 2525),
            "#!/bin/sh\n# Written by SLOPP on every start: hands PHP's mail to Mailpit.\n\
             exec /usr/bin/mailpit sendmail -S 127.0.0.1:2525 \"$@\"\n"
        );
        assert!(
            sendmail_script(Some(Path::new("/it's here/mailpit")), 1025)
                .contains("exec '/it'\\''s here/mailpit' sendmail")
        );
        let missing = sendmail_script(None, 1025);
        assert!(missing.contains("exit 1\n") && missing.contains("cat >/dev/null"));
    }
}
