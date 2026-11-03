//! Windows 向けビルドのとき、exe にアイコンとバージョン情報を埋め込む。
//! アイコンはリソース ID 1 で入る（GPUI はウィンドウのアイコンに ID 1 を使う）。

fn main() {
    println!("cargo:rerun-if-changed=../../assets/wimux.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/wimux.ico")
        .set("ProductName", "Sondev Terminal")
        .set("FileDescription", "Sondev Terminal")
        .set("LegalCopyright", "Sondev");
    if let Err(e) = res.compile() {
        // アイコンが入らないだけなのでビルドは止めない
        println!("cargo:warning=アイコンを埋め込めませんでした: {e}");
    }
}
