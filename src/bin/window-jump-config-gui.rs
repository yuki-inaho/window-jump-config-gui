fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("window-jump-config-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Window Jump configuration GUI");
        println!();
        println!("Usage: window-jump-config-gui");
        println!("       window-jump-config-gui --version");
        return Ok(());
    }
    if let Some(arg) = args.first() {
        anyhow::bail!("unknown argument for GUI binary: {arg}");
    }

    window_jump::run_gui()
}
