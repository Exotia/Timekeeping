use clap::Parser;
use tk::cli::{Cli, ExitCode, commands, open_ctx};

fn main() {
    let cli = Cli::parse();
    let ctx = match open_ctx(cli.home.as_deref()) {
        Ok(c) => c,
        Err((ExitCode(code), err)) => {
            eprintln!("error: {err:#}");
            std::process::exit(code);
        }
    };
    let result = match cli.cmd {
        None => tk::tui::run(ctx),
        Some(cmd) => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            commands::run(cmd, &ctx, &mut lock)
        }
    };
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
