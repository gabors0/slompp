//! Embeds the icon and version information in slopp.exe. GPUI loads icon
//! resource 1 for its windows, so this is the taskbar's icon as well as
//! Explorer's and the installer shortcuts'.

fn main() {
    println!("cargo::rerun-if-changed=assets/windows/slopp.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("assets/windows/slopp.ico", "1")
        .set("FileDescription", "SLOPP")
        .set("ProductName", "SLOPP");
    if let Err(err) = res.compile() {
        panic!("couldn't embed slopp.exe's resources: {err}");
    }
}
