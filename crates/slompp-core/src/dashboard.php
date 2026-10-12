<?php
// SLOMPP's welcome page. Replace it with your own site whenever you like.

if (isset($_GET['phpinfo'])) {
    phpinfo();
    exit;
}

$db = null;
$dbError = null;
if (function_exists('mysqli_connect')) {
    mysqli_report(MYSQLI_REPORT_OFF);
    $link = @mysqli_connect('localhost', 'root', '');
    if ($link) {
        $db = mysqli_get_server_info($link);
        mysqli_close($link);
    } else {
        $dbError = mysqli_connect_error();
    }
} else {
    $dbError = 'The mysqli extension is not loaded.';
}
$pma = is_dir(__DIR__ . '/../phpMyAdmin');
$h = fn($s) => htmlspecialchars((string) $s, ENT_QUOTES);
?>
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SLOMPP</title>
<style>
  :root { --bg:#f3f4f7; --panel:#fff; --text:#1b2030; --muted:#5d6577; --line:#dde1ea; --ok:#2f9e63; --off:#a0a7b5; --accent:#e8692a; }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#151a24; --panel:#1d2330; --text:#e8ebf2; --muted:#98a1b3; --line:#2c3446; --ok:#5bc489; --off:#5d6577; --accent:#f08a4b; }
  }
  * { box-sizing: border-box; }
  body { margin:0; background:var(--bg); color:var(--text); font:16px/1.55 "IBM Plex Sans", system-ui, sans-serif; }
  main { max-width: 680px; margin: 0 auto; padding: 64px 20px; }
  h1 { font-size: 40px; line-height: 1.1; margin: 0 0 8px; letter-spacing: -0.02em; }
  p.lead { color: var(--muted); margin: 0 0 40px; }
  .rack { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  .unit { display: flex; align-items: center; gap: 14px; padding: 16px 20px; border-top: 1px solid var(--line); }
  .unit:first-child { border-top: 0; }
  .lamp { width: 10px; height: 10px; border-radius: 50%; background: var(--off); flex: none; }
  .lamp.on { background: var(--ok); box-shadow: 0 0 0 4px color-mix(in srgb, var(--ok) 20%, transparent); }
  .name { font-weight: 600; width: 96px; flex: none; }
  .detail { color: var(--muted); font-family: "IBM Plex Mono", ui-monospace, monospace; font-size: 14px; overflow-wrap: anywhere; }
  nav { display: flex; flex-wrap: wrap; gap: 12px; margin-top: 28px; }
  nav a { color: var(--text); text-decoration: none; padding: 9px 16px; border: 1px solid var(--line); border-radius: 8px; background: var(--panel); }
  nav a:hover { border-color: var(--accent); }
  nav a:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  footer { margin-top: 48px; color: var(--muted); font-size: 14px; }
  code { font-family: "IBM Plex Mono", ui-monospace, monospace; font-size: 0.9em; }
</style>
</head>
<body>
<main>
  <h1>Your stack is up.</h1>
  <p class="lead">Put your projects in <code><?= $h(__DIR__) ?></code> and they appear here.</p>

  <div class="rack">
    <div class="unit">
      <span class="lamp on"></span><span class="name">Apache</span>
      <span class="detail"><?= $h($_SERVER['SERVER_SOFTWARE'] ?? 'Apache') ?></span>
    </div>
    <div class="unit">
      <span class="lamp on"></span><span class="name">PHP</span>
      <span class="detail"><?= $h(PHP_VERSION) ?> · <?= $h(php_sapi_name()) ?></span>
    </div>
    <div class="unit">
      <span class="lamp<?= $db ? ' on' : '' ?>"></span><span class="name">MySQL</span>
      <span class="detail"><?= $db ? $h($db) : $h('Not reachable: ' . $dbError) ?></span>
    </div>
  </div>

  <nav>
    <?php if ($pma): ?><a href="/phpmyadmin/">Open phpMyAdmin</a><?php endif; ?>
    <a href="?phpinfo=1">Show phpinfo()</a>
  </nav>

  <footer>Served by SLOMPP. Edit or delete <code>index.php</code> to make this page yours.</footer>
</main>
</body>
</html>
