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
        r#"# Apache httpd configuration for SLOMPP.
#
# This file is yours to edit; SLOMPP never rewrites it. The control panel
# passes these when it starts Apache:
#
#   SLOMPP_MODULES     the directory holding Apache's mod_*.so
#   HTTP_PORT          the HTTP port from the panel's settings
#   HTTPS_PORT         the HTTPS port from the panel's settings
#   SSL                defined when a certificate and mod_ssl are available
#   PHP_MODULE         defined when PHP runs inside Apache (PHP_MODULE_PATH)
#   PHP_FPM            defined when PHP runs as PHP-FPM (PHP_FPM_SOCKET)

ServerRoot "{apache}"
DefaultRuntimeDir "{run}"
PidFile "{run}/httpd.pid"

# mod_php isn't thread-safe, so it needs the prefork MPM.
<IfDefine PHP_MODULE>
    LoadModule mpm_prefork_module "${{SLOMPP_MODULES}}/mod_mpm_prefork.so"
</IfDefine>
<IfDefine !PHP_MODULE>
    LoadModule mpm_event_module "${{SLOMPP_MODULES}}/mod_mpm_event.so"
</IfDefine>

# Some distributions build these into httpd itself.
<IfModule !unixd_module>
    LoadModule unixd_module "${{SLOMPP_MODULES}}/mod_unixd.so"
</IfModule>
<IfModule !log_config_module>
    LoadModule log_config_module "${{SLOMPP_MODULES}}/mod_log_config.so"
</IfModule>
<IfModule !version_module>
    LoadModule version_module "${{SLOMPP_MODULES}}/mod_version.so"
</IfModule>

LoadModule authn_core_module "${{SLOMPP_MODULES}}/mod_authn_core.so"
LoadModule authn_file_module "${{SLOMPP_MODULES}}/mod_authn_file.so"
LoadModule authz_core_module "${{SLOMPP_MODULES}}/mod_authz_core.so"
LoadModule authz_host_module "${{SLOMPP_MODULES}}/mod_authz_host.so"
LoadModule authz_user_module "${{SLOMPP_MODULES}}/mod_authz_user.so"
LoadModule auth_basic_module "${{SLOMPP_MODULES}}/mod_auth_basic.so"
LoadModule access_compat_module "${{SLOMPP_MODULES}}/mod_access_compat.so"
LoadModule alias_module "${{SLOMPP_MODULES}}/mod_alias.so"
LoadModule autoindex_module "${{SLOMPP_MODULES}}/mod_autoindex.so"
LoadModule dir_module "${{SLOMPP_MODULES}}/mod_dir.so"
LoadModule env_module "${{SLOMPP_MODULES}}/mod_env.so"
LoadModule filter_module "${{SLOMPP_MODULES}}/mod_filter.so"
LoadModule deflate_module "${{SLOMPP_MODULES}}/mod_deflate.so"
LoadModule headers_module "${{SLOMPP_MODULES}}/mod_headers.so"
LoadModule mime_module "${{SLOMPP_MODULES}}/mod_mime.so"
LoadModule rewrite_module "${{SLOMPP_MODULES}}/mod_rewrite.so"
LoadModule setenvif_module "${{SLOMPP_MODULES}}/mod_setenvif.so"

<IfDefine SSL>
    LoadModule socache_shmcb_module "${{SLOMPP_MODULES}}/mod_socache_shmcb.so"
    LoadModule ssl_module "${{SLOMPP_MODULES}}/mod_ssl.so"
</IfDefine>
<IfDefine PHP_FPM>
    LoadModule proxy_module "${{SLOMPP_MODULES}}/mod_proxy.so"
    LoadModule proxy_fcgi_module "${{SLOMPP_MODULES}}/mod_proxy_fcgi.so"
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

EnableSendfile on

# PHP, phpMyAdmin and Adminer
Include "conf/extra/httpd-slompp.conf"

<IfDefine SSL>
    Include "conf/extra/httpd-ssl.conf"
</IfDefine>

# The stack root: {root}
"#
    )
}

pub fn httpd_slompp_conf(layout: &Layout) -> String {
    let php = conf_path(&layout.php());
    let pma = conf_path(&layout.phpmyadmin());
    format!(
        r#"# PHP, phpMyAdmin and Adminer for SLOMPP. Yours to edit.

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
        r#"# HTTPS for SLOMPP, with a self-signed certificate for localhost.
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
    let error_log = conf_path(&layout.php_error_log());
    let tmp = conf_path(&layout.tmp());
    let socket = conf_path(&layout.mysql_socket());
    format!(
        r#"; PHP settings for SLOMPP: development-friendly, like XAMPP's.
; Yours to edit. Extensions come from your PHP installation's own ini
; directory; add `extension=` lines here for more.

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

[Date]
; date.timezone = Europe/Berlin

[Session]
session.save_handler = files
session.save_path = "{tmp}"
session.use_strict_mode = 1
session.gc_maxlifetime = 1440

[MySQLi]
mysqli.default_socket = "{socket}"

[Pdo_mysql]
pdo_mysql.default_socket = "{socket}"

[opcache]
opcache.enable = 1
opcache.validate_timestamps = 1
opcache.revalidate_freq = 0
"#
    )
}

/// PHP-FPM's pool. Internal plumbing, so it's rewritten on every start.
pub fn php_fpm_conf(layout: &Layout) -> String {
    let run = conf_path(&layout.run());
    let socket = conf_path(&layout.php_fpm_socket());
    let log = conf_path(&layout.php_fpm_log());
    format!(
        r#"; Written by SLOMPP on every start; edit php.ini instead.
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
    let data = conf_path(&layout.mysql_data());
    let socket = conf_path(&layout.mysql_socket());
    let error_log = conf_path(&layout.mysql_error_log());
    let run = conf_path(&layout.run());
    let tmp = conf_path(&layout.tmp());
    format!(
        r#"# MariaDB settings for SLOMPP. Yours to edit.
# The port comes from the control panel's settings.

[client]
socket = "{socket}"
default-character-set = utf8mb4

[mysqld]
datadir = "{data}"
socket = "{socket}"
pid-file = "{run}/mysqld.pid"
tmpdir = "{tmp}"
log_error = "{error_log}"
bind-address = 127.0.0.1
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

/// phpMyAdmin's config: log in as root with no password over the socket,
/// as XAMPP does.
pub fn phpmyadmin_config(layout: &Layout, secret: &str, port: u16) -> String {
    let socket = conf_path(&layout.mysql_socket());
    let tmp = conf_path(&layout.tmp().join("phpmyadmin"));
    let theme = crate::phpmyadmin::THEME;
    format!(
        r#"<?php
/* phpMyAdmin configuration for SLOMPP. Yours to edit. */

$cfg['blowfish_secret'] = '{secret}';

$i = 0;
$i++;
$cfg['Servers'][$i]['auth_type'] = 'config';
$cfg['Servers'][$i]['user'] = 'root';
$cfg['Servers'][$i]['password'] = '';
$cfg['Servers'][$i]['AllowNoPassword'] = true;
$cfg['Servers'][$i]['host'] = 'localhost';
$cfg['Servers'][$i]['socket'] = '{socket}';
$cfg['Servers'][$i]['port'] = '{port}';

$cfg['ThemeDefault'] = '{theme}';

$cfg['TempDir'] = '{tmp}';
$cfg['UploadDir'] = '';
$cfg['SaveDir'] = '';
"#
    )
}

/// Adminer's `index.php`: logs in as root over the socket without asking,
/// then hands over to `adminer.php`. Adminer won't log in without a password
/// unless told to, hence `login()`.
pub fn adminer_index(layout: &Layout) -> String {
    let socket = conf_path(&layout.mysql_socket());
    format!(
        r#"<?php
/* Adminer for SLOMPP. Yours to edit: "Get Adminer" replaces adminer.php,
 * never this file. It logs in as root without a password, like phpMyAdmin
 * in this stack, so logging out just logs in again. */

function adminer_object()
{{
    class SlomppAdminer extends Adminer\Adminer
    {{
        function credentials()
        {{
            return array(':{socket}', 'root', '');
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
    return new SlomppAdminer();
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
